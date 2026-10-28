//! The summary of a window dragged on the Timeline.

use super::jobs::JobRef;

/// The window to summarize, and the visible range it sits in (ms since
/// epoch): "usual" is what the range did outside the window.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WindowQuery {
    pub from_ms: i64,
    pub to_ms: i64,
    pub range_from_ms: i64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct WindowCause {
    pub signature: String,
    pub sample: String,
    pub count: u32,
    /// The jobs of the cause in the window, newest first ("Open N jobs").
    pub jobs: Vec<JobRef>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct WindowSummary {
    pub from_ms: i64,
    pub to_ms: i64,
    pub done: u32,
    pub failed: u32,
    /// What the rest of the visible range did, scaled to the window's length.
    pub usual_done: f64,
    pub usual_failed: f64,
    /// Time from added to started, 95th percentile; `None` = no job started.
    pub wait_p95_ms: Option<i64>,
    pub usual_wait_p95_ms: Option<i64>,
    /// Most frequent first.
    pub causes: Vec<WindowCause>,
    /// Jobs looked at, and whether older ones were left out.
    pub scanned: u32,
    pub scan_capped: bool,
}
