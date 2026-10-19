use super::{FsOperation, Gate};
use crate::{DirEntry, FsError};
use futures::channel::oneshot;
use std::collections::VecDeque;

struct Rule {
    operation: FsOperation,
    path: String,
    gate: Option<Gate>,
    error: Option<FsError>,
}

#[derive(Default)]
pub(super) struct Control {
    rules: VecDeque<Rule>,
    scripted_listings: bool,
    pending_listings: VecDeque<PendingListing>,
}

type ListingResult = Result<Vec<DirEntry>, FsError>;
type ListingReceiver = oneshot::Receiver<ListingResult>;

/// An explicitly scripted protocol response; the coherent tree remains unchanged.
pub struct PendingListing {
    path: String,
    reply: oneshot::Sender<ListingResult>,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ListingReplyError {
    #[error("scripted listing exceeds the fake response bound")]
    TooLarge,
    #[error("the listing request was canceled")]
    Closed,
}

impl PendingListing {
    pub fn path(&self) -> &str {
        &self.path
    }

    /// Allows invalid protocol names for consumer validation tests, while keeping
    /// every response bounded (200,000 entries / 16 MiB of names).
    pub fn send(self, result: Result<Vec<DirEntry>, FsError>) -> Result<(), ListingReplyError> {
        if let Ok(entries) = &result {
            let bytes = entries
                .iter()
                .try_fold(0usize, |sum, entry| sum.checked_add(entry.name.len()));
            if entries.len() > 200_000 || bytes.is_none_or(|bytes| bytes > 16 * 1024 * 1024) {
                return Err(ListingReplyError::TooLarge);
            }
        }
        self.reply
            .send(result)
            .map_err(|_| ListingReplyError::Closed)
    }
}

impl Control {
    pub(super) fn script_listings(&mut self) {
        self.scripted_listings = true;
    }

    pub(super) fn has_pending_listings(&self) -> bool {
        !self.pending_listings.is_empty()
    }

    pub(super) fn take_pending_listing(&mut self) -> Option<PendingListing> {
        self.pending_listings.pop_front()
    }

    pub(super) fn listing(&mut self, path: &str) -> Result<Option<ListingReceiver>, FsError> {
        if !self.scripted_listings {
            return Ok(None);
        }
        if self.pending_listings.len() >= 64 {
            return Err(FsError::Unsupported("pending fake listing bound".into()));
        }
        let (reply, receive) = oneshot::channel();
        self.pending_listings.push_back(PendingListing {
            path: path.into(),
            reply,
        });
        Ok(Some(receive))
    }

    pub(super) fn add(
        &mut self,
        operation: FsOperation,
        path: String,
        gate: Option<Gate>,
        error: Option<FsError>,
    ) {
        self.rules.push_back(Rule {
            operation,
            path,
            gate,
            error,
        });
    }

    pub(super) fn take(
        &mut self,
        operation: FsOperation,
        path: &str,
    ) -> (Option<Gate>, Option<FsError>) {
        let Some(index) = self
            .rules
            .iter()
            .position(|rule| rule.operation == operation && rule.path == path)
        else {
            return (None, None);
        };
        self.rules
            .remove(index)
            .map_or((None, None), |rule| (rule.gate, rule.error))
    }
}
