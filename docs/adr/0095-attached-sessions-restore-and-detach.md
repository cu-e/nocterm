# 0095. Attached sessions restore their screen and detach on close

Status: accepted

## Context

A terminal tab owns the session it opens: the tab sends the target and launch to a
`Transport`, receives `Event::Output` and, when the tab closes, asks the session to end. That fits
a shell the tab started. It does not fit a session that already runs somewhere else and that a
tab only looks at: a long-lived shell on a remote node can be shown by several tabs, in turn or
at once, and closing one tab must not kill the program.

Two things are missing. A tab that attaches later has nothing on screen: the program drew it
before the tab existed, and replaying every byte it ever printed is neither possible nor wanted.
And a tab can only use the transports installed for every terminal (`Deps`) or the one a
`LocalCommand` brings, a command that ends with its tab.

## Decision

**`Event::Restore(Vec<u8>)` is the session contract's picture of the screen.** A transport that
attaches to a session sends the screen as its host holds it right now, before the output that
follows. The owner of the screen discards what it drew (rows, scrollback, modes, selection, a
running search, the title and the shell state it learned), paints the bytes on a cleared
emulator of its current size and goes on with the output after it. The emulator is cleared in
place and its change counters keep counting forward, so a cache keyed by them never takes the
restored screen for an earlier state. The bytes are not output of the program: they are not
recorded, and of what they provoke only a window title takes effect. No answer reaches the
program (a status query inside a picture is not a question), no bell rings and no clipboard is
written. The decoder of the owner starts afresh around them so a cut-off character cannot join
the next output. `Event` stays exhaustive; every consumer decides what a
picture means to it, and an owner that has no screen ignores it.

**A tab can be attached to a session reached through its own transport.** `AttachedSession`
holds a title, the destination the owner of the transport names, and the transport, and `nocterm_terminal::open_attached` opens a tab on it the way
`open_local_command` opens one for a command: nothing is launched, no profile is saved, the
tab never reconnects by profile. The transport is bound to one session; what it does with the
tab's requests is its contract.

**Closing an attached tab detaches.** The tab sends `Command::Close` and drops the session, as
every tab does. For an attached transport that means leaving the session, not ending its
program; ending it is a separate act that the owner of the transport offers elsewhere.

**One connection starts its automatic recording once.** A transport that reconnects by itself
reports `Connected` again after `Connecting`. That is the same connection continuing, so the
terminal starts the automatic recording at the first `Connected` of a session only; a new
session, such as the one `Reconnect` opens, starts it again.

## Consequences

- A transport that can re-attach has one way to show the current screen, with no change to
  `ConnectRequest` or the existing transports.
- The one exhaustive match on `Event` outside the terminal, the SSH test support, adds an arm
  and keeps ignoring pictures; consumers that already end in `_` are unchanged.
- An attached tab is a command-like tab: no reconnect button, no saved profile, no recording
  at all (the action is hidden). A later change that wants any of them for attached sessions decides it explicitly.
- Detaching on close is a rule of the transports that implement attached sessions, not of the
  terminal; the terminal never learns whether the program lives on.
