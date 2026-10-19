//! Shared dev-only snapshot writer: record admission and gate the first outcome.
use crate::{SnapshotWriter, StoreError};
use futures::channel::oneshot;
use std::sync::{Arc, Mutex};

pub type WriteLog<T> = Arc<Mutex<Vec<T>>>;
pub type WriteRelease = oneshot::Sender<Result<(), StoreError>>;

/// One scripted first write and its exact admitted snapshot log.
pub struct GatedWriter<T> {
    writer: SnapshotWriter<T>,
    release: WriteRelease,
    writes: WriteLog<T>,
}

impl<T> GatedWriter<T> {
    pub fn into_parts(self) -> (SnapshotWriter<T>, WriteRelease, WriteLog<T>) {
        (self.writer, self.release, self.writes)
    }
}

/// Records every supplied value before waiting; only the first write is gated.
pub fn gated_writer<T: Send + 'static>() -> GatedWriter<T> {
    let (release, receive) = oneshot::channel();
    let gate = Arc::new(Mutex::new(Some(receive)));
    let writes: WriteLog<T> = Arc::new(Mutex::new(Vec::new()));
    let observed = writes.clone();
    let writer: SnapshotWriter<T> = Arc::new(move |value| {
        observed.lock().unwrap().push(value);
        let gate = gate.lock().unwrap().take();
        Box::pin(async move {
            match gate {
                Some(gate) => gate.await.unwrap(),
                None => Ok(()),
            }
        })
    });
    GatedWriter {
        writer,
        release,
        writes,
    }
}
