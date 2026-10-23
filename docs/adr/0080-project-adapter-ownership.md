# 0080. Project resource, environment and filesystem adapters have separate owners

Status: accepted

## Context

Project preparation combined resource creation and rollback, bounded host commands,
managed tool installation, session activation and container filesystem streaming.
These responsibilities change for different reasons. Their shared executor and
container identity must survive separate owners without adding adapter dependencies
or granting existing resources the privileges of fresh creation.

## Decision

The composition root supplies three independent adapters through domain contracts:

| Adapter | Owns | Provides |
| --- | --- | --- |
| `nocterm-project-runtime` | Preparation, ownership/reconciliation, rollback, bounded commands, image search, pull progress, inspection and explicit lifecycle actions | `ProjectPreparation`, `ImageSearch`, `WorkspaceContainerActions`, `EnvironmentCommandFactory` |
| `nocterm-project-environment` | Platform/account observations, managed language recipes and workspace activation | `WorkspaceEnvironment`, `CreatedWorkspaceEnvironment`, `WorkspaceSessionFactory`, `WorkspaceCommandActivation` |
| `nocterm-project-container-fs` | Container-scoped transports/execution and bounded filesystem streaming/publication | `ContainerSessionFactory`, `RemoteFs` |

All dependency kinds between these adapters are forbidden. Their normal and test
contracts come from domain/foundation crates. Native filesystem and environment
acquisition for local activation remain in the environment adapter.

`nocterm-project-providers` owns the pure `ContainerLocation::exec_request` builder
and the narrow command, creation-installation and activation contracts. The builder
preserves the original engine, full container ID, argument order and stdin, with an
explicit choice of workspace working directory. It performs no I/O and owns no
native activation scripts, timers or process execution.

`EnvironmentCommands` exposes only run, check, executor, scoped-executor and
observation operations. Runtime keeps the concrete bounded command owner. Its scopes
share the original started instant, cancellation, progress and total budget;
installation receives that context rather than starting another factory context.
Only existing bounded recovery deliberately uses a fresh uncancelled context.

Creation passes its actual `OwnedContainer` receipt to the separate
`CreatedWorkspaceEnvironment` capability. Ordinary `WorkspaceEnvironment::install`
accepts no fresh flag or owned-creation authority. Fresh Arch package-database
initialization remains confined to the creation path. Runtime retains destructive
inspection and exact attempt/project/ID checks; an installer failure cannot authorize
deleting an attached or otherwise unowned resource.

The environment adapter owns `NativeWorkspaceSessions` because its fixed exports
share the managed installer layout. Container sessions receive its
`WorkspaceCommandActivation` explicitly and activate requests before exact-container
wrapping. Filesystem commands use `/` independently of workspace cwd, nested sessions
retain the same filesystem Arc, and unavailable container capabilities never project
the host filesystem. The bounds and publication identity checks from
[ADR0076](0076-container-filesystem-capability.md) remain unchanged.

Startup constructs command factory, environment and activation, then preparation
and container sessions. Bootstrap retains the supplied trait handles in HostServices
and ProjectResources and forwards those same handles to Projects and Terminal.
No adapter acquires another through a global or hidden concrete constructor.

Public composition tests live under the existing app project-setup owner and use
the real command, environment and container adapters with shared session fakes.
They retain installer arguments/download policy, fresh/existing Arch behavior,
suspended-install cleanup authority and actual activation/nested-session assertions.
Pure probe and activation tests stay with environment; filesystem tests use the
shared inert recording activation fixture. Runtime tests isolate the creation
collaborator through the existing shared provider fake. No adapter test dependency
recreates an adapter edge, and no fake duplicates native command deadlines.

## Alternatives

An environment or filesystem dependency on runtime would couple independent adapters
and violate the layer rules, including for development dependencies. A third shared
executor adapter adds another owner without a separate lifecycle. Moving timers,
native activation or process execution into the domain would add I/O to pure rules.
Compatibility reexports would conceal the final owner and preserve old coupling.

## Consequences

Each adapter has one responsibility card with short invariants; the application owns
cross-adapter assembly and composition tests. Domain seams preserve original authority
without expanding the preparation API into environment or filesystem policy. There
are two new internal crates and no new external dependencies, services, globals or
shutdown handlers.

This updates the adapter ownership described in
[ADR0071](0071-provider-workspace-locations.md),
[ADR0074](0074-captured-host-image-search-and-language-versions.md),
[ADR0075](0075-observed-workspace-environment-management.md) and
[ADR0076](0076-container-filesystem-capability.md). Their behavior and authority
requirements continue to apply.
