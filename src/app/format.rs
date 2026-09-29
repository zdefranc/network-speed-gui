//! Human-readable formatting for numbers shown in the GUI and the log header.
//!
//! Units match iperf3's own: bit rates use decimal prefixes (1 Mbit = 10^6
//! bits) and byte counts use binary prefixes (1 MByte = 2^20 bytes), so the
//! numbers in the GUI line up with the raw log.

/// Formats a bit rate like iperf3 does, e.g. `65.7 Mbits/sec`.
pub fn bitrate(bits_per_second: u64) -> String {
    const UNITS: [&str; 5] = [
        "bits/sec",
        "Kbits/sec",
        "Mbits/sec",
        "Gbits/sec",
        "Tbits/sec",
    ];
    scaled(bits_per_second as f64, 1000.0, &UNITS)
}

/// Formats a byte count like iperf3 does, e.g. `23.5 MBytes`.
pub fn bytes(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["Bytes", "KBytes", "MBytes", "GBytes", "TBytes"];
    scaled(bytes as f64, 1024.0, &UNITS)
}

fn scaled(mut value: f64, step: f64, units: &[&str]) -> String {
    let mut unit = 0;
    while value >= step && unit + 1 < units.len() {
        value /= step;
        unit += 1;
    }
    // Keep three significant digits.
    let decimals = if value >= 100.0 {
        0
    } else if value >= 10.0 {
        1
    } else {
        2
    };
    format!("{value:.decimals$} {}", units[unit])
}
