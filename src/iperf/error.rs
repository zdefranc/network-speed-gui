//! Error type for issues that can occur locating or running iperf3.

use std::path::PathBuf;

use super::locate::{MIN_SUPPORTED_VERSION, Version};

/// Possible errors when attempting to locate or run iperf3.
#[derive(Debug, Clone, thiserror::Error)]
pub enum IperfError {
    /// No iperf3 executable was found on `PATH`.
    #[error("iperf3 was not found on PATH")]
    NotOnPath,

    /// The configured path does not point to an existing file.
    #[error("no file exists at \"{}\"", .0.display())]
    NotFound(PathBuf),

    /// The binary ran, but its `--version` output didn't look like iperf3 (or it timed out).
    #[error("\"{path}\" does not look like iperf3 (unexpected --version output: {output:?})", path = .path.display())]
    UnrecognizedVersion {
        /// The binary that was probed.
        path: PathBuf,
        /// The first line it printed.
        output: String,
    },

    /// iperf3 is older than [`MIN_SUPPORTED_VERSION`].
    #[error("iperf3 {found} is too old; version {MIN_SUPPORTED_VERSION} or newer is required")]
    VersionTooOld {
        /// The version that was found.
        found: Version,
    },

    /// The process could not be started, or a DLL it needs is missing.
    #[error("could not start iperf3: {0}")]
    Spawn(String),

    /// iperf3 printed an `iperf3: error - ...` line, e.g. "unable to connect to server".
    #[error("iperf3 error: {message}")]
    Reported {
        /// The text after `iperf3: error - `.
        message: String,
    },

    /// iperf3 exited with a non-zero code without printing an error line.
    #[error("iperf3 exited with {}{}", describe_code(*.code), describe_tail(.output_tail))]
    NonZeroExit {
        /// Exit code, if the OS reported one.
        code: Option<i32>,
        /// The last few lines of output, as a hint.
        output_tail: String,
    },

    /// iperf3 exited successfully but never printed both summary rows.
    #[error("iperf3 finished without printing a complete sender/receiver summary")]
    IncompleteOutput,

    /// Error communicating with the iperf process.
    #[error("error while communicating with iperf3: {0}")]
    Io(String),
}

fn describe_code(code: Option<i32>) -> String {
    match code {
        Some(c) => format!("exit code {c}"),
        None => "an unknown exit status".to_owned(),
    }
}

fn describe_tail(tail: &str) -> String {
    if tail.trim().is_empty() {
        String::new()
    } else {
        format!(":\n{tail}")
    }
}
