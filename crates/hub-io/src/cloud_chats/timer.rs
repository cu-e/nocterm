//! The wall clock and delays of Cloud chat waits and sources, on the adapter's runtime.

use std::time::Duration;

use futures::future::BoxFuture;
use nocterm_cloud_model::{identity::Timestamp, tool::ToolTimer};
use tokio::runtime::Handle;

pub(super) struct NativeTimer {
    runtime: Handle,
}

impl NativeTimer {
    pub(super) fn new(runtime: Handle) -> Self {
        Self { runtime }
    }
}

impl ToolTimer for NativeTimer {
    fn now(&self) -> Timestamp {
        // A clock before the epoch reads as zero: finite windows then never open early.
        Timestamp::from_millis(u64::try_from(nocterm_hub_client::now_ms()).unwrap_or(0))
    }

    fn delay(&self, duration: Duration) -> BoxFuture<'static, ()> {
        let _entered = self.runtime.enter();
        Box::pin(tokio::time::sleep(duration))
    }
}
