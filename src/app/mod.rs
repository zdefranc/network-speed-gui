//! The egui GUI. It only uses the `iperf` facade: start a test, cancel it,
//! and read its [`RunEvent`]s.
//!
//! One window with three sections, each in its own file:
//! 1. **Test setup** (`config_panel`): host, port, duration, iperf3 path, Start/Terminate
//! 2. **Live throughput** (`live_panel`): the latest bitrate and a chart of the last 30 intervals
//! 3. **Result** (`result_panel`): summary or error, and "Save log..."
//!
//! **Why the window never freezes:** egui redraws the whole UI from this
//! struct many times a second, so the UI thread must never wait on anything
//! slow. Slow work (probing iperf3, running a test) happens on background
//! threads inside `iperf`. The UI thread only checks their results with
//! non-blocking calls (`try_take`, `try_events`), and the background threads
//! call `ctx.request_repaint()` to wake the UI when something new arrives.

mod config_panel;
mod format;
mod live_panel;
mod result_panel;

use std::path::PathBuf;

use network_speed_gui::iperf::{
    self, Iperf3Binary, Outcome, PendingProbe, RunEvent, RunHandle, TestConfig,
};

use live_panel::LiveSeries;

/// Application state. One instance lives for the whole program.
pub struct NetworkSpeedApp {
    /// Kept to create repaint callbacks for background work.
    ctx: egui::Context,

    // Test setup inputs, as typed.
    host: String,
    port: String,
    duration: String,

    iperf_path: String,
    iperf_state: IperfState,
    /// Shows the "iperf3 not found on PATH" popup.
    show_not_found_modal: bool,

    run: RunState,
    /// Chart data. Kept after a run ends so the last test stays visible.
    live: LiveSeries,
}

/// State of finding a usable iperf3 installation.
enum IperfState {
    /// The startup search of `PATH` is running.
    DetectingOnPath(PendingProbe),
    /// A path from the user is being checked.
    Checking(PendingProbe),
    /// A valid path is found.
    Valid(Iperf3Binary),
    /// The no valid path; a String stating why.
    Invalid(String),
}

/// The lifecycle of a test.
enum RunState {
    /// No test has been run yet; the starting state of the application.
    Idle,
    Running {
        handle: RunHandle,
        config: TestConfig,
        iperf: Iperf3Binary,
    },
    Ended(EndedRun),
}

/// Everything about a finished test: what the Result section shows and what "Save log..." writes.
struct EndedRun {
    config: TestConfig,
    iperf: Iperf3Binary,
    outcome: Outcome,
    log: String,
    /// Result of the last save attempt. It's stored rather than kept in a
    /// local because egui redraws every frame: `save_row` reads it each frame
    /// to keep showing "Saved to ..." after the click.
    save_feedback: Option<Result<PathBuf, String>>,
}

impl NetworkSpeedApp {
    /// Creates the app and starts looking for iperf3 on `PATH` in the background.
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        let ctx = cc.egui_ctx.clone();
        let probe = iperf::detect_in_background(repaint(&ctx));
        Self {
            ctx,
            host: String::new(),
            port: String::new(),
            duration: String::new(),
            iperf_path: String::new(),
            iperf_state: IperfState::DetectingOnPath(probe),
            show_not_found_modal: false,
            run: RunState::Idle,
            live: LiveSeries::default(),
        }
    }

    fn is_running(&self) -> bool {
        matches!(self.run, RunState::Running { .. })
    }

    /// Validates the current inputs.
    fn config(&self) -> Result<TestConfig, iperf::ConfigError> {
        TestConfig::from_inputs(&self.host, &self.port, &self.duration)
    }

    /// The iperf3 binary to use, if one has been validated.
    fn valid_iperf(&self) -> Option<&Iperf3Binary> {
        match &self.iperf_state {
            IperfState::Valid(binary) => Some(binary),
            _ => None,
        }
    }

    /// Starts checking `self.iperf_path` in the background.
    fn check_iperf_path(&mut self) {
        // Strip quotes too: Windows' "Copy as path" wraps paths in them.
        let path = PathBuf::from(self.iperf_path.trim().trim_matches('"'));
        self.iperf_state =
            IperfState::Checking(iperf::probe_in_background(path, repaint(&self.ctx)));
    }

    /// Applies the result of a background probe, once it's ready.
    fn poll_iperf_probe(&mut self) {
        let result = match &self.iperf_state {
            IperfState::DetectingOnPath(p) | IperfState::Checking(p) => p.try_take(),
            _ => None,
        };
        let Some(result) = result else { return };
        let was_startup = matches!(self.iperf_state, IperfState::DetectingOnPath(_));

        match result {
            Ok(binary) => {
                self.iperf_path = binary.path.display().to_string();
                self.iperf_state = IperfState::Valid(binary);
            }
            Err(e) => {
                // The startup PATH search failed, show modal.
                if was_startup {
                    self.show_not_found_modal = true;
                }
                self.iperf_state = IperfState::Invalid(e.to_string());
            }
        }
    }

    fn start_test(&mut self) {
        let (Ok(config), Some(iperf)) = (self.config(), self.valid_iperf().cloned()) else {
            return;
        };
        self.live.clear();
        self.run = match iperf::start_test(&iperf.path, &config, repaint(&self.ctx)) {
            Ok(handle) => RunState::Running {
                handle,
                config,
                iperf,
            },
            Err(e) => RunState::Ended(EndedRun {
                config,
                iperf,
                outcome: Outcome::Failed(e),
                log: String::new(),
                save_feedback: None,
            }),
        };
    }

    /// Kills iperf3 and returns immediately.
    fn terminate_test(&self) {
        if let RunState::Running { handle, .. } = &self.run {
            handle.cancel();
        }
    }

    /// Applies the running test's new events.
    fn poll_run_events(&mut self) {
        let RunState::Running { handle, .. } = &self.run else {
            return;
        };
        for event in handle.try_events() {
            match event {
                RunEvent::Interval(m) => self.live.push(&m),
                RunEvent::Ended { outcome, log } => {
                    // Move the config and binary out of `Running` into `Ended`.
                    // `mem::replace` takes ownership of the old state; the
                    // `Idle` placeholder is used a temporary and overwritten on the next line.
                    // Drops the handle here as iperf3 has already exited.
                    let old = std::mem::replace(&mut self.run, RunState::Idle);
                    if let RunState::Running { config, iperf, .. } = old {
                        self.run = RunState::Ended(EndedRun {
                            config,
                            iperf,
                            outcome,
                            log,
                            save_feedback: None,
                        });
                    }
                    return;
                }
            }
        }
    }

    fn not_found_modal(&mut self, ctx: &egui::Context) {
        if !self.show_not_found_modal {
            return;
        }
        let modal = egui::Modal::new(egui::Id::new("iperf3_not_found")).show(ctx, |ui| {
            ui.set_max_width(420.0);
            ui.heading("iperf3 not found");
            ui.add_space(6.0);
            ui.label(format!(
                "No iperf3 version {} or newer was found on your PATH.",
                iperf::MIN_SUPPORTED_VERSION
            ));
            ui.label(
                "Set the iperf3 path manually in the Test setup section, either by \
                 typing it or with \"Browse...\".",
            );
            ui.add_space(8.0);
            ui.vertical_centered(|ui| ui.button("OK").clicked()).inner
        });
        if modal.inner || modal.should_close() {
            self.show_not_found_modal = false;
        }
    }
}

/// A callback that wakes the GUI from a background thread. Implements `Send + 'static`
/// so it can be moved into the threads `iperf` spawns.
fn repaint(ctx: &egui::Context) -> impl Fn() + Send + 'static {
    let ctx = ctx.clone();
    move || ctx.request_repaint()
}

impl eframe::App for NetworkSpeedApp {
    /// Runs before every frame, and also while the window is minimized, so a
    /// test that ends in the background still updates the state.
    fn logic(&mut self, _ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.poll_iperf_probe();
        self.poll_run_events();
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        egui::CentralPanel::default_margins().show(ui, |ui| {
            egui::ScrollArea::vertical()
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    ui.heading("iperf3 Network Speed Test");
                    ui.add_space(8.0);
                    self.config_section(ui);
                    ui.add_space(8.0);
                    live_panel::show(ui, &self.live, &self.run);
                    ui.add_space(8.0);
                    result_panel::show(ui, &mut self.run);
                });
        });
        let ctx = ui.ctx().clone();
        self.not_found_modal(&ctx);
    }
}

/// Draws a titled, framed section.
fn section<R>(ui: &mut egui::Ui, title: &str, add_contents: impl FnOnce(&mut egui::Ui) -> R) -> R {
    egui::Frame::group(ui.style())
        .inner_margin(12.0)
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(egui::RichText::new(title).strong().size(16.0));
            ui.add_space(6.0);
            add_contents(ui)
        })
        .inner
}
