//! The tool calls in progress, so `ranch-mcp` can stop between two calls:
//! once it is told to stop, no new call starts, and it exits when the last
//! one running is done (an approved write and its audit row are never cut
//! apart).

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::{Duration, Instant};

static STOPPING: AtomicBool = AtomicBool::new(false);
static IN_FLIGHT: AtomicUsize = AtomicUsize::new(0);

/// A call in progress; counted until dropped.
pub struct Call(());

impl Drop for Call {
    fn drop(&mut self) {
        IN_FLIGHT.fetch_sub(1, Ordering::SeqCst);
    }
}

/// Start a call, unless the process is stopping.
pub fn begin() -> Option<Call> {
    IN_FLIGHT.fetch_add(1, Ordering::SeqCst);
    let call = Call(());
    (!STOPPING.load(Ordering::SeqCst)).then_some(call)
}

/// Refuse new calls from now on, then wait until none is running, or
/// `at_most` has passed.
pub async fn stop(at_most: Duration) {
    STOPPING.store(true, Ordering::SeqCst);
    let deadline = Instant::now() + at_most;
    while IN_FLIGHT.load(Ordering::SeqCst) > 0 && Instant::now() < deadline {
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}
