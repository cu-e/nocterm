# Architecture decision records

Each record states one accepted decision: its context, the decision, the
alternatives rejected and the consequences. Records are not edited after
acceptance; a later record supersedes an earlier one.

| # | Decision |
| - | -------- |
| [0001](0001-settings-sections.md) | Settings are typed sections owned by the crates that use them |
| [0002](0002-commands-through-dispatch.md) | Item commands are GPUI actions dispatched to the command item |
| [0003](0003-credential-store-port.md) | Terminals reach saved credentials through a port |
| [0004](0004-write-queue.md) | Persisted files share one bounded write queue |
| [0005](0005-agent-runtime-service.md) | The agent runtime is a service crate reached through a client port |
| [0006](0006-workspace-ports.md) | The side panel is a state machine; tabs open through a session factory |
| [0007](0007-bootstrap.md) | One bootstrap for the application and its GUI tests |
| [0008](0008-code-limits.md) | Function size and complexity limits shrink a grandfathered list |
| [0009](0009-thread-composer.md) | A chat's composer state is kept apart from its conversation |
| [0010](0010-session-lifecycle.md) | A chat's session is a pure state machine, connected when shown |
