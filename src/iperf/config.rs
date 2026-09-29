//! Validated test configuration.
//!
//! A [`TestConfig`] is built using [`TestConfig::from_inputs`]. This verifies
//! correct config values before using them to launch iperf3.

/// Lowest accepted port (port 0 can't be connected to).
const MIN_PORT: u16 = 1;
/// Shortest accepted test, in seconds.
const MIN_DURATION_SECS: u32 = 1;

/// Settings for a iperf3 TCP client run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TestConfig {
    host: String,
    port: u16,
    duration_secs: u32,
}

/// Errors when validating a user's setup config ([`TestConfig`]).
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ConfigError {
    /// The host field is empty.
    #[error("Enter a server host")]
    EmptyHost,
    /// The host contains whitespace or starts with `-`.
    #[error("The host must not contain spaces or start with '-'")]
    InvalidHost,
    /// The port isn't a number from 1 to 65535.
    #[error("The port must be a whole number from {MIN_PORT} to 65535")]
    InvalidPort,
    /// The duration isn't a whole number of at least one second.
    #[error("The duration must be a whole number of seconds >= {MIN_DURATION_SECS}")]
    InvalidDuration,
}

impl TestConfig {
    /// Validates the raw text from the GUI's input fields (whitespace is trimmed).
    pub fn from_inputs(host: &str, port: &str, duration_secs: &str) -> Result<Self, ConfigError> {
        let host = host.trim();
        if host.is_empty() {
            return Err(ConfigError::EmptyHost);
        }
        // A host starting with '-' would be read by iperf3 as an option.
        if host.starts_with('-') || host.chars().any(char::is_whitespace) {
            return Err(ConfigError::InvalidHost);
        }

        let port: u16 = port.trim().parse().map_err(|_| ConfigError::InvalidPort)?;
        if port < MIN_PORT {
            return Err(ConfigError::InvalidPort);
        }

        let duration_secs: u32 = duration_secs
            .trim()
            .parse()
            .map_err(|_| ConfigError::InvalidDuration)?;
        if duration_secs < MIN_DURATION_SECS {
            return Err(ConfigError::InvalidDuration);
        }

        Ok(Self {
            host: host.to_owned(),
            port,
            duration_secs,
        })
    }

    /// Server host name or IP address.
    pub fn host(&self) -> &str {
        &self.host
    }

    /// Test length in seconds.
    pub fn duration_secs(&self) -> u32 {
        self.duration_secs
    }

    /// The iperf3 formatted command-line arguments: `-c <host> -p <port> -t <secs> -i 1 --forceflush`.
    pub fn to_args(&self) -> Vec<String> {
        vec![
            "-c".into(),
            self.host.clone(),
            "-p".into(),
            self.port.to_string(),
            "-t".into(),
            self.duration_secs.to_string(),
            "-i".into(),
            "1".into(),
            // Ensure output is flushed for every interval
            "--forceflush".into(),
        ]
    }
}
