//! Runtimes the engine tracks: lifecycle, persistence, keep-alive,
//! streaming execution and credential consent.

pub mod manager;
pub mod state;

pub use manager::{
    Emit, FileEntry, FileListing, SessionManager, StopOutcome, DEFAULT_EXECUTE_TIMEOUT,
};
pub use state::{AssignmentView, SessionState, SessionView};
