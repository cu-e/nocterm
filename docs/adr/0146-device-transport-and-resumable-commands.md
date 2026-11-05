# 0146. Cloud effects go through the hub client; commands resume after a restart

Status: accepted. Supersedes, in [0144](0144-the-device-side-runs-on-the-contract-device-core.md),
the temporary request signer (decision 9), the outbox of exact publications (the outbox half of
decision 5), the rule that the first freshness of a subscription is not shown (decision 3) and
the consequences that a pending command's row is gone after a restart and that hub-io
duplicates the request proof. Implements the desktop side of contract ADR 0010 (device
transport and resumable commands).

## Context

ADR 0144 moved the device side of Cloud chats onto the contract's device core, but the pinned
contract had no device transport, no way to resume a command and no freshness for a chat
before its first round. hub-io therefore signed request proofs itself with the device key read
from its credential file, kept raw publications and published them again after a restart
without a receipt probe or a pending row, guessed a seat's pool and answered `NotFound` for an
agent without an enrolled seat, ignored the first freshness of each subscription and rebuilt
tool titles from the tool name. Contract ADR 0010 added all of it to the contract.

## Decision

1. **The hub client transports Cloud effects.** `cloud_device::effects` maps `Capabilities`,
   `Probe`, `Publish`, `Replay`, `Watch` and `RunnerStatus` one to one onto `HubClient`'s
   device calls, which refresh the token and sign under the client's own signer; a failed call
   answers its effect with `Input::from(CloudCallError)`. The device key is read only by
   `nocterm-hub-client`, and hub-io does not depend on `nocterm-hub-grpc`.
2. **Commands resume.** When a delivery first asks to publish, hub-io keeps the core's sealed
   `StoredCommand` (`OpenChat::stored`) as a private file under the chat and its command id
   (`device-v1/<account digest>/outbox/<chat>/<command>.command`, no plaintext prompt) and
   removes it when the delivery is done or failed. When the chat is read again,
   `OpenChat::resume` tracks each kept command's pending row again and resumes its delivery
   with the same bytes and a receipt probe; one with nothing left to deliver (its run started,
   its window ended) or that does not open for the chat is forgotten.
3. **Seats are the core's.** `StartInstance` names the agent, the enrolled seat's pool and the
   chat, and hub-io sends it as it is; an explicit Wake runs the core's `seat::WakeSeat`. An
   agent without an enrolled seat is the core's `NotEnrolled`, which the chat port reports as
   `AgentNotStarted` (start the agent first, which enrols it).
4. **Opening.** The neutral `Freshness` has `Opening` for a chat before its first round (the
   default); a window shows nothing alarming for it, and every freshness a subscription
   delivers is applied.
5. **Tool rows.** The neutral `ToolRow` carries the core's title (`"<destination> · <label>"`)
   and its typed `ToolOutcome`; `nocterm-cloud-chat` uses both instead of rebuilding them.

## Consequences

- A command a person sent survives a restart: its pending row shows again and settles from
  the receipt or the runner's events.
- Outbox files of the earlier layout (`outbox/<publication>.publication`) are no longer read.
- The device core's reducer version 3 makes earlier chat checkpoints read again from the hub
  once.
