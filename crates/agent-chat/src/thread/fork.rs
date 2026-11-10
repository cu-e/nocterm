//! A prepared fork owns its copied durable body until the destination admits it.
use super::{Attachment, Composer, Restore, config::ChatConfig, document::ChatDocument};
/// What a forked chat starts from.
pub struct Fork {
    chat: nocterm_ai::history::SavedChat,
    restore: Option<Restore>,
    attachments: Vec<Attachment>,
    _allocation: nocterm_core::allocation::AllocationLease,
}

impl Fork {
    pub(crate) fn prepared(mut self) -> Result<Self, nocterm_ai::history::HistoryCodecError> {
        self.chat = nocterm_ai::history::prepare_owned(self.chat)?;
        Ok(self)
    }
    pub(crate) fn key(&self) -> nocterm_ai::execution_origin::ChatKey {
        nocterm_ai::execution_origin::ChatKey::from_header(&self.chat.header)
    }
}

impl Fork {
    pub(super) fn restoration(
        session: &Option<nocterm_ai::model::ids::SessionId>,
        workdir: &Option<std::path::PathBuf>,
        restore: Option<&Restore>,
    ) -> Option<Restore> {
        match (session, restore) {
            (Some(session), _) => workdir.clone().map(|workdir| Restore {
                session: session.clone(),
                workdir,
                fork: true,
            }),
            (None, Some(restore)) => Some(Restore {
                fork: true,
                ..restore.clone()
            }),
            (None, None) => None,
        }
    }
    pub(super) fn capture(
        mut chat: nocterm_ai::history::SavedChat,
        source: ForkSource<'_>,
        restore: Option<Restore>,
        len: usize,
        allocation: nocterm_core::allocation::AllocationLease,
    ) -> Self {
        let whole = len >= source.document.entries().len();
        let end = len.min(source.document.entries().len());
        chat.entries = source.document.entries()[..end].to_vec();
        chat.times = source
            .document
            .transcript()
            .times
            .iter()
            .take(end)
            .copied()
            .collect();
        chat.header.title = source.document.transcript().title.clone();
        chat.header.name = Some(format!("{} (fork)", source.title));
        chat.model = source.config.model();
        chat.config = source.config.choices().clone();
        chat.mode = source.config.mode_choice().map(ToString::to_string);
        chat.pending_history = source.document.pending_history() || !whole;
        Self {
            chat,
            restore: restore.filter(|_| whole),
            attachments: source.composer.default_attachments().to_vec(),
            _allocation: allocation,
        }
    }
    pub(super) fn into_parts(
        self,
    ) -> (
        nocterm_ai::history::SavedChat,
        Option<Restore>,
        Vec<Attachment>,
    ) {
        (self.chat, self.restore, self.attachments)
    }
}

pub(super) struct ForkSource<'a> {
    document: &'a ChatDocument,
    composer: &'a Composer,
    config: &'a ChatConfig,
    title: String,
}
impl<'a> ForkSource<'a> {
    pub(super) fn new(
        document: &'a ChatDocument,
        composer: &'a Composer,
        config: &'a ChatConfig,
        title: String,
    ) -> Self {
        Self {
            document,
            composer,
            config,
            title,
        }
    }
}
