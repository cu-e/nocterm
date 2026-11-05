//! The prompts the person sent that the runner has not accepted, as the chat shows them
//! (ADR 0134), and the waiters of submitted commands.
//!
//! A prompt shows at once, before the conversation has tracked it; then it follows its
//! command's state in the transcript. A refused or unanswered prompt stays until it is sent
//! again (as a new prompt that replaces it) or dismissed; at most four refused ones are shown.
use futures::channel::oneshot;
use nocterm_ai::conversation::{PendingPrompt, PendingPromptKey, PendingPromptState};
use nocterm_cloud_conversation_model::{
    CloudTranscript, CommandKey, CommandKind, CommandState, ConversationAction,
};
use std::collections::BTreeSet;

/// At most this many refused prompts stay shown; older ones are forgotten.
const MAX_REFUSED_PROMPTS: usize = 4;
/// Dismissed commands remembered; the conversation tracks at most 32 commands.
const MAX_DISMISSED: usize = 64;
/// Keys of prompts not tracked yet start with this byte, which no command id is made of on
/// purpose; a collision only affects which row a click names.
const PLACEHOLDER: u8 = 0xff;

/// How a command a waiter waits for ended badly.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum CommandFailure {
    Rejected,
    Unreachable,
    Stale,
}
pub(crate) type CommandWaiter = oneshot::Sender<Result<(), CommandFailure>>;

pub(crate) fn prompt_key(command: CommandKey) -> PendingPromptKey {
    PendingPromptKey::from_bytes(*command.as_bytes())
}
pub(crate) fn command_key(prompt: PendingPromptKey) -> CommandKey {
    CommandKey::from_bytes(*prompt.as_bytes())
}

/// What a command's settled state means for its waiter; `None` while it is on its way.
pub(crate) fn outcome(state: CommandState) -> Option<Result<(), CommandFailure>> {
    match state {
        CommandState::Sending => None,
        CommandState::Delivered | CommandState::Accepted => Some(Ok(())),
        CommandState::Refused => Some(Err(CommandFailure::Rejected)),
        CommandState::Unreachable => Some(Err(CommandFailure::Unreachable)),
    }
}

struct PromptBody {
    command: CommandKey,
    action: ConversationAction,
    observed: bool,
}
#[derive(Default)]
pub(crate) struct PendingPrompts {
    /// Prompts handed to the conversation that it has not tracked yet.
    placeholders: Vec<(PendingPromptKey, ConversationAction, Option<CommandKey>)>,
    bodies: Vec<PromptBody>,
    dismissed: BTreeSet<CommandKey>,
    waiters: Vec<(CommandKey, CommandWaiter)>,
    next: u64,
}
impl PendingPrompts {
    /// Shows `text` at once; returns the key of its placeholder row.
    pub(crate) fn sending(
        &mut self,
        action: &ConversationAction,
        replaces: Option<CommandKey>,
    ) -> PendingPromptKey {
        self.next = self.next.wrapping_add(1);
        let mut bytes = [PLACEHOLDER; 16];
        bytes[8..].copy_from_slice(&self.next.to_be_bytes());
        let key = PendingPromptKey::from_bytes(bytes);
        if let Some(command) = replaces {
            self.bodies.retain(|body| body.command != command);
        }
        self.placeholders.push((key, action.clone(), replaces));
        key
    }
    /// The conversation took (or refused) the placeholder's prompt.
    pub(crate) fn tracked(&mut self, placeholder: PendingPromptKey, command: Option<CommandKey>) {
        if let Some(index) = self
            .placeholders
            .iter()
            .position(|(key, _, _)| *key == placeholder)
        {
            let (_, action, replaces) = self.placeholders.remove(index);
            if let Some(command) = command {
                self.bodies.push(PromptBody {
                    command,
                    action,
                    observed: false,
                });
                if let Some(replaced) = replaces {
                    self.dismissed.insert(replaced);
                }
                while self.dismissed.len() > MAX_DISMISSED {
                    self.dismissed.pop_first();
                }
            } else if let Some(replaced) = replaces {
                self.bodies.push(PromptBody {
                    command: replaced,
                    action,
                    observed: true,
                });
            }
        }
    }
    pub(crate) fn body(&self, command: CommandKey) -> Option<ConversationAction> {
        self.bodies
            .iter()
            .find(|body| body.command == command)
            .map(|body| body.action.clone())
    }
    pub(crate) fn bytes(&self) -> usize {
        self.placeholders
            .iter()
            .map(|(_, action, _)| prompt_bytes(action))
            .chain(self.bodies.iter().map(|body| prompt_bytes(&body.action)))
            .sum()
    }
    pub(crate) fn replacement_bytes(&self, replaces: Option<CommandKey>) -> usize {
        replaces
            .and_then(|command| self.bodies.iter().find(|body| body.command == command))
            .map_or(0, |body| prompt_bytes(&body.action))
    }
    pub(crate) fn admits(&self, action: &ConversationAction, replaces: Option<CommandKey>) -> bool {
        let removed = usize::from(replaces.is_some());
        self.placeholders.len() + self.bodies.len() < 32 + removed
            && self
                .bytes()
                .saturating_sub(self.replacement_bytes(replaces))
                .saturating_add(prompt_bytes(action))
                <= 32 * 1024 * 1024
    }
    pub(crate) fn holds_retry_bodies(&self) -> bool {
        !self.bodies.is_empty()
    }
    /// Waits for `command` to settle; settles at once when `transcript` already says how.
    pub(crate) fn wait(
        &mut self,
        command: CommandKey,
        waiter: CommandWaiter,
        transcript: &CloudTranscript,
        reading: bool,
    ) {
        match transcript
            .pending_command(command)
            .and_then(|known| outcome(known.state))
        {
            Some(result) => {
                let _ = waiter.send(result);
            }
            None if reading => self.waiters.push((command, waiter)),
            None => {
                let _ = waiter.send(Err(CommandFailure::Unreachable));
            }
        }
    }
    /// Settles the waiters whose commands settled in `transcript`.
    pub(crate) fn follow(&mut self, transcript: &CloudTranscript) {
        let shown: BTreeSet<_> = self
            .rows(transcript)
            .iter()
            .map(|row| command_key(row.key))
            .collect();
        self.bodies.retain_mut(|body| {
            if transcript.pending_command(body.command).is_some() {
                body.observed = true;
                return shown.contains(&body.command);
            }
            !body.observed
                && !transcript
                    .runs()
                    .iter()
                    .any(|run| run.prompt_command == body.command)
        });
        let mut index = 0;
        while index < self.waiters.len() {
            let command = self.waiters[index].0;
            match transcript
                .pending_command(command)
                .and_then(|known| outcome(known.state))
            {
                Some(result) => {
                    let (_, waiter) = self.waiters.remove(index);
                    let _ = waiter.send(result);
                }
                None => index += 1,
            }
        }
    }
    /// Forgets a refused or unanswered prompt; whether there was one to forget.
    pub(crate) fn dismiss(&mut self, command: CommandKey, transcript: &CloudTranscript) -> bool {
        let failed = transcript.pending_command(command).is_some_and(|known| {
            known.kind == CommandKind::Prompt
                && matches!(
                    known.state,
                    CommandState::Refused | CommandState::Unreachable
                )
        });
        if !failed || !self.dismissed.insert(command) {
            return false;
        }
        self.bodies.retain(|body| body.command != command);
        while self.dismissed.len() > MAX_DISMISSED {
            self.dismissed.pop_first();
        }
        true
    }
    /// The rows: tracked prompts in send order, then those on their way to the conversation.
    pub(crate) fn rows<'a>(&'a self, transcript: &'a CloudTranscript) -> Vec<PendingPrompt<'a>> {
        let tracked: Vec<_> = transcript
            .pending()
            .iter()
            .filter(|known| {
                known.kind == CommandKind::Prompt
                    && known.state != CommandState::Accepted
                    && !self.dismissed.contains(&known.command)
                    && !self
                        .placeholders
                        .iter()
                        .any(|(_, _, replaces)| *replaces == Some(known.command))
            })
            .collect();
        let refused = tracked
            .iter()
            .filter(|known| known.state == CommandState::Refused)
            .count();
        let mut skip = refused.saturating_sub(MAX_REFUSED_PROMPTS);
        let mut rows = Vec::with_capacity(tracked.len() + self.placeholders.len());
        for known in tracked {
            if known.state == CommandState::Refused && skip > 0 {
                skip -= 1;
                continue;
            }
            rows.push(PendingPrompt {
                key: prompt_key(known.command),
                text: prompt_label(
                    &known.text,
                    self.bodies
                        .iter()
                        .find(|body| body.command == known.command)
                        .map(|body| &body.action),
                ),
                state: match known.state {
                    CommandState::Sending => PendingPromptState::Sending,
                    CommandState::Delivered | CommandState::Accepted => {
                        PendingPromptState::Delivered
                    }
                    CommandState::Unreachable => PendingPromptState::Unreachable,
                    CommandState::Refused => PendingPromptState::Refused,
                },
            });
        }
        rows.extend(
            self.placeholders
                .iter()
                .map(|(key, action, _)| PendingPrompt {
                    key: *key,
                    text: prompt_label(action.prompt_text().unwrap_or_default(), Some(action)),
                    state: PendingPromptState::Sending,
                }),
        );
        rows
    }
    /// Whether a command is still on its way: nothing may evict the chat meanwhile.
    pub(crate) fn busy(&self) -> bool {
        !self.placeholders.is_empty() || !self.waiters.is_empty()
    }
    /// Reading stopped without an execution answer. Kept command states stay unchanged:
    /// an already-published command may still be accepted and read after reopening.
    pub(crate) fn reading_ended(&mut self) {
        for (_, waiter) in self.waiters.drain(..) {
            let _ = waiter.send(Err(CommandFailure::Unreachable));
        }
    }
    /// The conversation ended: every waiter learns it.
    pub(crate) fn invalidate(&mut self) {
        self.placeholders.clear();
        self.bodies.clear();
        for (_, waiter) in self.waiters.drain(..) {
            let _ = waiter.send(Err(CommandFailure::Stale));
        }
    }
}

/// The original prompt bodies retained for failed retries, bounded before admission.
pub(crate) fn prompt_bytes(action: &ConversationAction) -> usize {
    match action {
        ConversationAction::Prompt {
            text,
            images,
            model,
            ..
        } => text
            .len()
            .saturating_add(images.iter().map(|image| image.bytes.len()).sum::<usize>())
            .saturating_add(
                model
                    .as_ref()
                    .map_or(0, |model| model.provider.len() + model.model.len()),
            ),
        _ => 0,
    }
}

/// Display only: the original command body remains byte-identical and can contain no text.
fn prompt_label<'a>(text: &'a str, action: Option<&ConversationAction>) -> &'a str {
    if !text.trim().is_empty() {
        return text;
    }
    match action {
        Some(ConversationAction::Prompt { images, .. }) if images.len() == 1 => "Image attachment",
        Some(ConversationAction::Prompt { images, .. }) if !images.is_empty() => {
            "Image attachments"
        }
        None if text.is_empty() => "Image attachments",
        _ => text,
    }
}
