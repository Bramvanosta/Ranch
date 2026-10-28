//! Where progress goes: the desktop window, or nowhere.

use std::sync::Arc;

use crate::application::ports::ProgressSink;
use crate::domain::events::QueueEventsNotice;
use crate::domain::jobs::BulkProgress;

/// A shell without a window (Ranch server, `ranch-mcp`, tests): progress
/// is dropped and the operation's result is the report.
pub struct NoProgress;

impl NoProgress {
    pub fn sink() -> Arc<dyn ProgressSink> {
        Arc::new(Self)
    }
}

impl ProgressSink for NoProgress {
    fn bulk(&self, _progress: BulkProgress) {}
    fn queue_events(&self, _notice: QueueEventsNotice) {}
    fn shown(&self) -> bool {
        false
    }
}

/// The desktop window: each report is a Tauri event (`bulk:progress`,
/// `queue:events`).
#[cfg(feature = "desktop")]
pub struct TauriProgress(pub tauri::AppHandle);

#[cfg(feature = "desktop")]
impl TauriProgress {
    pub fn sink(app: tauri::AppHandle) -> Arc<dyn ProgressSink> {
        Arc::new(Self(app))
    }

    fn emit<S: serde::Serialize + Clone>(&self, event: &str, payload: S) {
        use tauri::Emitter;
        if let Err(e) = self.0.emit(event, payload) {
            tracing::debug!(error = %e, event, "event not delivered");
        }
    }
}

#[cfg(feature = "desktop")]
impl ProgressSink for TauriProgress {
    /// The window receives the contract's shape of each report.
    fn bulk(&self, progress: BulkProgress) {
        self.emit(
            "bulk:progress",
            crate::contract::jobs::BulkProgressEvent::from(progress),
        );
    }

    fn queue_events(&self, notice: QueueEventsNotice) {
        self.emit(
            "queue:events",
            crate::contract::queues::QueueEventsNotice::from(notice),
        );
    }

    fn shown(&self) -> bool {
        true
    }
}
