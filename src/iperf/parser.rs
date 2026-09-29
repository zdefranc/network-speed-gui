//! Line-by-line parser for iperf3's output.
//!
//! [`OutputParser::parse_line`] takes one line and says what it is: an
//! interval, one of the two summary rows, an error, or nothing of interest.
//!
//! Interval rows and summary rows have the same shape:
//!
//! ```text
//! <preamble>
//! [ ID] Interval           Transfer     Bitrate
//! [  5]   0.00-1.01   sec   825 KBytes  6.68 Mbits/sec                  <- interval
//! - - - - - - - - - - - - - - - - - - - - - - - - -                     <- separator
//! [ ID] Interval           Transfer     Bitrate
//! [  5]   0.00-3.00   sec  23.5 MBytes  65.7 Mbits/sec                  sender
//! [  5]   0.00-3.18   sec  21.9 MBytes  57.7 Mbits/sec                  receiver
//! ```
//!
//! The parser is a small state machine: `Preamble` → (first `[ ID]` header) →
//! `Intervals` → (`- - -` separator) → `Summary`. In `Summary`, a row must also
//! end in `sender` or `receiver`. Rows are split on whitespace because iperf3's column
//! widths change with the numbers.
//!
//! **Retransmits are not parsed.** iperf3 reads its `Retr` column from the
//! kernel's `TCP_INFO`, which iperf3 does not support on Windows
//! (<https://github.com/esnet/iperf/discussions/1957>), so the column never
//! appears in Windows output.

use std::sync::LazyLock;

use regex::Regex;

/// One throughput measurement: a live interval or a summary row.
#[derive(Debug, Clone, PartialEq)]
pub struct Measurement {
    /// Interval start, in seconds since the test began.
    pub start_secs: f64,
    /// Interval end, in seconds since the test began.
    pub end_secs: f64,
    /// Data transferred, in bytes.
    pub bytes: u64,
    /// Throughput in bits per second.
    pub bits_per_second: u64,
}

/// Information determined from a parsed line of iperf3 output.
#[derive(Debug, Clone, PartialEq)]
pub(super) enum ParsedLine {
    /// A live per-interval measurement.
    Interval(Measurement),
    /// The final `sender` summary row.
    SenderSummary(Measurement),
    /// The final `receiver` summary row.
    ReceiverSummary(Measurement),
    /// An `iperf3: error - <message>` line; carries `<message>`.
    Error(String),
    /// Anything else: banners, headers, warnings, blank lines, `iperf Done.`.
    Ignored,
}

/// The phase of the current iperf3 output.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum Phase {
    #[default]
    Preamble,
    Intervals,
    Summary,
}

/// Stateful parser for one iperf3 run. Create a new one for every run.
#[derive(Debug, Default)]
pub(super) struct OutputParser {
    phase: Phase,
}

impl OutputParser {
    /// Classifies one line of output.
    pub(super) fn parse_line(&mut self, line: &str) -> ParsedLine {
        let line = line.trim();

        if let Some(message) = error_message(line) {
            return ParsedLine::Error(message.to_owned());
        }
        if line.starts_with("[ ID]") {
            // The header appears before the intervals and again after the
            // separator. Only the first one changes the phase; the "- - -"
            // separator is what switches to Summary.
            if self.phase == Phase::Preamble {
                self.phase = Phase::Intervals;
            }
            return ParsedLine::Ignored;
        }
        if line.starts_with("- - -") {
            self.phase = Phase::Summary;
            return ParsedLine::Ignored;
        }

        let Some(row) = parse_data_row(line) else {
            return ParsedLine::Ignored;
        };
        match (self.phase, row.role) {
            (Phase::Intervals, Role::Interval) => ParsedLine::Interval(row.measurement),
            (Phase::Summary, Role::Sender) => ParsedLine::SenderSummary(row.measurement),
            (Phase::Summary, Role::Receiver) => ParsedLine::ReceiverSummary(row.measurement),
            // Ignore data rows returned in the wrong phase.
            _ => ParsedLine::Ignored,
        }
    }
}

/// Extracts `<message>` from `iperf3: error - <message>`.
fn error_message(line: &str) -> Option<&str> {
    const MARKER: &str = "iperf3: error - ";
    line.find(MARKER)
        .map(|i| line[i + MARKER.len()..].trim())
        .filter(|m| !m.is_empty())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Role {
    Interval,
    Sender,
    Receiver,
}

struct DataRow {
    measurement: Measurement,
    role: Role,
}

/// A data row starts with a numeric stream id in brackets, e.g. `[  5]`.
/// Capture 1 is everything after it. `LazyLock` compiles the regex once, the first time it's used.
static DATA_ROW: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^\[\s*\d+\]\s*(.*)$").expect("valid regex"));

/// Parses `[ id] start-end sec <n> <unit> <n> <unit> [extra columns] [sender|receiver]`.
///
/// Returns `None` for anything that doesn't match this shape.
fn parse_data_row(line: &str) -> Option<DataRow> {
    let rest = DATA_ROW.captures(line)?.get(1)?.as_str();

    let tokens: Vec<&str> = rest.split_whitespace().collect();
    // Minimum shape: interval, "sec", amount, unit, rate, unit.
    if tokens.len() < 6 || tokens[1] != "sec" {
        return None;
    }

    let (start, end) = tokens[0].split_once('-')?;
    let start_secs: f64 = start.parse().ok()?;
    let end_secs: f64 = end.parse().ok()?;
    let bytes = parse_bytes(tokens[2], tokens[3])?;
    let bits_per_second = parse_bitrate(tokens[4], tokens[5])?;

    let role = match tokens.last() {
        Some(&"sender") => Role::Sender,
        Some(&"receiver") => Role::Receiver,
        _ => Role::Interval,
    };

    Some(DataRow {
        measurement: Measurement {
            start_secs,
            end_secs,
            bytes,
            bits_per_second,
        },
        role,
    })
}

/// Converts iperf3's transfer column to bytes.
fn parse_bytes(value: &str, unit: &str) -> Option<u64> {
    let value: f64 = value.parse().ok()?;
    let multiplier: f64 = match unit {
        "Bytes" => 1.0,
        "KBytes" => 1024.0,
        "MBytes" => 1024.0 * 1024.0,
        "GBytes" => 1024.0 * 1024.0 * 1024.0,
        "TBytes" => 1024.0 * 1024.0 * 1024.0 * 1024.0,
        _ => return None,
    };
    Some((value * multiplier).round() as u64)
}

/// Converts iperf3's bitrate column to bits per second.
fn parse_bitrate(value: &str, unit: &str) -> Option<u64> {
    let value: f64 = value.parse().ok()?;
    let multiplier = match unit {
        "bits/sec" => 1.0,
        "Kbits/sec" => 1e3,
        "Mbits/sec" => 1e6,
        "Gbits/sec" => 1e9,
        "Tbits/sec" => 1e12,
        _ => return None,
    };
    Some((value * multiplier).round() as u64)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Runs every line of `output` through a fresh parser.
    fn parse_all(output: &str) -> Vec<ParsedLine> {
        let mut parser = OutputParser::default();
        output.lines().map(|l| parser.parse_line(l)).collect()
    }

    fn intervals(parsed: &[ParsedLine]) -> Vec<&Measurement> {
        parsed
            .iter()
            .filter_map(|p| match p {
                ParsedLine::Interval(m) => Some(m),
                _ => None,
            })
            .collect()
    }

    fn find_sender(parsed: &[ParsedLine]) -> &Measurement {
        parsed
            .iter()
            .find_map(|p| match p {
                ParsedLine::SenderSummary(m) => Some(m),
                _ => None,
            })
            .expect("sender summary")
    }

    fn find_receiver(parsed: &[ParsedLine]) -> &Measurement {
        parsed
            .iter()
            .find_map(|p| match p {
                ParsedLine::ReceiverSummary(m) => Some(m),
                _ => None,
            })
            .expect("receiver summary")
    }

    fn approx(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-6 * b.abs().max(1.0)
    }

    /// Real output from the iperf3 3.5 Windows (Cygwin) build.
    const WIN_3_5: &str = include_str!("../../tests/fixtures/iperf3_5_windows_success.txt");

    #[test]
    fn iperf_3_5_windows_intervals() {
        let parsed = parse_all(WIN_3_5);
        let iv = intervals(&parsed);
        assert_eq!(iv.len(), 3);

        assert!(approx(iv[0].start_secs, 0.0));
        assert!(approx(iv[0].end_secs, 1.01));
        assert_eq!(iv[0].bytes, 825 * 1024);
        assert_eq!(iv[0].bits_per_second, 6_680_000);

        assert_eq!(iv[1].bits_per_second, 83_600_000);
        assert_eq!(iv[2].bits_per_second, 108_000_000);
    }

    #[test]
    fn iperf_3_5_windows_summary() {
        let parsed = parse_all(WIN_3_5);
        let s = find_sender(&parsed);
        assert!(approx(s.end_secs, 3.00));
        assert_eq!(s.bytes, (23.5 * 1024.0 * 1024.0_f64).round() as u64);
        assert_eq!(s.bits_per_second, 65_700_000);

        let r = find_receiver(&parsed);
        assert!(approx(r.end_secs, 3.18));
        assert_eq!(r.bits_per_second, 57_700_000);
    }

    #[test]
    fn iperf_3_5_error_line() {
        let out = include_str!("../../tests/fixtures/iperf3_5_windows_connection_refused.txt");
        let parsed = parse_all(out);
        assert_eq!(
            parsed,
            vec![ParsedLine::Error(
                "unable to connect to server: Connection refused".into()
            )]
        );
    }

    /// The example output from the problem statement (iperf3 3.21).
    #[test]
    fn prompt_example_zero_intervals_and_summary() {
        let parsed = parse_all(include_str!("../../tests/fixtures/prompt_example.txt"));
        let iv = intervals(&parsed);
        assert_eq!(iv.len(), 5);
        assert_eq!(iv[0].bytes, 256 * 1024);
        // "0.00 Bytes  0.00 bits/sec" intervals are valid zero measurements.
        assert_eq!(iv[1].bytes, 0);
        assert_eq!(iv[1].bits_per_second, 0);
        assert!(approx(iv[4].end_secs, 5.01));

        let s = find_sender(&parsed);
        assert_eq!(s.bits_per_second, 6_910_000);
        let r = find_receiver(&parsed);
        assert!(approx(r.end_secs, 5.63));
        assert_eq!(r.bits_per_second, 4_600_000);
    }

    #[test]
    fn phases_follow_header_and_separator() {
        let interval = "[  5]   0.00-1.00   sec  1.00 MBytes  8.39 Mbits/sec";
        let sender = "[  5]   0.00-3.00   sec  23.5 MBytes  65.7 Mbits/sec   sender";
        let mut p = OutputParser::default();

        // Before the first header, data rows are ignored.
        assert_eq!(p.parse_line(interval), ParsedLine::Ignored);

        // After the header: intervals count, but a "sender" row isn't a summary yet.
        p.parse_line("[ ID] Interval           Transfer     Bitrate");
        assert!(matches!(p.parse_line(interval), ParsedLine::Interval(_)));
        assert_eq!(p.parse_line(sender), ParsedLine::Ignored);

        // After the separator: only sender/receiver rows count.
        p.parse_line("- - - - - - - - - - - - - - - - - - - - - - - - -");
        p.parse_line("[ ID] Interval           Transfer     Bitrate");
        assert!(matches!(p.parse_line(sender), ParsedLine::SenderSummary(_)));
    }

    #[test]
    fn every_unit_suffix() {
        assert_eq!(parse_bytes("3", "Bytes"), Some(3));
        assert_eq!(parse_bytes("1", "KBytes"), Some(1024));
        assert_eq!(parse_bytes("1.5", "MBytes"), Some(1_572_864));
        assert_eq!(parse_bytes("2", "GBytes"), Some(2 * 1024 * 1024 * 1024));
        assert_eq!(parse_bytes("1", "TBytes"), Some(1024_u64.pow(4)));
        assert_eq!(parse_bytes("1", "Kbits"), None);

        assert_eq!(parse_bitrate("5", "bits/sec"), Some(5));
        assert_eq!(parse_bitrate("2.5", "Kbits/sec"), Some(2_500));
        assert_eq!(parse_bitrate("65.7", "Mbits/sec"), Some(65_700_000));
        assert_eq!(parse_bitrate("1", "Gbits/sec"), Some(1_000_000_000));
        assert_eq!(parse_bitrate("1", "Tbits/sec"), Some(1_000_000_000_000));
        assert_eq!(parse_bitrate("1", "MBytes"), None);
        assert_eq!(parse_bitrate("abc", "Mbits/sec"), None);
    }

    #[test]
    fn malformed_rows_are_ignored() {
        let mut p = OutputParser::default();
        p.parse_line("[ ID] Interval           Transfer     Bitrate");
        for line in [
            "",
            "[  5]",
            "[  5]   0.00-1.00   sec",
            "[  5]   0.00-1.00   min  1.00 MBytes  8.39 Mbits/sec",
            "[  5]   x-1.00   sec  1.00 MBytes  8.39 Mbits/sec",
            "[  5]   0.00-1.00   sec  1.00 Furlongs  8.39 Mbits/sec",
            "[abc]   0.00-1.00   sec  1.00 MBytes  8.39 Mbits/sec",
            "random text",
        ] {
            assert_eq!(p.parse_line(line), ParsedLine::Ignored, "line: {line:?}");
        }
    }
}
