/// A deterministic suspension point: release wakes every waiter, including future
/// waiters. Dropping a clone does not release the gate.
#[derive(Clone)]
pub struct Gate {
    release: async_channel::Sender<()>,
    waiters: async_channel::Receiver<()>,
}

impl Gate {
    pub fn pending() -> Self {
        let (release, waiters) = async_channel::bounded(1);
        Self { release, waiters }
    }

    pub fn open() -> Self {
        let gate = Self::pending();
        gate.release();
        gate
    }

    pub fn release(&self) {
        self.release.close();
    }

    pub async fn wait(&self) {
        let _ = self.waiters.recv().await;
    }
}
