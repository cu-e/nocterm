# 0074. Image search and language versions retain captured host authority

Status: accepted

## Context

New container workspaces need local image filtering, registry repository discovery
and explicit toolchain versions. Registry search differs between engines and must
not acquire authority from whichever terminal becomes active later. A language
label alone cannot represent a user's requested version truthfully.

## Decision

`nocterm-project-providers` owns a narrow `ImageSearch` contract, bounded checked
queries, neutral repository results and typed failures. The separate preparation
contract accepts checked `LanguageSelection` values. Language-specific version
rules reject unsupported syntax before I/O. Distribution defaults, stable channels
and numeric requests remain distinct choices.

`nocterm-project-runtime` implements registry search through the supplied original
`HostExec` and selected Docker or Podman engine. Docker searches Docker Hub;
Podman searches the user's configured registries. Engine configuration and existing
credentials retain their owners. Search uses bounded machine-readable commands,
does not pull images and provides no universal tag browser or registry API.
Selected language versions are installed and verified inside new supported
containers. Unavailable requested versions fail preparation without a silent
fallback to distribution tools.

The Projects feature composes independent image and language presentation owners.
Local filtering is immediate and independent of registry errors. Registry work is
debounced, cancellable and generation checked, with a bounded short-lived cache
keyed by original source, engine and query. Rate-limit failures impose a conservative
cooldown without automatic retries. A manually supplied complete reference remains
usable. Selecting a result preserves its literal reference without starting another
search. Language enablement preserves version drafts; custom images retain their
existing tools. The application explicitly injects the image-search collaborator.

## Alternatives

- Extending preparation with discovery responsibilities broadens an unrelated port.
- Searching through a current terminal can move requests to a different host.
- A bundled registry SDK and credential store duplicate the engine's existing setup.
- Relabeling distribution packages with a numeric version misrepresents installed tools.
- Rewriting typed image names or treating repository matches as tags changes user intent.

## Consequences

CLI search availability and metadata depend on the selected engine and registry.
Search failure cannot block local selection or manual references. Installation
errors remain preparation failures before publication. Pure values and validation,
process I/O, UI state and application composition retain their existing layers.
