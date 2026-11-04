//! The prompts the person sent that the runner has not accepted, as the chat shows them
//! (ADR 0134), and the waiters of submitted commands.
//!
//! A prompt shows at once, before the conversation has tracked it; then it follows its
//! command's state in the transcript. A refused or unanswered prompt stays until it is sent
//! again (as a new prompt that replaces it) or dismissed; at most four refused ones are shown.
use futures::channel::oneshot;
use nocterm_ai::conversation::{PendingPrompt, PendingPromptKey, PendingPromptState};
use nocterm_cloud_conversation_model::{CloudTranscript, CommandKey, CommandKind, CommandState};
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

#[derive(Default)]
pub(crate) struct PendingPrompts {
    /// Prompts handed to the conversation that it has not tracked yet.
    placeholders: Vec<(PendingPromptKey, String)>,
    dismissed: BTreeSet<CommandKey>,
    waiters: Vec<(CommandKey, CommandWaiter)>,
    next: u64,
}
impl PendingPrompts {
    /// Shows `text` at once; returns the key of its placeholder row.
    pub(crate) fn sending(&mut self, text: &str) -> PendingPromptKey {
        self.next = self.next.wrapping_add(1);
        let mut bytes = [PLACEHOLDER; 16];
        bytes[8..].copy_from_slice(&self.next.to_be_bytes());
        let key = PendingPromptKey::from_bytes(bytes);
        self.placeholders.push((key, text.to_owned()));
        key
    }
    /// The conversation took (or refused) the placeholder's prompt.
    pub(crate) fn tracked(&mut self, placeholder: PendingPromptKey) {
        self.placeholders.retain(|(key, _)| *key != placeholder);
    }
    /// Waits for `command` to settle; settles at once when `transcript` already says how.
    pub(crate) fn wait(
        &mut self,
        command: CommandKey,
        waiter: CommandWaiter,
        transcript: &CloudTranscript,
    ) {
        match transcript
            .pending_command(command)
            .and_then(|known| outcome(known.state))
        {
            Some(result) => {
                let _ = waiter.send(result);
            }
            None => self.waiters.push((command, waiter)),
        }
    }
    /// Settles the waiters whose commands settled in `transcript`.
    pub(crate) fn follow(&mut self, transcript: &CloudTranscript) {
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
                text: &known.text,
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
        rows.extend(self.placeholders.iter().map(|(key, text)| PendingPrompt {
            key: *key,
            text,
            state: PendingPromptState::Sending,
        }));
        rows
    }
    /// Whether a command is still on its way: nothing may evict the chat meanwhile.
    pub(crate) fn busy(&self) -> bool {
        !self.placeholders.is_empty() || !self.waiters.is_empty()
    }
    /// The conversation ended: every waiter learns it.
    pub(crate) fn invalidate(&mut self) {
        self.placeholders.clear();
        for (_, waiter) in self.waiters.drain(..) {
            let _ = waiter.send(Err(CommandFailure::Stale));
        }
    }
}
