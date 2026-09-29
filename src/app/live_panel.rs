//! "Live throughput" section: the latest interval as a number, plus a line chart.

use std::collections::VecDeque;

use egui::RichText;
use egui_plot::{Line, Plot, PlotBounds, PlotPoints, Points};
use network_speed_gui::iperf::Measurement;

use super::{RunState, format, section};

/// Most intervals shown on the chart.
const MAX_PLOT_POINTS: usize = 30;

/// The newest [`MAX_PLOT_POINTS`] interval measurements.
#[derive(Default)]
pub(super) struct LiveSeries {
    /// `[interval end time in seconds, Mbits/sec]`, oldest first.
    points: VecDeque<[f64; 2]>,
    latest_bps: Option<u64>,
    /// Intervals received in this run (not capped, unlike `points`).
    count: usize,
}

impl LiveSeries {
    pub(super) fn push(&mut self, m: &Measurement) {
        if self.points.len() == MAX_PLOT_POINTS {
            self.points.pop_front();
        }
        self.points
            .push_back([m.end_secs, m.bits_per_second as f64 / 1e6]);
        self.latest_bps = Some(m.bits_per_second);
        self.count += 1;
    }

    pub(super) fn clear(&mut self) {
        *self = Self::default();
    }

    /// Chart area: x covers the visible points; y goes from 0 to the peak plus 15% (to give room at the top of the plot).
    /// Without explicit bounds, egui_plot shows -0.5..0.5 for an empty chart.
    fn plot_bounds(&self) -> PlotBounds {
        let (first_x, last_x) = match (self.points.front(), self.points.back()) {
            (Some(first), Some(last)) => (first[0], last[0]),
            _ => (0.0, 10.0),
        };
        let min_x = (first_x - 1.0).max(0.0);
        let max_x = last_x + 0.5; // +0.5 so the last marker isn't clipped
        let peak = self.points.iter().map(|p| p[1]).fold(0.0_f64, f64::max);
        let max_y = if peak > 0.0 { peak * 1.15 } else { 1.0 };
        PlotBounds::from_min_max([min_x, 0.0], [max_x, max_y])
    }
}

pub(super) fn show(ui: &mut egui::Ui, live: &LiveSeries, run: &RunState) {
    section(ui, "Live throughput", |ui| {
        ui.horizontal(|ui| {
            let value = match live.latest_bps {
                Some(bps) => RichText::new(format::bitrate(bps)),
                None => RichText::new("-- Mbits/sec"),
            };
            ui.label(value.size(30.0).strong());
            ui.add_space(12.0);
            ui.vertical(|ui| {
                ui.add_space(6.0);
                ui.label(RichText::new(status_text(live, run)).weak());
            });
        });
        ui.add_space(6.0);

        let points: Vec<[f64; 2]> = live.points.iter().copied().collect();
        let bounds = live.plot_bounds();
        let accent = egui::Color32::from_rgb(66, 150, 250);
        Plot::new("throughput_plot")
            .height(210.0)
            .x_axis_label("Time (s)")
            .y_axis_label("Mbits/sec")
            // The plot is app controlled and does not require user input.
            .allow_zoom(false)
            .allow_drag(false)
            .allow_scroll(false)
            .allow_boxed_zoom(false)
            .allow_double_click_reset(false)
            .show(ui, |plot_ui| {
                plot_ui.set_plot_bounds(bounds);
                // Add points and lines connecting them.
                plot_ui.line(
                    Line::new("Throughput", PlotPoints::from(points.clone()))
                        .stroke(egui::Stroke::new(2.0, accent)),
                );
                plot_ui.points(
                    Points::new("Intervals", PlotPoints::from(points))
                        .radius(3.0)
                        .color(accent),
                );
            });
    });
}

/// Short status text next to the live number.
fn status_text(live: &LiveSeries, run: &RunState) -> String {
    match run {
        RunState::Running { config, .. } if live.count == 0 => {
            format!("Connecting to {}...", config.host())
        }
        RunState::Running { config, .. } => {
            let command = config.to_args().join(" ");
            format!(
                "Interval {} of ~{}  ·  iperf3 {command}",
                live.count,
                config.duration_secs()
            )
        }
        RunState::Ended(_) if live.count > 0 => "Latest interval of the last test".to_owned(),
        RunState::Ended(_) | RunState::Idle => "Press Start to run a test".to_owned(),
    }
}
