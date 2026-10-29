//! Native millisecond clock and delays on an explicitly supplied runtime.
use futures::future::BoxFuture;
use nocterm_cloud_model::{identity::Timestamp, tool::ToolTimer};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::runtime::Handle;

/// The supervisor owns the runtime through final drain. The existing display
/// WallClock reports whole seconds; grant deadlines require millisecond reads.
pub struct NativeToolTimer {
    runtime: Handle,
}
impl NativeToolTimer {
    pub fn new(runtime: Handle) -> Self {
        Self { runtime }
    }
}
impl ToolTimer for NativeToolTimer {
    fn now(&self) -> Timestamp {
        let milliseconds = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .ok()
            .and_then(|duration| u64::try_from(duration.as_millis()).ok())
            .unwrap_or(u64::MAX);
        // An unavailable time expires finite signed windows. Existing authority
        // owners also permanently fence a later backwards observation.
        Timestamp::from_millis(milliseconds)
    }
    fn delay(&self, duration: Duration) -> BoxFuture<'static, ()> {
        let _entered = self.runtime.enter();
        Box::pin(tokio::time::sleep(duration))
    }
}
