//! Windows desktop front end for iperf3. This binary is only the GUI (`app`);
//! all iperf3 logic is in the library's `network_speed_gui::iperf` module.

// A Rust .exe is a console program by default, so Windows would open a
// console window next to the GUI. In release builds, mark it as a GUI program
// instead. Debug builds keep the console so panic messages stay visible.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
#![warn(missing_docs)]

mod app;

fn main() -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("iperf3 Network Speed Test")
            .with_inner_size([820.0, 900.0])
            .with_min_inner_size([640.0, 600.0]),
        ..Default::default()
    };
    eframe::run_native(
        "iperf3 Network Speed Test",
        options,
        Box::new(|cc| Ok(Box::new(app::NetworkSpeedApp::new(cc)))),
    )
}
