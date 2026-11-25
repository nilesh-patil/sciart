//! The library's one error type (LIB-R1, LIB-R3, LIB-R6), opaque in the manner of
//! `std::io::Error`: a fieldless [`ErrorKind`] to match on, accessors for the rest.

use std::fmt;

/// What kind of thing went wrong; see [`Error::kind`].
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ErrorKind {
    /// No scene, and no retired scene name, is called this; [`Error::scene`] has the name.
    UnknownScene,
    /// The scene panicked. The stage stays failed: every later call returns this kind
    /// again. Only reported when the crate is built with unwinding; with
    /// `panic = "abort"` the process aborts instead.
    ScenePanicked,
    /// An argument was out of range, e.g. a negative, non-finite or too long `advance`.
    InvalidInput,
    /// Writing the output failed; [`std::error::Error::source`] has the I/O error.
    Io,
}

/// An error from building, stepping or drawing a [`Stage`](crate::Stage).
#[derive(Debug)]
pub struct Error {
    kind: ErrorKind,
    scene: Option<String>,
    detail: &'static str,
    io: Option<std::io::Error>,
}

impl Error {
    /// What kind of error this is.
    pub fn kind(&self) -> ErrorKind {
        self.kind
    }

    /// The scene the error is about, when there is one: the unknown name, or the scene
    /// that panicked.
    pub fn scene(&self) -> Option<&str> {
        self.scene.as_deref()
    }

    fn new(kind: ErrorKind, scene: Option<String>, detail: &'static str) -> Self {
        Self {
            kind,
            scene,
            detail,
            io: None,
        }
    }

    pub(crate) fn unknown_scene(name: &str) -> Self {
        Self::new(ErrorKind::UnknownScene, Some(name.to_owned()), "")
    }

    pub(crate) fn panicked(scene: &str) -> Self {
        Self::new(ErrorKind::ScenePanicked, Some(scene.to_owned()), "")
    }

    pub(crate) fn invalid(detail: &'static str) -> Self {
        Self::new(ErrorKind::InvalidInput, None, detail)
    }

    #[cfg_attr(not(feature = "png"), allow(dead_code))] // only the PNG writer does I/O
    pub(crate) fn io(e: std::io::Error) -> Self {
        Self {
            io: Some(e),
            ..Self::new(ErrorKind::Io, None, "")
        }
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let scene = self.scene.as_deref().unwrap_or("");
        match (self.kind, &self.io) {
            (ErrorKind::UnknownScene, _) => write!(f, "unknown scene '{scene}'"),
            (ErrorKind::ScenePanicked, _) => write!(f, "scene '{scene}' panicked"),
            (ErrorKind::Io, Some(e)) => write!(f, "write failed: {e}"),
            _ => f.write_str(if self.detail.is_empty() {
                "invalid input"
            } else {
                self.detail
            }),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        self.io
            .as_ref()
            .map(|e| e as &(dyn std::error::Error + 'static))
    }
}

#[cfg(test)]
mod tests {
    use std::error::Error as _;

    use super::*;

    #[test]
    fn lib_r3_errors_expose_a_kind_a_scene_and_a_source() {
        let unknown = Error::unknown_scene("nebula");
        assert_eq!(unknown.kind(), ErrorKind::UnknownScene);
        assert_eq!(unknown.scene(), Some("nebula"));
        assert!(unknown.source().is_none());
        assert_eq!(unknown.to_string(), "unknown scene 'nebula'");

        let panicked = Error::panicked("aurora");
        assert_eq!(
            (panicked.kind(), panicked.scene()),
            (ErrorKind::ScenePanicked, Some("aurora"))
        );

        let invalid = Error::invalid("advance takes 0..=3600 seconds");
        assert_eq!(
            (invalid.kind(), invalid.scene()),
            (ErrorKind::InvalidInput, None)
        );
        assert_eq!(invalid.to_string(), "advance takes 0..=3600 seconds");

        let io = Error::io(std::io::Error::other("disk full"));
        assert_eq!(io.kind(), ErrorKind::Io);
        assert_eq!(
            io.source().map(ToString::to_string).as_deref(),
            Some("disk full")
        );
    }
}
