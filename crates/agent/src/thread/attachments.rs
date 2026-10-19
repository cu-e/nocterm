//! Composer editing owns future context separately from active grants and saved defaults.
use super::{Attachment, composer::AttachmentSelection};
use gpui_kit::App;
use nocterm_ai::history::live::{
    LiveAttachmentRef, LocalAttachmentFact,
    inputs::{LocalItemKey, SavedAttachmentRef},
};
use nocterm_workspace::TerminalEntry;

impl Attachment {
    /// The borrowed identity the Live document stores; a terminal is its item key only.
    pub(super) fn live_ref(&self) -> LiveAttachmentRef<'_> {
        match self {
            Self::Terminal(id) => {
                LiveAttachmentRef::LocalItem(LocalItemKey::from_bits(id.as_u64()))
            }
            Self::Connection(id) => LiveAttachmentRef::Connection(id),
            Self::Group(id) => LiveAttachmentRef::Group(id),
            Self::UnavailableLocal(title) => LiveAttachmentRef::UnavailableLocal(title),
        }
    }
}

/// A stable attachment name copied out of the workspace for one decision.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum StableAttachment {
    Connection(String),
    Group(String),
    LocalTerminal(String),
}
impl StableAttachment {
    pub(super) fn as_ref(&self) -> SavedAttachmentRef<'_> {
        match self {
            Self::Connection(value) => SavedAttachmentRef::Connection(value),
            Self::Group(value) => SavedAttachmentRef::Group(value),
            Self::LocalTerminal(value) => SavedAttachmentRef::LocalTerminal(value),
        }
    }
}

/// The stable names of open terminals, keyed by their workspace item.
pub(super) struct LocalFacts(Vec<(LocalItemKey, StableAttachment)>);
impl LocalFacts {
    pub(super) fn as_facts(&self) -> Vec<LocalAttachmentFact<'_>> {
        self.0
            .iter()
            .map(|(key, stable)| LocalAttachmentFact {
                key: *key,
                saved: stable.as_ref(),
            })
            .collect()
    }
    /// Names every open terminal that has reported its state.
    pub(super) fn read(terminals: &[TerminalEntry], cx: &App) -> Self {
        Self(
            terminals
                .iter()
                .filter_map(|entry| {
                    let info = entry.capabilities.state().info(cx)?;
                    let stable = match info.profile {
                        Some(id) => StableAttachment::Connection(id.to_string()),
                        None => StableAttachment::LocalTerminal(info.title.to_string()),
                    };
                    Some((LocalItemKey::from_bits(entry.item.as_u64()), stable))
                })
                .collect(),
        )
    }
    fn find(&self, key: LocalItemKey) -> Option<&StableAttachment> {
        self.0
            .iter()
            .find(|(known, _)| *known == key)
            .map(|(_, stable)| stable)
    }
}

/// Stable, de-duplicated attachments ready to be saved with a queued prompt.
pub(super) struct StableAttachments(Vec<StableAttachment>);
impl StableAttachments {
    /// The selection with terminals named by `facts`, without duplicates.
    pub(super) fn select(selection: AttachmentSelection<'_>, facts: &LocalFacts) -> Self {
        use super::composer::AttachmentRef;
        let mut stable = Vec::new();
        for attachment in selection.iter() {
            let value = match attachment {
                AttachmentRef::Connection(id) => Some(StableAttachment::Connection(id.to_string())),
                AttachmentRef::Group(id) => Some(StableAttachment::Group(id.to_string())),
                AttachmentRef::UnavailableLocal(title) => {
                    Some(StableAttachment::LocalTerminal(title.to_string()))
                }
                AttachmentRef::Terminal(key) => facts.find(key).cloned(),
            };
            if let Some(value) = value
                && !stable.contains(&value)
            {
                stable.push(value);
            }
        }
        Self(stable)
    }
    pub(super) fn refs(&self) -> Vec<SavedAttachmentRef<'_>> {
        self.0.iter().map(StableAttachment::as_ref).collect()
    }
}
