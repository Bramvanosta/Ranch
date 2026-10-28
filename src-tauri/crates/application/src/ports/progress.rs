use ranch_domain::events::QueueEventsNotice;
use ranch_domain::jobs::BulkProgress;

/// What the core reports while it works: how far a bulk action or a
/// maintenance run has got, and which queues of a followed connection
/// moved. Best effort: the operation's result is what counts.
pub trait ProgressSink: Send + Sync {
    /// A bulk action or a maintenance run moved on.
    fn bulk(&self, progress: BulkProgress);
    /// Queues of a followed connection moved.
    fn queue_events(&self, notice: QueueEventsNotice);
    /// Whether anything shows what is sent. Following a connection's
    /// events is not started when nothing does: the front end keeps polling.
    fn shown(&self) -> bool;
}
