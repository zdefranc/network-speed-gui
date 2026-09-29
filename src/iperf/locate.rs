//! Finding a usable iperf3 executable and checking its version.
//!
//! At startup the GUI calls [`detect_in_background`], which searches `PATH`.
//! If that fails, the user can enter a path to an iperf3 installation,
//! which it then checks with [`probe_in_background`].
//!
//! A probe runs `iperf3 --version`. That proves the file really starts (its
//! Cygwin DLLs are present) and gives the version to compare with
//! [`MIN_SUPPORTED_VERSION`]. Probes run on a background thread to avoid UI freezing.

use std::fmt;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc::{self, Receiver};
use std::thread;
use std::time::{Duration, Instant};

use super::error::IperfError;

/// Oldest supported iperf3 version.
pub const MIN_SUPPORTED_VERSION: Version = Version {
    major: 3,
    minor: 5,
    patch: 0,
};

/// How long the `iperf3 --version` probe can run before giving up.
const PROBE_TIMEOUT: Duration = Duration::from_secs(10);

/// An iperf3 version number such as `3.5` or `3.10.1`.
///
/// Derives `PartialOrd` to allow for ordered comparison of version fields.
#[derive(Debug, Clone, Copy, PartialEq, PartialOrd)]
pub struct Version {
    /// Major version (always 3 for iperf3).
    pub major: u32,
    /// Minor version.
    pub minor: u32,
    /// Patch version; `0` when the version string has no value.
    pub patch: u32,
}

impl fmt::Display for Version {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.patch == 0 {
            write!(f, "{}.{}", self.major, self.minor)
        } else {
            write!(f, "{}.{}.{}", self.major, self.minor, self.patch)
        }
    }
}

impl Version {
    /// Extracts the version from `iperf3 --version` output.
    ///
    /// The first will always start with `iperf <version>`. Ex. `iperf 3.21`, `iperf 3.10.3`,
    /// or `3.16+` (in rare circumstances).
    pub fn parse_from_version_output(output: &str) -> Option<Version> {
        let first = output.lines().find(|l| !l.trim().is_empty())?;
        let mut words = first.split_whitespace();
        if words.next()? != "iperf" {
            return None;
        }
        let number = words.next()?;
        let mut parts = number.split('.').map(|p| {
            // Handles the case where a "+" is at the end of the version output.
            let digits: String = p.chars().take_while(char::is_ascii_digit).collect();
            digits.parse::<u32>()
        });
        let major = parts.next()?.ok()?;
        let minor = parts.next()?.ok()?;
        let patch = match parts.next() {
            Some(p) => p.ok()?,
            None => 0,
        };
        Some(Version {
            major,
            minor,
            patch,
        })
    }
}

/// An iperf3 executable that was run successfully and meets the minimum version.
#[derive(Debug, Clone)]
pub struct Iperf3Binary {
    /// Absolute or user-supplied path to the executable.
    pub path: PathBuf,
    /// Version reported by `--version`.
    pub version: Version,
}

/// A probe running on a background thread. Poll it with [`PendingProbe::try_take`].
#[derive(Debug)]
pub struct PendingProbe {
    recv: Receiver<Result<Iperf3Binary, IperfError>>,
}

impl PendingProbe {
    /// Returns the result when the probe has finished; `None` otherwise.
    pub fn try_take(&self) -> Option<Result<Iperf3Binary, IperfError>> {
        self.recv.try_recv().ok()
    }
}

/// Searches `PATH` for iperf3 and checks its version, on a background thread.
/// `notify` is called when the result is ready.
pub fn detect_in_background(notify: impl Fn() + Send + 'static) -> PendingProbe {
    spawn_probe(notify, || {
        let path = which::which("iperf3").map_err(|_| IperfError::NotOnPath)?;
        probe(&path)
    })
}

/// Checks that `path` is iperf3 [`MIN_SUPPORTED_VERSION`] or newer, on a background thread.
pub fn probe_in_background(path: PathBuf, notify: impl Fn() + Send + 'static) -> PendingProbe {
    spawn_probe(notify, move || probe(&path))
}

fn spawn_probe(
    notify: impl Fn() + Send + 'static,
    work: impl FnOnce() -> Result<Iperf3Binary, IperfError> + Send + 'static,
) -> PendingProbe {
    let (send, recv) = mpsc::channel();
    thread::spawn(move || {
        let _ = send.send(work());
        notify();
    });
    PendingProbe { recv }
}

/// Runs `path --version` and validates the result.
fn probe(path: &Path) -> Result<Iperf3Binary, IperfError> {
    if !path.is_file() {
        return Err(IperfError::NotFound(path.to_path_buf()));
    }

    let mut command = Command::new(path);
    command
        .arg("--version")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    super::runner::hide_console_window(&mut command);

    let mut child = command
        .spawn()
        .map_err(|e| IperfError::Spawn(e.to_string()))?;

    // Time out in case the chosen program never exits.
    let deadline = Instant::now() + PROBE_TIMEOUT;
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if Instant::now() >= deadline => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(IperfError::UnrecognizedVersion {
                    path: path.to_path_buf(),
                    output: "(timed out)".into(),
                });
            }
            Ok(None) => thread::sleep(Duration::from_millis(20)),
            Err(e) => return Err(IperfError::Io(e.to_string())),
        }
    }

    let mut output = String::new();
    if let Some(mut stdout) = child.stdout.take() {
        let mut bytes = Vec::new();
        stdout
            .read_to_end(&mut bytes)
            .map_err(|e| IperfError::Io(e.to_string()))?;
        output = String::from_utf8_lossy(&bytes).into_owned();
    }

    let version = Version::parse_from_version_output(&output).ok_or_else(|| {
        IperfError::UnrecognizedVersion {
            path: path.to_path_buf(),
            output: output.lines().next().unwrap_or("").trim().to_owned(),
        }
    })?;
    if version < MIN_SUPPORTED_VERSION {
        return Err(IperfError::VersionTooOld { found: version });
    }
    Ok(Iperf3Binary {
        path: path.to_path_buf(),
        version,
    })
}
