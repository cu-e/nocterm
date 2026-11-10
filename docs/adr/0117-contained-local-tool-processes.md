# 0117. Cloud local tools receive an explicit contained process capability

Status: accepted

## Context

A headless runner retains account keys and finite provider credentials. A Local
tool child must not inherit access to that supervisor's home, state files,
process memory, environment or sockets. Isolating only the interactive shell
leaves structured `HostExec` calls as another native process launch path.

## Decision

The Linux `nocterm-local::ContainedLocal` adapter supplies both `Transport` and
`HostExec`. Constructing it requires a successful actual bubblewrap/prlimit probe.
The caller performs preparation during bounded native startup before admitting
work. A missing executable or denied namespace yields a typed failure.

Each child receives fresh user, PID, network, IPC and mount namespaces, dropped
capabilities, an empty environment with explicit HOME/PATH/LANG, a fresh proc/dev/tmp
and a single private workspace at `/workspace`. System tools under `/usr` and
their libraries are mounted read-only. The supervisor's private state must be
separate from the workspace and these system mounts; linked or public workspace
directories are refused. The parent must protect their filesystem ancestry.
The child has no shared host network or home, Docker socket or supervisor mount.

Prlimit runs inside the new namespace before the requested program. Finite hard
and soft limits constrain address space, process count, descriptors, CPU seconds
and individual file size. Applying the process limit to the host launcher would
incorrectly count unrelated processes of its host uid. The namespace launcher
remains owned by the existing local native process/PTY lifecycle.

All program arguments are literal argv entries after option terminators.
Interactive sessions expose this same contained structured executor, rather
than the ordinary local executor. Captured connection authority is retained by
the session and every structured execution operation. Closing or revoking the
source uses the existing native process cleanup machinery. Desktop Local
transports retain their existing explicitly composed capabilities.

## Consequences

The runner supervisor still supplies distinct account principals, aggregate
cgroup limits, workspace storage quotas and bounded startup/shutdown. Child
address-space and per-file limits do not establish an account RSS or total disk
quota. Remote SSH execution retains its separate principal and grant boundaries.
The runner composition must choose the contained capability explicitly before
admission; an unavailable Linux boundary cannot select the ordinary Local adapter.

Live Linux fixtures exercise actual namespace execution, hidden supervisor files
and parent processes, an isolated network, literal arguments, PTY/structured
execution parity and source revocation. They require installed bubblewrap and
prlimit plus enabled user namespaces; they can be selected with `--include-ignored`.
