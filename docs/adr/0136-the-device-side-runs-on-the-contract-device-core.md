# 0136. The device side of Cloud chats runs on the contract's device core; hub-io is a thin adapter

Status: accepted. Supersedes the parts of earlier decisions that place device-side Cloud chat
logic in this workspace: the `nocterm-cloud-model::projection` placement and its replay,
outbox and cache ports named in [0129](0129-the-device-allows-a-queued-run-to-use-the-model.md),
[0133](0133-cloud-chats-in-the-history-column-and-seats-woken-for-prompts.md) and
[0134](0134-one-composer-for-every-conversation-and-pending-prompts.md); in
[0118](0118-account-encrypted-cloud-hub-bootstrap.md), the rule that history freshness
requires the runner's challenge response before anything is shown, and the work leases that
precede SDK/decode/crypto allocations, for device flows; and the history row title rule of
[0133](0133-cloud-chats-in-the-history-column-and-seats-woken-for-prompts.md) ("host · id").
Implements the desktop side of contract ADR 0007 (device flows, resumable reader, freshness)
and the listing half of contract ADR 0008 (runner chat directory).

## Context

The device side of Cloud chats existed three times: the desktop's `hub-io` decoded runner
events to the runner's own `CoordinatorEvent`s and `cloud-model::projection` reduced them, the
desktop kept its own codecs of the account's private records, head probes and command sealing,
and the contract's sans-I/O `nocterm-cloud-device` did all of it again for the web panel. The
desktop read an open chat by probing the runner and sleeping one second in a loop, never used
Watch, listed chats as "host · id" rows without time, and could not read a chat whose history
was compacted (a replay gap), that had an SSH session, or whose writer epoch changed.

## Decision

1. **The core decides; hub-io performs.** `nocterm-hub-io::cloud_device` runs the core's flows
   (`Discover`, `ListChats`, `OpenChat`, `Deliver`, `CreateChat`, `ProviderLease`): it answers
   each `Effect` with the hub's Store, Cloud and agent calls (`effects`), signs with
   `HubClient::sign_cloud`, sleeps until `Step::Idle { until_ms }` and delivers replay and
   Watch frames in arrival order (`driver`). Verification, the transcript, freshness, command
   building, seat wake before a command (ADR 0133 §2) and the order of the steps are the
   core's. hub-io stays the only crate that sees the contract (ADR 0096).
2. **No polling.** An open chat probes on open, on Watch reconnect, after a receipt of this
   device's own command and on Refresh; Watch hints replay only what is new.
3. **Freshness is shown as the core reports it.** `Current` after a fresh witness reached;
   `Unconfirmed` when the runner did not answer (a sleeping seat): the panel says "Seat asleep
   — stored history, may be incomplete" and offers Wake (`StartInstance` for the chat, then a
   refresh); `Withheld` when the hub withholds history the runner witnessed. The first
   freshness of a subscription, before any round, is not shown. Commands are allowed in every
   state; the core names the expected control revision.
4. **A neutral conversation model.** The new domain crate `nocterm-cloud-conversation-model`
   owns the transcript a window holds (`CloudTranscript`, rows, runs, approvals, pending
   commands), `Freshness`, `ConversationAction`, the `CloudConversation` port with its
   `ConversationUpdate`s and typed `CloudConversationError`, and a `test-support` fake. hub-io
   maps the core's display values to it (`mapping`) and implements the port
   (`DeviceConversation`, one task per open chat, ended when nothing subscribes and no
   command is on its way, or when the account ends). `nocterm-cloud-chat` keeps only the GPUI
   entity over the deltas, the agent-chat `ConversationSource`, the history admission and the
   ADR 0134 pending-prompt presentation. `nocterm-cloud-model` is the runner's domain only.
5. **Sealed checkpoints and an outbox.** The core's `VerifiedChat` and `VerifiedDirectory`
   checkpoints (sealed with a key derived from the account key) are kept as private files
   under the chats root (`device-v1/<account digest>/`), so a reopened chat shows its verified
   prefix at once and replays only what is new; the earlier `verified-page.v1` caches are
   removed at start. Each command's exact publication is kept before it is first sent and
   removed once it settled; publications left from an earlier run are published again with
   the same bytes until the hub confirms them or their lifetime ends.
6. **The history lists summaries.** `ListChats` reads each runner's directory from its
   checkpoint and confirms it with the runner's catalog. A row shows the summary's title, last
   activity, state, model and bound servers, says "as of" when the runner did not confirm it,
   and sorts with the local chats by last activity. A runner without summaries shows
   "Chat · <id>". Opening a row does not wake a seat.
7. **Account records use the contract's layouts.** The hub-trust and runner-registry objects
   that runner enrolment writes are encoded and decoded by `HubTrustRecord` and
   `RunnerRegistry`; the desktop keeps only the conversion. Chat material and grants are
   written by `CreateChat` alone.
8. **Key custody.** A flow holds a copy of the unlocked account keys (`AccountKeys`, zeroed on
   drop). Every owner of flows watches the account's key notices and drops its flows on lock,
   sign-out or replacement login.
9. **A temporary request signer.** The pinned contract's hub client offers no device-signed
   raw `CloudService` client for core-built requests (`ProbeHead`, `PublishOne`, `Replay`,
   `Watch`, `RunnerStatus`). Until it does, `cloud_device::rpc` signs the request proof with
   the public `nocterm_hub_model::pop::request_message` layout and the device key from the
   adapter's own credential file; `Capabilities` is a public call and goes unsigned.

## Consequences

- One reducer and one verifier serve the desktop and the web; compacted chats, chats with SSH
  sessions and writer-epoch changes are read like any other.
- A pending command is device-local: after a restart its row is gone, and the runner's
  accepted event shows the prompt. An unanswered prompt is retried as a new prompt.
- Device flows reserve no per-module work leases; the core bounds what a flow holds.
- The request signer in hub-io duplicates the hub client's request proof until the contract
  exposes a transport for core-built requests; it is to be removed then.
