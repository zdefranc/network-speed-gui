//! Runs iperf3 as a subprocess and turns its output into [`RunEvent`]s.
//!
//! ```text
//!  iperf3.exe ──stdout──▶ reader thread ─┐
//!             ──stderr──▶ reader thread ─┴─ lines ─▶ worker thread ── RunEvent ─▶ GUI
//!                                                   (parser + log)    + notify()
//! ```
//!
//! **The worker** parses each line, keeps the log, and sends events over a
//! channel. After every event it calls `notify` (the GUI's `request_repaint`).
//! When both pipes close, it waits for iperf3 to exit and sends the final
//! [`RunEvent::Ended`] with the outcome and the full log.

use std::collections::VecDeque;
use std::io::{BufRead, BufReader, Read};
use std::os::windows::process::CommandExt;
use std::path::Path;
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::thread;
use std::time::Duration;
use windows_sys::Win32::System::Threading::CREATE_NO_WINDOW;

use super::parser::{OutputParser, ParsedLine};
use super::{IperfError, Outcome, RunEvent, Summary, TestConfig};

/// How many of the last output lines go into an "exited with code N" error.
const OUTPUT_TAIL_LINES: usize = 5;

/// Windows `STATUS_DLL_NOT_FOUND` as an exit code. The Windows iperf3 builds
/// need their Cygwin DLLs next to `iperf3.exe`. Without them, the process
/// starts and immediately exits with this code.
const STATUS_DLL_NOT_FOUND: i32 = 0xC000_0135_u32 as i32;

/// A running iperf3 test. Dropping it cancels the test.
#[derive(Debug)]
pub struct RunHandle {
    events: Receiver<RunEvent>,
    child: Arc<Mutex<Child>>,
    cancelled: Arc<AtomicBool>,
    // Only held for its Drop, which kills iperf3 if it's still alive.
    _job: Option<super::job::KillOnCloseJob>,
}

/// Starts `exe` as an iperf3 client with the given configuration.
///
/// Results arrive through [`RunHandle::try_events`]. `notify` is called from a
/// background thread after every event.
pub fn start_test(
    exe: &Path,
    config: &TestConfig,
    notify: impl Fn() + Send + 'static,
) -> Result<RunHandle, IperfError> {
    if !exe.is_file() {
        return Err(IperfError::NotFound(exe.to_path_buf()));
    }

    let mut command = Command::new(exe);
    command
        .args(config.to_args())
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    hide_console_window(&mut command);

    let mut child = command
        .spawn()
        .map_err(|e| IperfError::Spawn(e.to_string()))?;

    // Set the child up to be killed if app is terminated.
    let job = super::job::KillOnCloseJob::new()
        .and_then(|job| job.assign(&child).map(|()| job))
        .ok();

    let stdout = child.stdout.take().expect("stdout is piped");
    let stderr = child.stderr.take().expect("stderr is piped");
    // Two OS pipes, one reader thread each merged into one channel so the worker sees lines in arrival order.
    let (line_send, line_recv) = mpsc::channel();
    spawn_line_reader(stdout, line_send.clone());
    spawn_line_reader(stderr, line_send);

    let (event_send, event_recv) = mpsc::channel();
    let child = Arc::new(Mutex::new(child));
    let cancelled = Arc::new(AtomicBool::new(false));

    let worker = Worker {
        child: Arc::clone(&child),
        cancelled: Arc::clone(&cancelled),
        events: event_send,
        notify: Box::new(notify),
    };
    thread::spawn(move || worker.run(line_recv));

    Ok(RunHandle {
        events: event_recv,
        child,
        cancelled,
        _job: job,
    })
}

impl RunHandle {
    /// Returns the events that arrived since the last call, without blocking.
    pub fn try_events(&self) -> Vec<RunEvent> {
        self.events.try_iter().collect()
    }

    /// Kills iperf3.
    pub fn cancel(&self) {
        let mut child = lock(&self.child);
        // Only attempt to kill the process if it has not already exited.
        if matches!(child.try_wait(), Ok(None)) {
            // Use SeqCst aa a safe default.
            self.cancelled.store(true, Ordering::SeqCst);
            let _ = child.kill();
        }
    }
}

impl Drop for RunHandle {
    fn drop(&mut self) {
        self.cancel();
    }
}

/// Stops a console window flashing up when this GUI app starts iperf3.
pub(super) fn hide_console_window(command: &mut Command) {
    command.creation_flags(CREATE_NO_WINDOW);
}

/// Locks a mutex even if it's "poisoned".
///
/// `Mutex::lock` returns `Err` if a thread panicked while holding the lock.
/// `RunHandle::cancel` must still be able to kill iperf3 in this case, so the guard is
/// taken anyway instead of panicking too.
fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Reads `pipe` line by line and sends each line to `send`.
fn spawn_line_reader(pipe: impl Read + Send + 'static, send: Sender<String>) {
    thread::spawn(move || {
        let mut reader = BufReader::new(pipe);
        let mut buf = Vec::new();
        // Ends on EOF or a read error.
        while let Ok(n) = reader.read_until(b'\n', &mut buf)
            && n > 0
        {
            // Lossy: Bad bytes become U+FFFD instead of losing the line.
            let line = String::from_utf8_lossy(&buf)
                .trim_end_matches(['\r', '\n'])
                .to_owned();
            if send.send(line).is_err() {
                break;
            }
            buf.clear();
        }
    });
}

/// Runs on its own thread: parses every output line, sends an `Interval` event
/// per measurement, and sends one `Ended` event once iperf3 has exited.
struct Worker {
    child: Arc<Mutex<Child>>,
    cancelled: Arc<AtomicBool>,
    events: Sender<RunEvent>,
    notify: Box<dyn Fn() + Send>,
}

impl Worker {
    fn emit(&self, event: RunEvent) {
        // `send` only fails if the receiver (inside the RunHandle) was dropped,
        // i.e. nobody is listening any more, so there's nothing to do about it.
        let _ = self.events.send(event);
        (self.notify)();
    }

    fn run(self, lines: Receiver<String>) {
        let mut parser = OutputParser::default();
        let mut log = String::new();
        let mut tail = VecDeque::with_capacity(OUTPUT_TAIL_LINES);
        let mut sender = None;
        let mut receiver = None;
        let mut error_message = None;

        // Read untill the associated line readers have completed.
        for line in lines {
            log.push_str(&line);
            log.push('\n');
            if !line.trim().is_empty() {
                if tail.len() == OUTPUT_TAIL_LINES {
                    tail.pop_front();
                }
                tail.push_back(line.clone());
            }

            match parser.parse_line(&line) {
                ParsedLine::Interval(m) => self.emit(RunEvent::Interval(m)),
                ParsedLine::SenderSummary(m) => sender = Some(m),
                ParsedLine::ReceiverSummary(m) => receiver = Some(m),
                // Treat the first error as the important message.
                ParsedLine::Error(msg) => {
                    error_message.get_or_insert(msg);
                }
                ParsedLine::Ignored => {}
            }
        }

        let status = self.wait_for_exit();
        let outcome = if self.cancelled.load(Ordering::SeqCst) {
            Outcome::Cancelled
        } else {
            let summary: Option<Summary> = sender
                .zip(receiver)
                .map(|(sender, receiver)| Summary { sender, receiver });
            decide_outcome(status, summary, error_message, &tail)
        };
        self.emit(RunEvent::Ended { outcome, log });
    }

    /// Polls for the exit, holding the lock only briefly.
    fn wait_for_exit(&self) -> std::io::Result<ExitStatus> {
        loop {
            if let Some(status) = lock(&self.child).try_wait()? {
                return Ok(status);
            }
            thread::sleep(Duration::from_millis(20));
        }
    }
}

/// Decides the outcome of a completed.
fn decide_outcome(
    status: std::io::Result<ExitStatus>,
    summary: Option<Summary>,
    error_message: Option<String>,
    tail: &VecDeque<String>,
) -> Outcome {
    let status = match status {
        Ok(status) => status,
        Err(e) => return Outcome::Failed(IperfError::Io(e.to_string())),
    };
    if let Some(message) = error_message {
        return Outcome::Failed(IperfError::Reported { message });
    }
    if status.code() == Some(STATUS_DLL_NOT_FOUND) {
        return Outcome::Failed(IperfError::Spawn(
            "a required DLL is missing; keep the Cygwin DLLs (cygwin1.dll, ...) \
            in the same folder as iperf3.exe"
                .into(),
        ));
    }
    if !status.success() {
        return Outcome::Failed(IperfError::NonZeroExit {
            code: status.code(),
            output_tail: Vec::from(tail.clone()).join("\n"),
        });
    }
    match summary {
        Some(summary) => Outcome::Finished(summary),
        None => Outcome::Failed(IperfError::IncompleteOutput),
    }
}
