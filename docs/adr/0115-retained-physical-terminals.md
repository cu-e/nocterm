# 0115. Physical terminals outlive immutable run views

Status: accepted

## Context

A Cloud chat can issue another prompt while its original runner-owned SSH/PTY and
background commands remain alive. A run-local registry that closes on drop cannot
provide that lifetime. Cloning Session and adding an event reader is unsafe:
Session has one receiver, so the readers would divide bytes. Rewriting an old
job's run or receipt would substitute authority and lose its durable effect link.

## Decision

`nocterm-agent-tools` owns `RetainedTerminals`, explicitly retained by native
composition for one account/chat/runner-key generation. It owns at most sixteen
physical terminal entries and a shared four-active/sixteen-retained job registry.
Each physical entry owns its original Session, exact SessionReceipt, bounded VT,
charset decoder, prompt/failure state and one supervised reader. Closing the
supervisor or explicitly removing the receipt retires that physical resource.
A dropped reader, including before its first poll, closes its physical resource.
There is no new connection, copied byte stream or second reader on reattachment.

SessionRegistry remains an immutable RunBinding view. Each retained attachment
gets a host-supplied fresh opaque handle for an exact existing receipt. Detach,
completion and drop fence that view without closing a later run's connection.
Ordinary unretained registries preserve close-on-drop. Views are not grants:
every new input, exec, connection or cancellation needs its current exact finite
authority. A closed or lost connection is not silently authenticated again.
RetainedToolsFactory implements the domain RunToolRuntimeFactory seam through
these views; application composition supplies its original physical owner.

Background jobs retain their original BackgroundCommand, dispatch, run/session,
terminal and command IDs. Their first native poll still requires the original
strict ToolAuthority after durable Retained acknowledgement. Subsequent draining
checks the exact physical receipt and a separately injected original-job
BackgroundCommandAuthority, including original source/grant revocation and expiry.
Foreground completion does not retarget or revoke that original liveness owner.
The existing command timeout remains finite and unchanged. New run read/cancel
requires its own authority for that exact physical receipt, then addresses the
original command without rewriting its journal identity.

BackgroundCommandCleanup is a separate consumer capability: it owns current
cleanup writer authority and validates the actually retained original effect.
It may append an already observed historical terminal fact after foreground
retirement or writer replacement, without renewing target I/O or minting Retained.
Unknown outcomes preserve their original mutation fence. Actual journal write and
cleanup capability issuance remain with the native journal/coordinator owner.

PTY live ownership uses a fresh private Arc identity allocated by the physical
owner on every acquisition. Full run/epoch and intent remain descriptive metadata.
A prompt may complete that owner before its read waiter resumes. Releasing or
dropping an old waiter matches only its original Arc by pointer identity, so even
repeated caller coordinates cannot reset a newer live command. VT restoration still
confers no prompt or execution authority. Unknown-effect reconciliation remains
the authoritative journal/coordinator's responsibility.

## Alternatives

A second Session reader would lose output. Draining a run registry into a new one
would orphan old observers and jobs. Rebinding an old request would grant it new
run authority. Opening a second PTY would lose the original cwd/projection and
could require new SSH consent. Keeping every old run alive would duplicate budgets
and obscure admission versus cleanup lifetime.

## Consequences

Physical connections and bounded output continue across prompts and desktop
attachment changes while the runner's original source permits them. The device's
login authorizer is not retained after SSH authentication. A lost connection needs
fresh consent; this owner provides no key custody or automatic reconnect. Durable
unknown effects, account/global budgets, source trust, job liveness policy,
shutdown scheduling and native cleanup writer authority are explicit collaborators.
