//! Admitted incoming data owns its quota through physical background row preparation.
use crate::{thread::PreparedRows, transcript_cost::CandidateCost};
use gpui::BackgroundExecutor;
use nocterm_cloud_conversation_model::{CloudConversationError, ConversationDelta, Freshness};
use nocterm_core::allocation::Allocated;

pub(crate) enum AdmittedUpdate {
    Transcript(Box<Allocated<(ConversationDelta, CandidateCost)>>),
    ComposerChanged,
    Freshness(Freshness),
    Refused(CloudConversationError),
}
impl AdmittedUpdate {
    /// One subscription awaits one physical preparation at a time. Dropping its waiter
    /// does not release the allocation while a running parser still owns its data.
    pub(crate) async fn prepare(self, executor: &BackgroundExecutor) -> PreparedUpdate {
        match self {
            Self::Transcript(input) => {
                let prepared = executor
                    .spawn(async move {
                        (*input).map(|(delta, cost)| {
                            let rows = PreparedRows::new(&delta);
                            PreparedTranscript { delta, cost, rows }
                        })
                    })
                    .await;
                PreparedUpdate::Transcript(Box::new(prepared))
            }
            Self::ComposerChanged => PreparedUpdate::ComposerChanged,
            Self::Freshness(value) => PreparedUpdate::Freshness(value),
            Self::Refused(error) => PreparedUpdate::Refused(error),
        }
    }
}

pub(crate) enum PreparedUpdate {
    Transcript(Box<Allocated<PreparedTranscript>>),
    ComposerChanged,
    Freshness(Freshness),
    Refused(CloudConversationError),
}
/// Plain completion data, consumed atomically by the original subscription generation.
pub(crate) struct PreparedTranscript {
    delta: ConversationDelta,
    cost: CandidateCost,
    rows: PreparedRows,
}
impl PreparedTranscript {
    pub(crate) fn into_parts(self) -> (ConversationDelta, CandidateCost, PreparedRows) {
        (self.delta, self.cost, self.rows)
    }
}
