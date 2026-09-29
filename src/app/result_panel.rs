//! "Result" section: run summary or error, and "Save log..." export.

use std::path::{Path, PathBuf};

use egui::{Color32, RichText};
use network_speed_gui::iperf::{Outcome, Summary};

use super::{EndedRun, RunState, format, section};

pub(super) fn show(ui: &mut egui::Ui, run: &mut RunState) {
    section(ui, "Result", |ui| {
        match run {
            RunState::Idle => {
                ui.label(RichText::new("No test has been run yet.").weak());
            }
            RunState::Running { .. } => {
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label("Test in progress. The summary appears here when it finishes.");
                });
            }
            RunState::Ended(ended) => match &ended.outcome {
                Outcome::Finished(summary) => summary_grid(ui, summary),
                Outcome::Failed(error) => error_box(ui, &error.to_string()),
                Outcome::Cancelled => {
                    ui.label("Test cancelled. iperf3 was stopped; no summary is available.");
                }
            },
        }

        ui.add_space(8.0);
        ui.separator();
        save_row(ui, run);
    });
}

fn summary_grid(ui: &mut egui::Ui, summary: &Summary) {
    egui::Grid::new("summary_grid")
        .num_columns(3)
        .spacing([24.0, 6.0])
        .striped(true)
        .show(ui, |ui| {
            ui.label("");
            ui.label(RichText::new("Sender").strong());
            ui.label(RichText::new("Receiver").strong());
            ui.end_row();

            ui.label("Throughput");
            ui.label(format::bitrate(summary.sender.bits_per_second));
            ui.label(format::bitrate(summary.receiver.bits_per_second));
            ui.end_row();

            ui.label("Data transferred");
            ui.label(format::bytes(summary.sender.bytes));
            ui.label(format::bytes(summary.receiver.bytes));
            ui.end_row();

            ui.label("Duration");
            ui.label(format!("{:.2} s", summary.sender.end_secs));
            ui.label(format!("{:.2} s", summary.receiver.end_secs));
            ui.end_row();

            // iperf3 can't read TCP retransmits on Windows (see the parser module).
            ui.label("Retransmits");
            ui.label(RichText::new("N/A (not reported by iperf3 on Windows)").weak());
            ui.end_row();
        });
}

fn error_box(ui: &mut egui::Ui, message: &str) {
    let fill = if ui.visuals().dark_mode {
        Color32::from_rgb(60, 24, 24)
    } else {
        Color32::from_rgb(253, 236, 236)
    };
    let color = ui.visuals().error_fg_color;
    egui::Frame::new()
        .fill(fill)
        .corner_radius(4.0)
        .inner_margin(8.0)
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(RichText::new("Test failed").strong().color(color));
            ui.label(RichText::new(message).color(color));
        });
}

fn save_row(ui: &mut egui::Ui, run: &mut RunState) {
    ui.horizontal(|ui| {
        // Save is only enabled once a run has ended. The native save dialog
        // blocks the UI thread while it's open, which would pause live updates.
        let ended = match run {
            RunState::Ended(ended) => Some(ended),
            _ => None,
        };
        let button = ui
            .add_enabled(ended.is_some(), egui::Button::new("💾  Save log..."))
            .on_hover_text("Save iperf3's full output of the last test as a .log file")
            .on_disabled_hover_text("Available after a test has finished");
        let Some(ended) = ended else { return };

        if button.clicked()
            && let Some(result) = save_log_dialog(ended)
        {
            ended.save_feedback = Some(result);
        }
        match &ended.save_feedback {
            Some(Ok(path)) => {
                ui.label(RichText::new(format!("Saved to {}", path.display())).small());
            }
            Some(Err(e)) => {
                ui.label(
                    RichText::new(format!("Could not save: {e}"))
                        .small()
                        .color(ui.visuals().error_fg_color),
                );
            }
            None => {}
        }
    });
}

/// Asks where to save, then writes the log. `None` if the user cancels the dialog.
fn save_log_dialog(run: &EndedRun) -> Option<Result<PathBuf, String>> {
    let now = chrono::Local::now();
    let default_name = format!(
        "iperf3_{}_{}.log",
        sanitize_for_filename(run.config.host()),
        now.format("%Y%m%d-%H%M%S")
    );
    let path = rfd::FileDialog::new()
        .set_title("Save iperf3 log")
        .set_file_name(default_name)
        .add_filter("Log file", &["log"])
        .save_file()?;
    Some(write_log(&path, run).map(|()| path))
}

/// Writes the logs stored in `run` from iperf3 to the provided path with a short header (including settings, binary).
fn write_log(path: &Path, run: &EndedRun) -> Result<(), String> {
    let body = format!(
        "# iperf3 Network Speed Test log\n\
         # iperf3:  {} ({})\n\
         # Command: iperf3 {}\n\
         # ------------------------------ iperf3 output ------------------------------\n\
         {}",
        run.iperf.version,
        run.iperf.path.display(),
        run.config.to_args().join(" "),
        run.log,
    );
    std::fs::write(path, body).map_err(|e| e.to_string())
}

/// Replaces characters Windows doesn't allow in file names.
fn sanitize_for_filename(s: &str) -> String {
    s.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '.' || c == '-' {
                c
            } else {
                '_'
            }
        })
        .collect()
}
