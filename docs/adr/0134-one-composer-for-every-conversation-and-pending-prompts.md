# 0134. One composer for every conversation, and pending prompts in the transcript

Status: accepted. Extends the conversation port of
[0133](0133-cloud-chats-in-the-history-column-and-seats-woken-for-prompts.md).

## Context

An open Cloud chat drew its transcript with the Local renderer but its footer through a
separate, minimal composer: its own footer row, its own send-button rules and no status,
queue or command slots. It looked unlike a Local chat. A prompt the person sent appeared
only after the runner accepted it (`PromptAccepted`), which can take as long as a managed
seat's boot; until then nothing in the transcript showed that it was sent, and a refused or
unanswered prompt showed only an error line. The history column beside a Cloud chat did not
widen the panel as it does beside a Local chat.

## Decision

1. **Capabilities in the neutral view.** `ConversationView::composer` declares what its
   composer offers (`ComposerCapabilities`): `attach` (terminals, servers and images),
   `configure` (model, mode and session options) and `usage` (the usage ring), beside
   `can_prompt`, `can_stop` and `can_allow`. A Local chat declares all three; a Cloud chat
   none, since its runner owns terminals, model and usage and a Cloud prompt is text.
2. **One footer.** The agent panel draws the same footer for every source: approvals, status,
   queue, commands and the composer with its field, controls, Allow and send button. The
   frame read from the source decides each control; the send/stop rules are shared. What a
   Local thread owns (its status text, queue, sign-in and permission requests, attachments and
   options) is drawn from the thread when the source declares the capability. A composer that
   does not attach takes no image pick or file paste.
3. **Pending prompts.** `ConversationView::pending_prompts` lists the prompts a source has not
   accepted, oldest first, each with a presentation key, its text and a state: `Sending`,
   `Delivered`, `Unreachable` or `Refused`. The panel draws them after the transcript as the same
   user row, subdued with a live line while on their way, and with Retry (and Dismiss for a
   refused prompt) when failed. The accepted row replaces a pending one in the same place. The
   status line shows "Waiting for the agent" while a prompt is on its way and the run is idle.
4. **Retry and dismissal.** `ConversationActions::retry_prompt` sends an unanswered prompt again
   under its own ID, and a refused prompt's text as a new prompt that replaces it;
   `dismiss_prompt` forgets a refused prompt. While the composer still holds the prompt
   (`ConversationSubmission::pending_prompt` matches the row), Retry is the composer's own
   send, keeping the refused/retry semantics of 0133. A new prompt sent after a refusal
   dismisses the refused row. Local sources have no pending prompts.
5. **Refused prompts keep only text.** A refusal still retires its command and payload
   permit; `CloudChat` keeps at most four refused prompts' text under a bytes-only
   reservation of the history admission.

## Consequences

- A Cloud chat looks like a Local chat; controls a source does not support are absent, not
  disabled.
- A sent Cloud prompt is visible at once; a failed one stays visible until it is retried,
  replaced or dismissed, or until more than four refusals push it out.
- A new source declares its composer capabilities and pending prompts instead of adding a
  footer of its own.
