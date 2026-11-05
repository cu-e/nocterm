//! Latest pending sealed snapshot per exact account/file, within one finite writer.
use nocterm_core::persist::{Rejected, WriteQueue, Writing};
use std::{collections::HashMap, path::PathBuf};

const MAX_PENDING_FILES: usize = 8;
const MAX_PENDING_BYTES: usize = 64 * 1024 * 1024;
pub(super) const MAX_CHECKPOINT_BYTES: usize = 32 * 1024 * 1024;

#[derive(Debug, thiserror::Error)]
pub(crate) enum QueueError {
    #[error("the checkpoint exceeds its 32 MiB limit")]
    Oversized,
    #[error("the Cloud checkpoint writer is full")]
    Full,
    #[error("the Cloud checkpoint writer is closed")]
    Closed,
}

pub(super) struct PendingCheckpoints {
    order: WriteQueue<PathBuf>,
    newest: HashMap<PathBuf, Vec<u8>>,
    bytes: usize,
}
impl Default for PendingCheckpoints {
    fn default() -> Self {
        Self {
            order: WriteQueue::new(MAX_PENDING_FILES),
            newest: HashMap::new(),
            bytes: 0,
        }
    }
}
impl PendingCheckpoints {
    /// Only the first offer of a drain starts a physical writer.
    pub(super) fn offer(&mut self, path: PathBuf, bytes: Vec<u8>) -> Result<bool, QueueError> {
        if bytes.capacity() > MAX_CHECKPOINT_BYTES {
            return Err(QueueError::Oversized);
        }
        let replaced = self.newest.get(&path).map_or(0, Vec::capacity);
        let retained = self.bytes - replaced + bytes.capacity();
        if retained > MAX_PENDING_BYTES {
            return Err(QueueError::Full);
        }
        let start = if replaced > 0 || self.newest.contains_key(&path) {
            false
        } else {
            self.order.push(path.clone()).map_err(|error| match error {
                Rejected::Full => QueueError::Full,
                Rejected::Closing => QueueError::Closed,
            })?
        };
        self.newest.insert(path, bytes);
        self.bytes = retained;
        Ok(start)
    }
    pub(super) fn take(&mut self) -> Option<(PathBuf, Vec<u8>, Writing)> {
        let path = self.order.take()?;
        let bytes = self.newest.remove(&path)?;
        self.bytes -= bytes.capacity();
        Some((path, bytes, self.order.begin_write()))
    }
    #[cfg(test)]
    pub(super) fn idle(&self) -> bool {
        self.order.is_empty() && self.order.in_flight().count() == 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repeated_pending_snapshots_keep_only_the_latest_bytes_for_each_file() {
        let mut queue = PendingCheckpoints::default();
        let path = PathBuf::from("account/chat");
        assert!(queue.offer(path.clone(), b"first".to_vec()).unwrap());
        for _ in 0..1000 {
            assert!(!queue.offer(path.clone(), b"latest".to_vec()).unwrap());
        }
        let (saved, bytes, writing) = queue.take().unwrap();
        assert_eq!((saved, bytes), (path.clone(), b"latest".to_vec()));
        assert!(!queue.offer(path, b"after in-flight".to_vec()).unwrap());
        drop(writing);
        assert_eq!(queue.take().unwrap().1, b"after in-flight");
        assert!(queue.take().is_none());
        assert!(queue.idle());
    }

    #[test]
    fn a_full_writer_preserves_admitted_snapshots_and_accepts_a_replacement() {
        let mut queue = PendingCheckpoints::default();
        for index in 0..MAX_PENDING_FILES {
            queue
                .offer(PathBuf::from(index.to_string()), vec![index as u8])
                .unwrap();
        }
        assert!(matches!(
            queue.offer(PathBuf::from("extra"), vec![9]),
            Err(QueueError::Full)
        ));
        queue.offer(PathBuf::from("0"), vec![42]).unwrap();
        assert_eq!(queue.take().unwrap().1, vec![42]);
        assert!(queue.offer(PathBuf::from("extra"), vec![9]).is_ok());
    }

    #[test]
    fn byte_admission_counts_retained_capacity_and_replacement_releases_it() {
        let mut queue = PendingCheckpoints::default();
        for name in ["one", "two"] {
            let mut bytes = Vec::with_capacity(MAX_CHECKPOINT_BYTES);
            bytes.push(1);
            queue.offer(PathBuf::from(name), bytes).unwrap();
        }
        assert!(matches!(
            queue.offer(PathBuf::from("three"), vec![3]),
            Err(QueueError::Full)
        ));
        queue.offer(PathBuf::from("one"), vec![1]).unwrap();
        assert!(queue.offer(PathBuf::from("three"), vec![3]).is_ok());
    }
}
