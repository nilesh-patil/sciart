//! The library's one error type (LIB-R1, LIB-R6).

use std::fmt;

/// What can go wrong when building, stepping or drawing a [`Stage`](crate::Stage).
#[non_exhaustive]
#[derive(Debug)]
pub enum Error {
    /// No scene, and no retired scene name, is called this.
    UnknownScene(String),
    /// The named scene panicked. The stage stays failed: every later call returns this
    /// error again. Only reported when the crate is built with unwinding; with
    /// `panic = "abort"` the process aborts instead.
    ScenePanicked(&'static str),
    /// Writing the output failed.
    Io(std::io::Error),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::UnknownScene(name) => write!(f, "unknown scene '{name}'"),
            Error::ScenePanicked(name) => write!(f, "scene '{name}' panicked"),
            Error::Io(e) => write!(f, "write failed: {e}"),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Error::Io(e) => Some(e),
            _ => None,
        }
    }
}

impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        Error::Io(e)
    }
}
