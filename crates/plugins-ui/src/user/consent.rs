//! What the user is asked before a package is installed or replaced, and the answer so far.

use std::{collections::BTreeSet, time::Duration};

use gpui_kit::Task;

use super::port::{Candidate, InstallRequest};

/// How long the buttons stay inert after the dialog opens, so a keystroke that was already on
/// its way cannot accept it.
pub(super) const INERT: Duration = Duration::from_millis(750);
/// The most characters of an author's reason that are quoted.
pub(super) const REASON_CHARS: usize = 160;

/// The consent dialog's state. It starts with every optional capability off, the data of a
/// replaced plugin discarded and the buttons inert.
pub(super) struct Consent {
    candidate: Candidate,
    allowed: BTreeSet<String>,
    keep_data: bool,
    downgrade_confirmed: bool,
    armed: bool,
    _timer: Task<()>,
}

impl Consent {
    pub(super) fn new(candidate: Candidate, timer: Task<()>) -> Self {
        Self {
            candidate,
            allowed: BTreeSet::new(),
            keep_data: false,
            downgrade_confirmed: false,
            armed: false,
            _timer: timer,
        }
    }

    pub(super) fn candidate(&self) -> &Candidate {
        &self.candidate
    }

    pub(super) fn arm(&mut self) {
        self.armed = true;
    }

    pub(super) fn is_armed(&self) -> bool {
        self.armed
    }

    pub(super) fn is_allowed(&self, capability: &str) -> bool {
        self.allowed.contains(capability)
    }

    pub(super) fn allow(&mut self, capability: &str, allowed: bool) {
        let optional = self
            .candidate
            .capabilities
            .iter()
            .any(|line| line.id == capability && !line.required);
        if !optional {
            return;
        }
        if allowed {
            self.allowed.insert(capability.to_owned());
        } else {
            self.allowed.remove(capability);
        }
    }

    pub(super) fn keeps_data(&self) -> bool {
        self.keep_data
    }

    pub(super) fn keep_data(&mut self, keep: bool) {
        self.keep_data = keep;
    }

    pub(super) fn confirms_downgrade(&self) -> bool {
        self.downgrade_confirmed
    }

    pub(super) fn confirm_downgrade(&mut self, confirmed: bool) {
        self.downgrade_confirmed = confirmed;
    }

    fn is_downgrade(&self) -> bool {
        self.candidate
            .replaces
            .as_ref()
            .is_some_and(|replaced| replaced.downgrade)
    }

    /// Whether the accept button works now.
    pub(super) fn can_accept(&self) -> bool {
        self.armed && (!self.is_downgrade() || self.downgrade_confirmed)
    }

    pub(super) fn request(&self) -> InstallRequest {
        InstallRequest {
            location: self.candidate.location.clone(),
            hash: self.candidate.hash.clone(),
            replaces: self
                .candidate
                .replaces
                .as_ref()
                .map(|replaced| replaced.hash.clone()),
            accepted: self.allowed.iter().cloned().collect(),
            keep_data: self.keep_data,
        }
    }
}

/// An author's reason as it is quoted: cut at [`REASON_CHARS`] characters.
pub(super) fn quote(reason: &str) -> String {
    let mut quoted: String = reason.chars().take(REASON_CHARS).collect();
    if reason.chars().count() > REASON_CHARS {
        quoted.push('\u{2026}');
    }
    quoted
}
