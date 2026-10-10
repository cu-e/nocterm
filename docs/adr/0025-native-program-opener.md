# 0025 — Explicit native program opening

Date: 2026-10-10
Status: accepted

Explorer previously started configured local programs with `Command` and created
an unbounded child-wait thread in the UI feature. Its launch dependencies were
absent from composition and errors depended on the lifetime of the invoking view.

`nocterm-session` defines `ProgramOpener`, the immutable `ProgramOpenRequest` and
typed `ProgramOpenError`. Requests preserve literal argument boundaries and native
working directories; the port performs no shell parsing or environment expansion.
An unpolled request launches nothing. Success acknowledges physical process spawn,
not successful completion of the external editor.

The existing local adapter supplies one `LocalProgramOpener` shared by all
application windows. At most 32 admitted start-and-wait workers own direct children.
A worker is created before it spawns the program, and it owns both the child and
admission before sending the acknowledgement. Spawn or worker-creation failure
releases admission. Natural exit and reaping release it. An exceptional wait
failure retires its slot because reaping has not been proven. Receipt or opener
handle cancellation cannot kill an accepted child. Standard input/output/error are
null. The owner uses neither the PTY supervisor nor process groups or Windows job
objects, so accepted editors can outlive the launching application. These programs
are outside application shutdown persistence and transfer draining.

Files receives the opener in `Deps`; panes retain its shared handle with their
original window handle. An App-owned task reports a typed launch failure only to
that original window. Removing a pane does not cancel an admitted launch, and
closing the originating window never redirects an error to another window.
Configured remote editors still use the workspace's host-session port. System
default applications and file-manager actions still use the platform's existing
open/reveal operations.

The shared session `FakeExec` also implements `ProgramOpener` with bounded queued
answers, literal request recording and deterministic suspension. It performs no
I/O, creates no threads and records a request only when the future is polled.
Native tests exercise child ownership, process lifetime, literal arguments, working
directories, failure and admission; GUI tests exercise configured local/remote
routing and window-specific asynchronous notices.
