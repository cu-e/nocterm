//! Disk ordering and quit deadlines without an application/executor dependency.
use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

/// Serialize disk publication. Freezing admission never waits for disk I/O.
#[derive(Clone, Default)]
pub struct WriteGate(Arc<Gate>);

#[derive(Default)]
struct Gate {
    closing: AtomicBool,
    disk: Arc<futures::lock::Mutex<()>>,
}

impl WriteGate {
    /// Stop normal workers before capturing the final immutable snapshot.
    pub fn freeze(&self) {
        self.0.closing.store(true, Ordering::Release);
    }

    pub fn is_frozen(&self) -> bool {
        self.0.closing.load(Ordering::Acquire)
    }

    /// A queued normal writer checks admission after preceding disk work ends.
    pub async fn normal(&self) -> Option<futures::lock::MutexGuard<'_, ()>> {
        let guard = self.0.disk.lock().await;
        (!self.is_frozen()).then_some(guard)
    }

    /// An off-thread publication retains this owned guard until its actual
    /// native operation ends, even when the original waiter is cancelled.
    pub async fn owned_normal(&self) -> Option<futures::lock::OwnedMutexGuard<()>> {
        let guard = self.0.disk.clone().lock_owned().await;
        (!self.is_frozen()).then_some(guard)
    }

    /// Final publication uses the same owned gate after normal admission closes.
    pub async fn owned_final_write(&self) -> futures::lock::OwnedMutexGuard<()> {
        self.0.disk.clone().lock_owned().await
    }

    /// The final writer runs after active disk work and bypasses frozen admission.
    pub async fn final_write(&self) -> futures::lock::MutexGuard<'_, ()> {
        self.0.disk.lock().await
    }
}

/// One shared budget for all phases of a shutdown, rather than a fresh timeout
/// per queued write. Owners race a self-contained background job with a timer.
#[derive(Clone, Copy)]
pub struct ShutdownDeadline(Instant);

impl ShutdownDeadline {
    pub fn new(budget: Duration) -> Self {
        Self(Instant::now() + budget)
    }

    pub fn remaining(self) -> Duration {
        self.0.saturating_duration_since(Instant::now())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures::{FutureExt as _, executor::block_on};

    #[test]
    fn an_owned_publication_outlives_its_caller_and_blocks_final_write() {
        let gate = WriteGate::default();
        let writer = block_on(gate.owned_normal()).unwrap();
        let observer = gate.clone();
        let tracked = crate::persist::InFlight::default();
        let writing = tracked.begin_write();
        let (release, wait) = std::sync::mpsc::channel();
        let work = std::thread::spawn(move || {
            let _writer = writer;
            let _writing = writing;
            wait.recv().unwrap();
        });
        drop(gate);
        observer.freeze();
        assert_eq!(tracked.count(), 1);
        assert!(observer.owned_final_write().now_or_never().is_none());
        release.send(()).unwrap();
        work.join().unwrap();
        assert_eq!(tracked.count(), 0);
        assert!(block_on(observer.owned_normal()).is_none());
        assert!(observer.owned_final_write().now_or_never().is_some());
        assert!(observer.final_write().now_or_never().is_some());
    }

    #[test]
    fn freeze_is_nonblocking_and_final_waits_for_active_write() {
        let gate = WriteGate::default();
        let active = block_on(gate.normal()).unwrap();
        gate.freeze();
        assert!(gate.final_write().now_or_never().is_none());
        drop(active);
        assert!(block_on(gate.normal()).is_none());
        assert!(gate.final_write().now_or_never().is_some());
    }
}
