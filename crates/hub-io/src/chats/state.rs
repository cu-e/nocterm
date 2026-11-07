//! What one open hub chat has shown so far, and what each new item or draft adds.
//!
//! The rules are pure: the watch task feeds items and drafts in the order the hub sends them
//! and performs the outputs. A prompt of this client resolves when the first run that started
//! after its message ends; a message from another device starts a turn of its own.

use std::collections::{HashMap, HashSet};

use futures::channel::oneshot;
use nocterm_ai::AgentError;
use nocterm_ai::model::content::Content;
use nocterm_ai::model::session::{PromptOutcome, SessionUpdate, StopReason, TurnActivity};
use nocterm_hub_proto::v2;

use super::items;

/// What the watch task does after an item or draft.
pub(super) enum Output {
    Notice(Box<SessionUpdate>),
    /// Ask the person whether the tool call `call` may run.
    Permission {
        approval: String,
        call: String,
        title: String,
    },
    /// The prompt of this client ended.
    Resolve(
        oneshot::Sender<Result<PromptOutcome, AgentError>>,
        Result<PromptOutcome, AgentError>,
    ),
}

impl Output {
    pub(super) fn notice(update: SessionUpdate) -> Self {
        Self::Notice(Box::new(update))
    }
}

/// A prompt this client sent and waits the end of.
struct Pending {
    client_message: String,
    /// The seq of its message, once the hub listed it.
    after: Option<u64>,
    /// Runs that were already running when the message arrived: their end is not its end.
    earlier: HashSet<String>,
    done: oneshot::Sender<Result<PromptOutcome, AgentError>>,
}

#[derive(Default)]
pub(super) struct ChatState {
    last_seq: u64,
    sent: HashSet<String>,
    pending: Option<Pending>,
    /// Runs with items and no end yet.
    active: HashSet<String>,
    /// Text of each run's draft already shown, and how many parts it holds.
    drafts: HashMap<String, (String, u32)>,
    /// Titles of tool calls, for the approval questions about them.
    calls: HashMap<String, String>,
    /// Approvals asked for and not resolved yet, with their call.
    approvals: HashMap<String, String>,
}

impl ChatState {
    pub(super) fn last_seq(&self) -> u64 {
        self.last_seq
    }

    /// A new chat: what it holds up to `seq` is not shown.
    pub(super) fn skip_to(&mut self, seq: u64) {
        self.last_seq = seq;
    }

    /// Whether a run is under way: the chat shows that the agent works.
    pub(super) fn running(&self) -> bool {
        !self.active.is_empty()
    }

    /// A prompt this client is about to send under `client_message`.
    pub(super) fn expect(
        &mut self,
        client_message: String,
        done: oneshot::Sender<Result<PromptOutcome, AgentError>>,
    ) {
        self.sent.insert(client_message.clone());
        self.pending = Some(Pending {
            client_message,
            after: None,
            earlier: HashSet::new(),
            done,
        });
    }

    /// The prompt could not be sent; it waits for nothing.
    pub(super) fn abandon(&mut self) -> Option<oneshot::Sender<Result<PromptOutcome, AgentError>>> {
        self.pending.take().map(|pending| pending.done)
    }

    /// Questions still open, asked again when a chat reopens.
    pub(super) fn open_approvals(&self) -> Vec<Output> {
        self.approvals
            .iter()
            .map(|(approval, call)| Output::Permission {
                approval: approval.clone(),
                call: call.clone(),
                title: self.calls.get(call).cloned().unwrap_or_default(),
            })
            .collect()
    }

    /// Replays a page of history: what the chat shows, without questions or turns.
    pub(super) fn replay(&mut self, item: &v2::ChatItem) -> Vec<SessionUpdate> {
        self.apply(item, true)
            .into_iter()
            .filter_map(|output| match output {
                Output::Notice(update) => Some(*update),
                _ => None,
            })
            .collect()
    }

    /// One live item of the chat.
    pub(super) fn item(&mut self, item: &v2::ChatItem) -> Vec<Output> {
        if item.seq <= self.last_seq {
            return Vec::new();
        }
        self.apply(item, false)
    }

    fn apply(&mut self, item: &v2::ChatItem, replay: bool) -> Vec<Output> {
        use v2::chat_item::Body;
        self.last_seq = self.last_seq.max(item.seq);
        if !item.run_id.is_empty() && !matches!(item.body, Some(Body::RunEnded(_))) {
            self.active.insert(item.run_id.clone());
        }
        let Some(body) = &item.body else {
            return Vec::new();
        };
        match body {
            Body::User(message) => self.user(item.seq, message, replay),
            Body::Agent(text) => self.agent_text(&item.run_id, &text.text),
            Body::ToolCall(call) => {
                self.calls.insert(call.call_id.clone(), call.title.clone());
                vec![Output::notice(items::tool_call(call))]
            }
            Body::ToolResult(result) => vec![Output::notice(items::tool_result(result))],
            Body::ApprovalRequest(request) => {
                self.approvals
                    .insert(request.approval_id.clone(), request.call_id.clone());
                if replay {
                    return Vec::new();
                }
                vec![Output::Permission {
                    approval: request.approval_id.clone(),
                    call: request.call_id.clone(),
                    title: self
                        .calls
                        .get(&request.call_id)
                        .cloned()
                        .unwrap_or_default(),
                }]
            }
            Body::ApprovalResolution(resolution) => {
                self.approvals.remove(&resolution.approval_id);
                Vec::new()
            }
            Body::RunEnded(ended) => self.run_ended(item.seq, &item.run_id, ended, replay),
            Body::Change(_) => Vec::new(),
        }
    }

    fn user(&mut self, seq: u64, message: &v2::UserMessage, replay: bool) -> Vec<Output> {
        let prompt = items::prompt(message);
        if replay {
            return prompt
                .into_iter()
                .map(|part| Output::notice(SessionUpdate::UserMessageChunk(part)))
                .collect();
        }
        if let Some(pending) = self
            .pending
            .as_mut()
            .filter(|pending| pending.client_message == message.client_message_id)
        {
            pending.after = Some(seq);
            pending.earlier = self.active.clone();
            return Vec::new();
        }
        if self.sent.contains(&message.client_message_id) {
            return Vec::new();
        }
        vec![Output::notice(SessionUpdate::Turn(TurnActivity::Started {
            prompt,
        }))]
    }

    fn agent_text(&mut self, run: &str, text: &str) -> Vec<Output> {
        let shown = self
            .drafts
            .remove(run)
            .map(|(shown, _)| shown)
            .unwrap_or_default();
        let rest = text.strip_prefix(shown.as_str()).unwrap_or(text);
        if rest.is_empty() {
            return Vec::new();
        }
        vec![Output::notice(SessionUpdate::AgentMessageChunk(
            Content::text(rest.to_owned()),
        ))]
    }

    /// Text the agent of run `draft.run_id` is writing; only the part not shown yet is told.
    pub(super) fn draft(&mut self, draft: &v2::Draft) -> Vec<Output> {
        let (shown, parts) = self.drafts.entry(draft.run_id.clone()).or_default();
        let whole = if draft.snapshot {
            draft.text.clone()
        } else if draft.part == *parts + 1 {
            format!("{shown}{}", draft.text)
        } else {
            return Vec::new();
        };
        *parts = draft.part;
        let Some(rest) = whole.strip_prefix(shown.as_str()) else {
            return Vec::new();
        };
        let rest = rest.to_owned();
        *shown = whole;
        if rest.is_empty() {
            return Vec::new();
        }
        vec![Output::notice(SessionUpdate::AgentMessageChunk(
            Content::text(rest),
        ))]
    }

    fn run_ended(
        &mut self,
        seq: u64,
        run: &str,
        ended: &v2::RunEnded,
        replay: bool,
    ) -> Vec<Output> {
        let status = v2::RunStatus::try_from(ended.status).unwrap_or(v2::RunStatus::Completed);
        self.drafts.remove(run);
        if status == v2::RunStatus::Interrupted {
            // The run goes on as a new run; nothing ended for the person.
            self.active.remove(run);
            return Vec::new();
        }
        self.active.remove(run);
        self.approvals.clear();
        let mut outputs: Vec<Output> = items::run_error(ended)
            .map(Output::notice)
            .into_iter()
            .collect();
        if replay {
            return outputs;
        }
        let answers_pending = self.pending.as_ref().is_some_and(|pending| {
            pending.after.is_some_and(|after| seq > after) && !pending.earlier.contains(run)
        });
        if answers_pending && let Some(pending) = self.pending.take() {
            let stop_reason = match status {
                v2::RunStatus::Stopped => StopReason::Cancelled,
                v2::RunStatus::Limit => StopReason::MaxTurnRequests,
                _ => StopReason::EndTurn,
            };
            let outcome = PromptOutcome {
                stop_reason,
                usage: items::usage(ended),
            };
            outputs.push(Output::Resolve(pending.done, Ok(outcome)));
        } else {
            outputs.push(Output::notice(SessionUpdate::Turn(TurnActivity::Ended)));
        }
        outputs
    }
}

#[cfg(test)]
mod tests;
