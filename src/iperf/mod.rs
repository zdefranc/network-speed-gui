//! iperf3 module to handle state and task background execution.
//!
//! A test is:
//!
//! * configured with [`TestConfig`],
//! * started with [`start_test`], which returns a [`RunHandle`],
//! * observed with [`RunHandle::try_events`],
//! * cancelled with [`RunHandle::cancel`].
//!
//! A usable iperf3 binary is found with [`detect_in_background`] (search `PATH`)
//! or checked with [`probe_in_background`] (a user-chosen path).

mod config;
mod error;
mod job;
mod locate;
mod parser;
mod runner;

pub use config::{ConfigError, TestConfig};
pub use error::IperfError;
pub use locate::{
    Iperf3Binary, MIN_SUPPORTED_VERSION, PendingProbe, Version, detect_in_background,
    probe_in_background,
};
pub use parser::Measurement;
pub use runner::{RunHandle, start_test};

/// The result of a completed test: iperf3's two summary rows.
#[derive(Debug, Clone)]
pub struct Summary {
    /// The `sender` summary: what the client sent.
    pub sender: Measurement,
    /// The `receiver` summary: what the server reports it received.
    pub receiver: Measurement,
}

/// The outcome of the test.
#[derive(Debug, Clone)]
pub enum Outcome {
    /// iperf3 exited successfully and printed both summary rows.
    Finished(Summary),
    /// iperf3 could not be started, reported an error, or exited abnormally.
    Failed(IperfError),
    /// The user cancelled the test; the iperf3 process has exited.
    Cancelled,
}

/// Event that occured during a test run, sent from the runner's worker
/// thread.
///
/// Every run produces zero or more [`RunEvent::Interval`]s, followed by exactly one [`RunEvent::Ended`].
#[derive(Debug, Clone)]
pub enum RunEvent {
    /// The measurements reported in an interval.
    Interval(Measurement),
    /// The test has ended and iperf3 has exited. Always reported last.
    Ended {
        /// The test's outcome.
        outcome: Outcome,
        /// iperf3's complete log including stdout and stderr.
        log: String,
    },
}
