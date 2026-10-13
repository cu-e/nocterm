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
| [0011](0011-chat-configuration.md) | A chat owns its configuration, and every shown chat connects |
| [0012](0012-architecture-debt-ratchets.md) | Architectural debt is an exact inventory that may only shrink |
| [0013](0013-persistence-shutdown-service.md) | Ordered persistence and quit capture belong to a service |
| [0014](0014-vendor-source-replay.md) | Vendored sources replay from authenticated archives and ordered patches |
| [0015](0015-private-file-publication.md) | Private file publication shares one staged, atomic mechanism |
| [0016](0016-vault-storage-boundary.md) | Vault format and rules are independent of storage and lifecycle |
| [0017](0017-local-filesystem-capabilities.md) | Native filesystem capabilities are injected through pure session ports |
| [0018](0018-transfer-runtime-capabilities.md) | Transfer execution is an adapter over held filesystem and retryable publication ports |
| [0022](0022-agent-storage-owners.md) | Agent storage adapters and private lifecycle owners share persistence execution |
| [0025](0025-native-program-opener.md) | External local programs have an explicit bounded native owner |
| [0026](0026-workspace-state-owners.md) | Workspace transitions belong to private state owners with explicit shared layout |
| [0027](0027-connection-observation-ports.md) | Pure connection observation contracts and network I/O adapter |
| [0028](0028-theme-library-boundary.md) | Pure theme domain model, adapter registry and theme service |
| [0029](0029-output-recording-shared-driver.md) | Bounded append persistence and output recording |
| [0031](0031-archive-hydration-owner.md) | Hydrated archive and history permit state belong to a single owner |
| [0032](0032-narrow-terminal-ports.md) | Separate terminal ports bind approvals and secret answers to original Item/session/prompt epochs |
| [0033](0033-native-agent-sandbox-capabilities.md) | Inject native workspace preparation and fail-closed sandbox launch capabilities |
| [0034](0034-chat-config-composer-owners.md) | Per-chat configuration and composer transitions have private feature owners |
| [0035](0035-injected-image-codecs.md) | Inject bounded native image codecs and bind preview delivery to request generations |
| [0036](0036-connection-profile-native-boundary.md) | Connection documents and native values use explicit domain ports |
| [0037](0037-chat-document-semantic-revisions.md) | Durable chat revisions, pending candidates and capture have separate ownership |
| [0038](0038-private-terminal-state-owners.md) | Private terminal state owners commit input and presentation after captured session admission |
| [0039](0039-remote-os-observation-boundary.md) | Remote OS observation uses checked identities and an injected bounded adapter |
| [0040](0040-native-chat-metadata.md) | Native chat identity and timestamps use explicit host capabilities |
| [0042](0042-terminal-installed-dependencies.md) | Terminal sessions retain explicit installed providers and settings across windows |
| [0043](0043-retained-connections-installation.md) | Connection views retain explicit owners, settings and subscription lifetimes |
| [0044](0044-private-thread-coordination.md) | Private chat owners return named coordination effects |
| [0046](0046-shared-concrete-error-causes.md) | Shared error causes retain the original concrete error in standard source chains |
