//! Bounded acquisition of one complete feed, before any reconciliation effects.
use nocterm_connections_model::{
    profiles::Profile,
    replica::{
        ProfileReplica, RemoteBody, RemoteProfile, ReplicaCursor, ReplicaError, ReplicaPage,
    },
};
use nocterm_core::{SharedError, bounded_io::CountingWriter};
use std::{collections::HashSet, io, sync::Arc};

const MAX_PAGES: usize = 256;
const MAX_CHANGES: usize = 16_384;
const MAX_BYTES: usize = 16 * 1024 * 1024;
const MAX_CURSOR_BYTES: usize = 4096;

#[derive(Clone, Debug, thiserror::Error)]
pub enum ReplicationFeedError {
    #[error("The profile feed exceeds {MAX_PAGES} pages; nothing was synchronized.")]
    Pages,
    #[error("The profile feed exceeds {MAX_CHANGES} changes; nothing was synchronized.")]
    Changes,
    #[error("The profile feed exceeds its 16 MiB payload budget; nothing was synchronized.")]
    Bytes,
    #[error("The profile feed cursor exceeds {MAX_CURSOR_BYTES} bytes.")]
    CursorSize,
    #[error("The profile feed cursor did not advance.")]
    NonProgress,
    #[error("The profile feed repeated an earlier cursor.")]
    Cycle,
    #[error("The profile feed did not finish within two minutes.")]
    TimedOut,
    #[error("The profile feed could not be encoded for size validation: {0}")]
    Encoding(#[source] SharedError<serde_json::Error>),
}
pub(super) enum FetchError {
    Remote(ReplicaError),
    Invalid(ReplicationFeedError),
}
impl From<ReplicationFeedError> for FetchError {
    fn from(error: ReplicationFeedError) -> Self {
        Self::Invalid(error)
    }
}

struct Feed {
    changes: Vec<RemoteProfile>,
    cursor: Option<ReplicaCursor>,
    seen: HashSet<String>,
    pages: usize,
    bytes: usize,
}
impl Feed {
    fn new(cursor: Option<ReplicaCursor>) -> Result<Self, ReplicationFeedError> {
        let mut seen = HashSet::new();
        if let Some(cursor) = &cursor {
            check_cursor(cursor)?;
            seen.insert(cursor.as_str().to_owned());
        }
        Ok(Self {
            changes: Vec::new(),
            cursor,
            seen,
            pages: 0,
            bytes: 0,
        })
    }
    fn accept(&mut self, page: ReplicaPage) -> Result<bool, ReplicationFeedError> {
        check_cursor(&page.cursor)?;
        if page.changes.len() > MAX_CHANGES.saturating_sub(self.changes.len()) {
            return Err(ReplicationFeedError::Changes);
        }
        let unchanged = self.cursor.as_ref() == Some(&page.cursor);
        if unchanged && (page.more || !page.changes.is_empty()) {
            return Err(ReplicationFeedError::NonProgress);
        }
        if !unchanged && !self.seen.insert(page.cursor.as_str().to_owned()) {
            return Err(ReplicationFeedError::Cycle);
        }
        for change in &page.changes {
            self.charge(std::mem::size_of::<RemoteProfile>())?;
            if let RemoteBody::Present(profile) = &change.body {
                self.charge(std::mem::size_of::<Profile>())?;
                let bytes = profile_bytes(profile, MAX_BYTES - self.bytes)?;
                self.charge(bytes)?;
            }
        }
        self.pages += 1;
        self.changes.extend(page.changes);
        self.cursor = Some(page.cursor);
        Ok(page.more)
    }
    fn charge(&mut self, bytes: usize) -> Result<(), ReplicationFeedError> {
        self.bytes = self
            .bytes
            .checked_add(bytes)
            .filter(|bytes| *bytes <= MAX_BYTES)
            .ok_or(ReplicationFeedError::Bytes)?;
        Ok(())
    }
}
fn check_cursor(cursor: &ReplicaCursor) -> Result<(), ReplicationFeedError> {
    if cursor.as_str().len() > MAX_CURSOR_BYTES {
        Err(ReplicationFeedError::CursorSize)
    } else {
        Ok(())
    }
}

/// Runs on the supplied background executor. A failing/partial feed is discarded;
/// it never becomes a partial plan or an advanced durable cursor.
pub(super) async fn fetch(
    replica: Arc<dyn ProfileReplica>,
    cursor: Option<ReplicaCursor>,
) -> Result<(Vec<RemoteProfile>, ReplicaCursor), FetchError> {
    let mut feed = Feed::new(cursor)?;
    loop {
        if feed.pages == MAX_PAGES {
            return Err(ReplicationFeedError::Pages.into());
        }
        let page = replica
            .changes(feed.cursor.clone())
            .await
            .map_err(FetchError::Remote)?;
        if !feed.accept(page)? {
            break;
        }
    }
    Ok((
        feed.changes,
        feed.cursor.unwrap_or_else(|| ReplicaCursor::new("")),
    ))
}

fn profile_bytes(profile: &Profile, limit: usize) -> Result<usize, ReplicationFeedError> {
    let mut writer = CountingWriter::new(limit);
    match serde_json::to_writer(&mut writer, profile) {
        Ok(()) => Ok(writer.count()),
        Err(error) if error.io_error_kind() == Some(io::ErrorKind::OutOfMemory) => {
            Err(ReplicationFeedError::Bytes)
        }
        Err(error) => Err(ReplicationFeedError::Encoding(SharedError::from_error(
            error,
        ))),
    }
}
