//! "Test setup" section: host, port, duration, iperf3 path, and Start/Terminate.

use egui::{Button, Color32, RichText, TextEdit};

use super::{IperfState, NetworkSpeedApp, section};

impl NetworkSpeedApp {
    pub(super) fn config_section(&mut self, ui: &mut egui::Ui) {
        let running = self.is_running();
        section(ui, "Test setup", |ui| {
            // Inputs are locked during a test, so they always match what's running.
            ui.add_enabled_ui(!running, |ui| {
                egui::Grid::new("config_grid")
                    .num_columns(2)
                    .spacing([12.0, 8.0])
                    .min_col_width(110.0)
                    .show(ui, |ui| {
                        ui.label("Server host");
                        ui.add(
                            TextEdit::singleline(&mut self.host)
                                .hint_text("hostname or IP address")
                                .desired_width(320.0),
                        );
                        ui.end_row();

                        ui.label("Port");
                        digits_only_field(ui, &mut self.port, "e.g. 5201");
                        ui.end_row();

                        ui.label("Duration (s)");
                        digits_only_field(ui, &mut self.duration, "e.g. 10");
                        ui.end_row();

                        ui.label("iperf3 path");
                        self.iperf_path_row(ui);
                        ui.end_row();

                        ui.label("");
                        self.iperf_status_label(ui);
                        ui.end_row();
                    });
            });

            ui.add_space(8.0);
            self.start_terminate_buttons(ui);
        });
    }

    fn iperf_path_row(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            let response = ui.add(
                TextEdit::singleline(&mut self.iperf_path)
                    .hint_text(r"C:\path\to\iperf3.exe")
                    .desired_width(420.0),
            );
            // Check the iperf path when the user leaves the field.
            if response.lost_focus() && !matches!(self.iperf_state, IperfState::DetectingOnPath(_))
            {
                self.check_iperf_path();
            }
            if ui.button("Browse...").clicked()
                && let Some(path) = rfd::FileDialog::new()
                    .set_title("Select iperf3.exe")
                    .add_filter("iperf3 executable", &["exe"])
                    .pick_file()
            {
                self.iperf_path = path.display().to_string();
                self.check_iperf_path();
            }
        });
    }

    fn iperf_status_label(&self, ui: &mut egui::Ui) {
        let text = match &self.iperf_state {
            IperfState::DetectingOnPath(_) => RichText::new("Searching PATH for iperf3..."),
            IperfState::Checking(_) => RichText::new("Checking iperf3..."),
            IperfState::Valid(b) => {
                RichText::new(format!("✔ iperf3 {} ready", b.version)).color(ok_color(ui))
            }
            IperfState::Invalid(reason) => {
                RichText::new(format!("✖ {reason}")).color(ui.visuals().error_fg_color)
            }
        };
        ui.label(text.small());
    }

    fn start_terminate_buttons(&mut self, ui: &mut egui::Ui) {
        let running = self.is_running();
        let config = self.config();
        let can_start = !running && config.is_ok() && self.valid_iperf().is_some();

        ui.horizontal(|ui| {
            // Only one of Start / Terminate is ever enabled.
            let start =
                Button::new(RichText::new("▶  Start").size(15.0)).min_size([110.0, 30.0].into());
            if ui.add_enabled(can_start, start).clicked() {
                self.start_test();
            }
            let stop = Button::new(RichText::new("■  Terminate").size(15.0))
                .min_size([110.0, 30.0].into());
            if ui.add_enabled(running, stop).clicked() {
                self.terminate_test();
            }

            // Say why Start is disabled.
            if !running {
                let hint = match (&config, self.valid_iperf()) {
                    (Err(e), _) => Some(e.to_string()),
                    (Ok(_), None) => Some("Set a valid iperf3 path to start".to_owned()),
                    _ => None,
                };
                if let Some(hint) = hint {
                    ui.label(
                        RichText::new(hint)
                            .small()
                            .color(ui.visuals().warn_fg_color),
                    );
                }
            }
        });
    }
}

/// A text field that only keeps digits.
fn digits_only_field(ui: &mut egui::Ui, text: &mut String, hint: &str) {
    let response = ui.add(
        TextEdit::singleline(text)
            .hint_text(hint)
            .char_limit(5)
            .desired_width(80.0),
    );
    if response.changed() {
        text.retain(|c| c.is_ascii_digit());
    }
}

fn ok_color(ui: &egui::Ui) -> Color32 {
    if ui.visuals().dark_mode {
        Color32::from_rgb(110, 200, 120)
    } else {
        Color32::from_rgb(20, 130, 40)
    }
}
