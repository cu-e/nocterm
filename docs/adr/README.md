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
| [0006](0006-workspace-ports.md) | The side panel is a state machine; tabs open through a session factory (side-panel slot superseded in part by 0081) |
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
| [0026](0026-workspace-state-owners.md) | Workspace transitions belong to private state owners with explicit shared layout (Sidebar and RightPanelSlot superseded in part by 0081) |
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
| [0045](0045-private-panel-view-owners.md) | Private panel view owners retain controls and original completion authority |
| [0046](0046-shared-concrete-error-causes.md) | Shared error causes retain the original concrete error in standard source chains |
| [0047](0047-connection-publication-authority-service.md) | Connection publication service preserves committed profile authority histories |
| [0048](0048-guarded-connection-facts.md) | Original-source guarded raw facts and bounded connection jobs |
| [0049](0049-bounded-panel-coordination.md) | Private panel membership, notice tasks and finite presentation operations |
| [0050](0050-retained-snippet-installation.md) | Snippet views retain their original installation and private draft controls |
| [0051](0051-retained-keymap-installation.md) | Keymap pages and directory actions retain supplied ownership across windows |
| [0052](0052-retained-agent-installation.md) | Agent views retain runtime, policy and host installation authority |
| [0053](0053-retained-settings-ui-installation.md) | Settings UI retains its supplied settings installation |
| [0054](0054-checked-settings-text-receipts.md) | Native settings text owns checked publication receipts |
| [0055](0055-snippet-storage-boundaries.md) | Native snippet documents and viewless publication have separate owners |
| [0056](0056-owner-settings-pages.md) | Settings controls and pure schemas live with their actual owners |
| [0057](0057-checked-settings-catalog.md) | Checked ordered settings catalogs and explicit settings composition (page list and navigation superseded in part by 0086) |
| [0058](0058-neutral-agent-protocol-boundary.md) | Agent protocol types stop at the ACP adapter |
| [0059](0059-architecture-procedure-skills.md) | Architecture rules are applied through committed procedure skills and guards |
| [0060](0060-ssh-jump-host-routes.md) | SSH jump-host routes are resolved from saved profiles before opening and established by the SSH adapter |
| [0061](0061-agent-profiles-and-directory.md) | Agent profiles, an injected catalog and one directory of available agents |
| [0062](0062-terminal-contracts-below-the-workspace.md) | Terminal and connection contracts live below the window shell |
| [0063](0063-agent-chats-below-the-panel.md) | Agent chats live in a service crate below the panel |
| [0065](0065-retained-context-surfaces.md) | Context surfaces retain their native dock and original callback authority (whole-window selection superseded by0067) |
| [0066](0066-scoped-chat-storage.md) | Scoped chat authority precedes payload discovery in versioned storage |
| [0067](0067-project-tabs-with-inner-surfaces.md) | Workspaces are outer connection tabs with independent inner terminal surfaces (footer order superseded in part by 0083) |
| [0069](0069-workspace-chat-launch-boundary.md) | Workspace chat scope preserves the existing local agent launch lifecycle |
| [0070](0070-original-workspace-connection-recovery.md) | Workspace authentication belongs to its inner terminal |
| [0071](0071-provider-workspace-locations.md) | Workspace locations retain provider identity and session authority |
| [0072](0072-shared-directory-selection.md) | Directory selection shares read-only rules, adapters and controls |
| [0073](0073-terminal-free-local-workspace-creation.md) | Terminal-free workspace creation uses explicitly injected local authority |
| [0074](0074-captured-host-image-search-and-language-versions.md) | Image search and language versions retain captured host authority |
| [0075](0075-observed-workspace-environment-management.md) | Workspace tool changes retain explicit environment authority |
| [0076](0076-container-filesystem-capability.md) | Container files use a scoped session filesystem capability |
| [0077](0077-selected-workspace-monitoring.md) | Resource monitoring follows the selected workspace environment |
| [0078](0078-explicit-workspace-container-removal.md) | Explicit workspace container actions precede checked metadata removal |
| [0079](0079-project-terminal-contracts-below-shell.md) | Project terminal contracts live below the shell with captured typed placement |
| [0080](0080-project-adapter-ownership.md) | Project resource, environment and filesystem adapters have separate owners |
| [0081](0081-panel-groups-own-side-panels.md) | Panel groups own the window's side panels |
| [0082](0082-system-plugins-and-core-contributions.md) | Built-in shell contributions are core or belong to a system plugin in one checked catalog |
| [0083](0083-zoned-status-bar.md) | The status bar is zoned and drawn from a layout resolved against the plugin catalog |
| [0084](0084-layout-presets-are-app-wide-settings.md) | Layout presets are app-wide settings that every window follows |
| [0085](0085-one-plugin-model-with-separate-trust-tiers.md) | System and user plugins share one contribution and settings model, with separate trust tiers |
| [0086](0086-plugin-enablement-and-the-plugins-page.md) | Plugin enablement is app-wide and the Plugins page lists plugins with their own pages nested under it |
