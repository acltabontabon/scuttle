//! Scuttle's error model.
//!
//! The filesystem is hostile and changing. A directory we cannot read, a file
//! that vanished mid-walk, a permission we were never granted — none of those
//! are exceptional. They are Tuesday. Only genuine programming faults and
//! irrecoverable storage problems are errors here; everything else is recorded
//! as a [`Hiccup`] and the scan keeps going.

use std::path::Path;

#[derive(Debug, thiserror::Error)]
pub enum ScuttleError {
    #[error("storage: {0}")]
    Storage(#[from] rusqlite::Error),

    #[error("io: {0}")]
    Io(#[from] std::io::Error),

    #[error("serialization: {0}")]
    Serde(#[from] serde_json::Error),

    /// The requested finding, quarantine record or scan does not exist.
    #[error("{what} not found")]
    NotFound { what: String },

    /// The world moved between the scan and the action. Caller must re-scan.
    #[error("{0}")]
    Stale(String),

    /// The action was refused by the safety layer. This is never a bug; it is
    /// the point of the safety layer.
    #[error("{0}")]
    Refused(String),

    #[error("scan already running")]
    ScanBusy,

    #[error("{0}")]
    Internal(String),
}

pub type Result<T> = std::result::Result<T, ScuttleError>;

impl ScuttleError {
    pub fn not_found(what: impl Into<String>) -> Self {
        Self::NotFound { what: what.into() }
    }

    /// A stable machine-readable code so the UI can pick the right tone
    /// without string-matching prose.
    pub fn code(&self) -> &'static str {
        match self {
            Self::Storage(_) => "storage",
            Self::Io(_) => "io",
            Self::Serde(_) => "serde",
            Self::NotFound { .. } => "not_found",
            Self::Stale(_) => "stale",
            Self::Refused(_) => "refused",
            Self::ScanBusy => "scan_busy",
            Self::Internal(_) => "internal",
        }
    }
}

/// A non-fatal thing that went wrong while rummaging. Collected, counted and
/// reported; never a reason to abandon a scan.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Hiccup {
    pub kind: HiccupKind,
    /// Only the final path component — full paths are private (see docs/privacy).
    pub near: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HiccupKind {
    PermissionDenied,
    Vanished,
    Unreadable,
    LoopAvoided,
    TooDeep,
}

impl Hiccup {
    pub fn new(kind: HiccupKind, path: &std::path::Path) -> Self {
        Self {
            kind,
            near: path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| "<root>".into()),
        }
    }
}

/// Classify an io error into the right hiccup, so callers do not have to.
pub fn classify_io(err: &std::io::Error, path: &Path) -> Hiccup {
    use std::io::ErrorKind::*;
    let kind = match err.kind() {
        PermissionDenied => HiccupKind::PermissionDenied,
        NotFound => HiccupKind::Vanished,
        _ => HiccupKind::Unreadable,
    };
    Hiccup::new(kind, path)
}

/// Errors cross the IPC boundary as a small tagged object, never as a
/// stringified Rust type name.
impl serde::Serialize for ScuttleError {
    fn serialize<S: serde::Serializer>(&self, s: S) -> std::result::Result<S::Ok, S::Error> {
        use serde::ser::SerializeStruct;
        let mut st = s.serialize_struct("ScuttleError", 2)?;
        st.serialize_field("code", self.code())?;
        st.serialize_field("message", &self.to_string())?;
        st.end()
    }
}
