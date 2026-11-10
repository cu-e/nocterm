# 0106. Terminal byte state and command execution have shared headless owners

Status: accepted. Supersedes the deferred command-execution extraction alternative
in [0063](0063-agent-chats-below-the-panel.md).

## Context

Command execution was owned by a private module of the GPUI chat service, while
pure charset decoding and shell-protocol observation were private to the terminal
feature. A headless consumer cannot depend on either crate. Sharing source by
copying it would allow command limits, cancellation and terminal semantics to drift.
A service cannot depend directly on an execution adapter under the layer rules.

## Decision

`nocterm-session::command_jobs` declares the narrow `CommandJobs` and
`CommandJobsFactory` ports, neutral command snapshots, states, starts and typed
admission errors. `nocterm-agent-tools` is the adapter that owns the original
command registry and its runner. The application injects its factory through the
existing retained agent installation and runtime. Every chat obtains an independent
registry; its service schedules returned futures with its existing absolute timer
and retains its watchdog, approval and terminal-authority decisions. No new global
or shutdown handler is introduced.

The registry preserves four active commands, sixteen retained outcomes, a 64KiB
stdout prefix with continuous draining/counting, typed startup errors and the
transport's bounded stderr tail. Every job retains its originally admitted
executor. Cancellation drops output and preserves an unknown exit. Cancelling a
read waiter does not cancel its command.

`nocterm-vt` owns the pure incremental `TextCodec` and bounded `ShellIntegration`
OSC observer as well as screen emulation. It uses the existing session-domain
charset and conversion-error values. Terminal views retain session admission,
`ExpectedSession` checks, shell input and command-lease decisions, and reset
lifecycle. These helpers introduce neither GUI dependencies nor I/O into VT.

Shared session test support supplies scripted command observations for service and
feature tests without calling an executor. Adapter conformance tests use the
existing `FakeExec` and real bounded output channels to verify execution, draining,
deadlines, ownership and cancellation through the public contract.

## Alternatives

A direct service-to-adapter dependency violates the established layer graph.
Moving actual execution into a domain crate mixes I/O into pure contracts.
Copying private helpers creates multiple owners. Moving all chat tools into a
headless adapter would also move approvals and GPUI session authority, which
remain responsibilities of the chat service.

## Consequences

Headless consumers can reuse command execution and terminal byte state without
linking GPUI. Desktop command authority and lifecycle remain local and explicit.
These shared primitives confer no account, remote execution grant, background
runner lifetime or Cloud Chat durability.
