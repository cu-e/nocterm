//! Snapshot identity and dialog/save lifecycle.
use crate::store::{Profile, ProfileId};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
pub(super) struct EditState {
    id: ProfileId,
    original: Option<Profile>,
    pending: bool,
    dismissed: Dismissal,
}
impl EditState {
    pub(super) fn new(original: Option<Profile>) -> Self {
        Self {
            id: original
                .as_ref()
                .map_or_else(ProfileId::generate, |profile| profile.id),
            original,
            pending: false,
            dismissed: Default::default(),
        }
    }
    pub(super) fn id(&self) -> ProfileId {
        self.id
    }
    pub(super) fn original(&self) -> Option<&Profile> {
        self.original.as_ref()
    }
    pub(super) fn editing(&self) -> bool {
        self.original.is_some()
    }
    pub(super) fn pending(&self) -> bool {
        self.pending
    }
    pub(super) fn dismissal(&self) -> Dismissal {
        self.dismissed.clone()
    }
    pub(super) fn is_dismissed(&self) -> bool {
        self.dismissed.is_dismissed()
    }
    pub(super) fn dismiss(&self) {
        self.dismissed.dismiss();
    }
    pub(super) fn try_begin(&mut self) -> bool {
        if self.pending || self.is_dismissed() {
            return false;
        }
        self.pending = true;
        true
    }
    pub(super) fn finish(&mut self) -> bool {
        self.pending = false;
        !self.is_dismissed()
    }
    pub(super) fn accept(&mut self, profile: Profile) {
        self.original = Some(profile);
    }
}

/// Cloned dialog callback capability permits only monotonic dismissal.
#[derive(Clone, Default)]
pub(super) struct Dismissal {
    closed: Arc<AtomicBool>,
}
impl Dismissal {
    pub(super) fn dismiss(&self) {
        self.closed.store(true, Ordering::Release);
    }
    fn is_dismissed(&self) -> bool {
        self.closed.load(Ordering::Acquire)
    }
}
