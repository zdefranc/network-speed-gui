//! Core library for the iperf3 network speed GUI.
//!
//! The GUI lives in the binary (`main.rs` + `app/`) and can only use what
//! [`iperf`] makes `pub`, so the compiler enforces the GUI/core boundary.

#![warn(missing_docs)]

pub mod iperf;
