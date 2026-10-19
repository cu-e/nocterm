//! Control notices that arrive before the session id is known wait here.
use nocterm_ai::{
    history::source::ManagedNotice,
    model::{ids::SessionId, notice::SessionUpdate},
};

const LIMIT: usize = 64;

/// Bounded by count; a newer notice of one kind replaces the older one for its session.
#[derive(Default)]
pub(super) struct PendingNotices {
    held: Vec<ManagedNotice>,
}
impl PendingNotices {
    pub(super) fn keep(&mut self, notice: ManagedNotice) {
        let update = &notice.notice().update;
        if !matches!(
            update,
            SessionUpdate::Commands(_) | SessionUpdate::Mode(_) | SessionUpdate::Configuration(_)
        ) {
            return;
        }
        let kind = std::mem::discriminant(update);
        let session = notice.notice().session.clone();
        self.held.retain(|held| {
            held.notice().session != session
                || std::mem::discriminant(&held.notice().update) != kind
        });
        if self.held.len() == LIMIT {
            self.held.remove(0);
        }
        self.held.push(notice);
    }
    pub(super) fn take_for(&mut self, session: &SessionId) -> Vec<ManagedNotice> {
        std::mem::take(&mut self.held)
            .into_iter()
            .filter(|notice| &notice.notice().session == session)
            .collect()
    }
    pub(super) fn clear(&mut self) {
        self.held.clear();
    }
}
