//! Exact native observations mapped to the sole public head codec.
use super::{RunnerHeadError, codec};
use crate::cloud::{metadata_crypto, runner_transport};
use nocterm_cloud_model::{
    authority::StreamBinding,
    identity::{ChatControlRevision, ChatKeyEpoch, CloudChatId, CommandId, Fingerprint},
    journal::{
        CurrentChatHead, CurrentCloudHeads, CurrentHeadsCursor, CurrentHeadsPage, HeadAnchor,
        HeadAnchorKind, JournalCatalogBinding, MAX_CURRENT_HEADS_PAGE,
        OriginalCommandReceiptObservation, OriginalCommandReceiptQuery,
    },
};
use nocterm_hub_model::cloud::{
    RecordKind,
    head::{
        CatalogHeadCursor, CatalogHeadPage, ChatHead, HeadProbeScope, HeadQuery, HeadRecordAnchor,
        HeadTarget, HeadValue, ReceiptOutcome,
    },
};
use std::sync::Arc;

pub(super) struct HeadBinding {
    binding: JournalCatalogBinding,
    scope: HeadProbeScope,
}
impl HeadBinding {
    pub(super) fn new(
        binding: JournalCatalogBinding,
        scope: HeadProbeScope,
    ) -> Result<Self, RunnerHeadError> {
        if scope.namespace != metadata_crypto::namespace(&binding)?
            || scope.runner != metadata_crypto::runner(&binding)
        {
            return Err(RunnerHeadError::Binding);
        }
        Ok(Self { binding, scope })
    }
    pub(super) fn scope(&self) -> &HeadProbeScope {
        &self.scope
    }
    pub(super) fn request(&self, query: &HeadQuery) -> Result<HeadRead, RunnerHeadError> {
        if query.scope != self.scope {
            return Err(RunnerHeadError::Binding);
        }
        Ok(match &query.target {
            HeadTarget::Catalog { after } => HeadRead::Catalog(
                after
                    .as_ref()
                    .map(|value| {
                        CurrentHeadsCursor::new(
                            self.binding.clone(),
                            value.revision,
                            Fingerprint::from_bytes(value.digest),
                            CloudChatId::from_bytes(value.after),
                            value.seen,
                        )
                    })
                    .transpose()?,
            ),
            HeadTarget::Chat {
                chat_id,
                chat_key_epoch,
            } => {
                ChatKeyEpoch::new(*chat_key_epoch).map_err(codec)?;
                HeadRead::Chat(self.binding.stream(CloudChatId::from_bytes(*chat_id)))
            }
            HeadTarget::CommandReceipt {
                chat_id,
                chat_key_epoch,
                command_id,
                semantic_digest,
                expected_control_revision,
            } => HeadRead::Receipt(OriginalCommandReceiptQuery {
                stream: self.binding.stream(CloudChatId::from_bytes(*chat_id)),
                key_epoch: ChatKeyEpoch::new(*chat_key_epoch).map_err(codec)?,
                command: CommandId::from_bytes(*command_id),
                digest: Fingerprint::from_bytes(*semantic_digest),
                expected_control: ChatControlRevision::from_u64(*expected_control_revision),
            }),
        })
    }
    pub(super) fn value(
        &self,
        query: &HeadQuery,
        value: &HeadReadValue,
    ) -> Result<HeadValue, RunnerHeadError> {
        if query.scope != self.scope {
            return Err(RunnerHeadError::Binding);
        }
        match (&query.target, value) {
            (HeadTarget::Catalog { after }, HeadReadValue::Catalog(page)) => {
                if page.binding() != &self.binding
                    || page.heads().len() > MAX_CURRENT_HEADS_PAGE
                    || page.after().map(cursor).as_ref() != after.as_ref()
                {
                    return Err(RunnerHeadError::Binding);
                }
                Ok(HeadValue::Catalog(Box::new(CatalogHeadPage {
                    revision: page.revision(),
                    digest: *page.root().as_bytes(),
                    total_chats: page.total(),
                    after: page.after().map(cursor),
                    chats: page
                        .heads()
                        .iter()
                        .map(|head| self.head(head))
                        .collect::<Result<_, _>>()?,
                    has_more: page.next_cursor().is_some(),
                })))
            }
            (
                HeadTarget::Chat {
                    chat_id,
                    chat_key_epoch,
                },
                HeadReadValue::Chat(Some(head)),
            ) => {
                if head.stream().chat().as_bytes() != chat_id
                    || head.key_epoch().get() != *chat_key_epoch
                {
                    return Err(RunnerHeadError::Binding);
                }
                Ok(HeadValue::Chat(Box::new(self.head(head)?)))
            }
            (HeadTarget::Chat { .. }, HeadReadValue::Chat(None)) => {
                Err(RunnerHeadError::MissingChat)
            }
            (HeadTarget::CommandReceipt { .. }, HeadReadValue::Receipt(receipt)) => {
                Ok(HeadValue::CommandReceipt(match receipt {
                    OriginalCommandReceiptObservation::NotFound => ReceiptOutcome::NotFound,
                    OriginalCommandReceiptObservation::Accepted(control) => {
                        ReceiptOutcome::Accepted {
                            control_revision: control.get(),
                        }
                    }
                    OriginalCommandReceiptObservation::Conflict => ReceiptOutcome::Conflict,
                    OriginalCommandReceiptObservation::Unknown => ReceiptOutcome::Unknown,
                }))
            }
            _ => Err(RunnerHeadError::Binding),
        }
    }
    fn head(&self, head: &CurrentChatHead) -> Result<ChatHead, RunnerHeadError> {
        if !self.binding.contains(head.stream()) {
            return Err(RunnerHeadError::Binding);
        }
        let facts = head.facts();
        Ok(ChatHead {
            chat_id: *head.stream().chat().as_bytes(),
            chat_key_epoch: head.key_epoch().get(),
            current_writer: facts.writer.map(|value| value.get()),
            control_revision: facts.control.get(),
            deleted: facts.deleted,
            head: facts
                .head
                .as_ref()
                .map(|anchor| anchor_record(head, anchor))
                .transpose()?,
            retained_from: facts.event_floor.get(),
            snapshot: facts
                .snapshot
                .as_ref()
                .map(|anchor| anchor_record(head, anchor))
                .transpose()?,
        })
    }
}
fn cursor(value: &CurrentHeadsCursor) -> CatalogHeadCursor {
    CatalogHeadCursor {
        revision: value.revision(),
        digest: *value.root().as_bytes(),
        after: *value.after().as_bytes(),
        seen: value.seen(),
    }
}
fn anchor_record(
    head: &CurrentChatHead,
    anchor: &HeadAnchor,
) -> Result<HeadRecordAnchor, RunnerHeadError> {
    let context = runner_transport::record_context(head.stream(), head.key_epoch(), anchor)?;
    Ok(HeadRecordAnchor {
        kind: match anchor.kind() {
            HeadAnchorKind::Event => RecordKind::Event,
            HeadAnchorKind::Snapshot => RecordKind::Snapshot,
        },
        position: context.position(),
        id: context.id(),
        run: context.run(),
        ciphertext_digest: *anchor.ciphertext().as_bytes(),
    })
}

pub(super) enum HeadRead {
    Catalog(Option<CurrentHeadsCursor>),
    Chat(StreamBinding),
    Receipt(OriginalCommandReceiptQuery),
}
pub(super) enum HeadReadValue {
    Catalog(CurrentHeadsPage),
    Chat(Option<CurrentChatHead>),
    Receipt(OriginalCommandReceiptObservation),
}
impl HeadRead {
    pub(super) async fn load(
        self,
        reader: Arc<dyn CurrentCloudHeads>,
    ) -> Result<HeadReadValue, RunnerHeadError> {
        Ok(match self {
            Self::Catalog(after) => {
                HeadReadValue::Catalog(reader.catalog(after, MAX_CURRENT_HEADS_PAGE as u16).await?)
            }
            Self::Chat(stream) => HeadReadValue::Chat(reader.chat(stream).await?),
            Self::Receipt(query) => HeadReadValue::Receipt(reader.receipt(query).await?),
        })
    }
}
