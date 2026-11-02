//! The sciart binary's library target: offline QA modules shared by the cost example
//! and the test suites. The terminal app itself lives in the binary target's modules
//! (`main.rs` and siblings); this lib exists so examples and tests can link the QA code.

pub mod metrics;
