# 0122. Cloud chat creation protocol

Status: accepted

## Context

A Cloud chat is an end-to-end encrypted conversation between a device and a runner. The
hub stores only ciphertext, so the hub cannot create one. Before this decision nothing
created the first chat of a runner: the device source required an existing writer, and a
runner claimed a writer only when a command touched the stream. The device must also not
fabricate a writer ([ADR 0118](0118-account-encrypted-cloud-hub-bootstrap.md) keeps the
account material immutable), and a new command purpose would need an acknowledgement on an
empty chat.

## Decision

1. **The device mints the chat.** It draws a `CloudChatId` from the OS RNG and a first key
   epoch of 1. The writer epoch is claimed by the runner, never by the device.
2. **The destination runner.** A managed chat calls `start_instance_for_chat(agent, chat)`,
   which is idempotent and returns the instance runner; that runner must already be in the
   account registry, otherwise the result is `NotEnrolled` and the person starts the agent
   again. A plain chat uses a registry runner that is not an instance runner.
3. **Material.** `CloudAccountMaterial::create` stores the chat key wrapped for the account
   key, compare-create, idempotent per stream.
4. **Grant, then publish.** The sealed `ChatKeyGrant` is stored in the account as a
   `cloud-chat-grant` record before it is published. A retry republishes the stored bytes;
   a different blob for an installed epoch would fail closed at the runner. Only a
   `StoredChatKeyGrant` can publish.
5. **The runner opens the chat.** Admitting a grant yields `CommandInput::ChatOpened`. It
   is not a command: no acceptance, no acknowledgement, no control advance. The coordinator
   answers it with its memoized chat claim: writer epoch 1, an empty head and the catalog
   row. A repeated grant is a no-op.
6. **The device waits.** It opens the source while the writer is `Unavailable`, with a
   one second backoff and a 60 second deadline, republishing the stored grant every 15
   seconds. On the deadline the result is `RunnerNotReady`; the chat appears in the
   catalog once the runner claims it.
7. **Prompt policy.** A managed chat uses `ConversationPromptPolicy::runner_default`, which
   sends no target, so the runner's default destination applies, and never sends an
   existing session. The policy digest carries a tag byte, so default and targeted
   policies never share a fingerprint. A plain chat is controls-only: history, cancel,
   delete.
8. **Discovery.** Another device walks hub trust, account material, the runner registry and
   the authenticated catalog of each registered runner, then opens an entry with the same
   loop and no wait. A managed entry calls `start_instance_for_chat` first, which wakes an
   idle seat.
9. **Opening from Start.** Start stays seat-only
   ([ADR 0120](0120-cloud-agent-catalog-in-the-chat-menu.md)). When the start is ready the
   agent panel shows "Open chat" and opens the chat at once in the window that pressed Start.

The device side lives behind the `CloudChatLauncher` port of `nocterm-cloud-agents-model`,
implemented by `nocterm-hub-io`; the menu reaches it through `CloudAgentsAccess`.

## Consequences

- No new command purpose, no writer minted on a device, no change to the immutable chat
  material record, and the `CloudAgentLauncher` port and `StartedAgent` stay unchanged.
- A runner that crashes between acknowledging the grant and claiming the chat is recovered
  by the periodic republish.
- A plain chat has no prompting until a device provider capsule exists.
- The first prompt of a managed chat depends on the runner's default destination.

## Alternatives considered

- **A device-side creation source with no writer.** Rejected: it fabricates writer 1.
- **A "create chat" command purpose.** Rejected: closed purposes must not grow and it needs
  an acknowledgement on an empty chat.
- **Minting the chat id during Start.** Rejected: it changes the port of ADR 0120, and a
  retried start orphans chat tokens. Creating with the chat id also wakes an idle seat when
  another device opens the chat.
- **Prompting plain chats now.** Rejected: it would duplicate the provider-capsule work.
- **Growing the chat material record with the grant.** Rejected: it is fixed-format and
  bounded to 1024 bytes; a separate record kind is bounded by the envelope limit.
- **A hub listing of chats for discovery.** Rejected: it leaks structure and is not
  authenticated by the account.
