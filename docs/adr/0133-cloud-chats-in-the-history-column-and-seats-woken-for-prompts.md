# 0133. Cloud chats are listed in the history column, and a prompt wakes its seat

Status: accepted. Amends the discovery and wake rules of
[0122](0122-cloud-chat-creation-protocol.md) (§8) and the menu of
[0130](0130-cloud-agents-only-in-hub-windows.md).

## Context

Earlier Cloud chats of a hub window were a "History" block inside the Cloud tab of the
new-chat popover, labelled only with their host and the start of the chat identifier. The
history column beside a chat, where local chats are searched and opened, did not list them.

A managed seat sleeps after its idle time while a window keeps its chat open. Only opening a
chat woke the seat (`start_instance_for_chat`). The hub stores a command for a sleeping seat,
but only a running runner admits it and answers head probes. A prompt therefore stayed
pending forever, every receipt query of a retry and every replay poll went unanswered, and the
replay of the chat stopped for good at the first unanswered poll. The composer reported the
generic "cannot accept that action" for transport failures and for refusals alike, and a
refused prompt kept the composer bound to a retry that could only be refused again.

## Decision

1. **History column.** `ChatHistoryEntry` describes its row with a `ChatSummary` (stable
   key, title, host, model, optional time). The history column of the agent panel lists the
   window hub's Cloud chats after the local chats through the same row renderer, matches its
   search against what the row shows, and opens a row through its entry. A Cloud row offers
   no rename, pin, fork or delete: the port supports none. The column lists the chats when
   it is shown, when the window's hub becomes shown and after a chat is created or opened;
   the popover's Cloud tab keeps only new chats and agent rows. The catalog of a runner
   carries no title or time, so the adapter names a row after its host and identifier and
   gives no time.
2. **A prompt wakes its seat.** The adapter wraps the source of a managed chat
   (`hub-io` `cloud_chats::seat_wake`): before it delivers or queries a command other than
   Stop it calls `start_instance_for_chat` for the chat, which is idempotent; a failed wake
   keeps the command staged for a retry. Replay never wakes a seat.
3. **Replay outlives an unanswered poll.** `CloudChat` retries a poll that was not answered
   with a growing delay (1 s to 30 s), reports `ConversationView::reconnecting` meanwhile,
   and polls again as soon as one of its commands reaches the hub.
4. **Honest outcomes.** `ConversationError` gains `Unreachable` (the delivery was not
   answered; sending again retries the same command) and `Rejected` (the runner refused it).
   `ConversationSubmission::refused` tells the panel that sending the same input again is a
   new action, not a retry.

## Consequences

- Sending a prompt to a chat whose seat sleeps starts the seat; the first reply can take as
  long as the runner's boot.
- Viewing a chat of a sleeping seat shows "Reconnecting" until a prompt wakes the seat or it
  is woken elsewhere.
- Cloud rows have no time until the hub or the account records one; ordering by time across
  local and Cloud chats is not possible yet.
