# 0055. Snippet document I/O and viewless publication have separate owners

## Context

The retained snippet installation kept editor authority explicit, while its feature
still acquired native files, generated UUIDs and owned a viewless GPUI library model.
Those are separate domain, native adapter, service and presentation responsibilities.

## Decision

`nocterm-snippets` retains the exact library format and checked mutations, and provides
narrow blocking SnippetRepository and supplied SnippetIds contracts. Typed storage
errors retain a read/write operation and the original concrete source. The domain
performs no native I/O or entropy acquisition.

`nocterm-snippets-io` implements supplied-path acquisition/publication with the existing
core persist load/save operations, and native UUIDv4 generation. It does not change
comment rewriting, TOML bytes, atomic publication or missing-file behavior. Validation
and read-only fallback remain service policy.

`nocterm-snippets-store` owns the actual viewless four-field model around Persisted.
Composition prepares the loaded library before service creation, preserving malformed
and invalid input as empty read-only state with the original shared diagnostic. The
service uses the same durable queue32 or immediate memory path and shared shutdown
coordinator. Writers capture the supplied repository without App access; no second
worker, cache or quit hook is introduced.

Feature Installed retains three private collaborators: the actual SnippetsHandle,
original SettingsHandle and supplied SnippetIds. A new editor calls the identity
capability once at draft construction. Existing editors, rendering, saves and retries
preserve their current ID. Editor controls, original expected records, modal completion
and terminal paste authority remain governed by ADR0050. Bootstrap retains only an
installation whose service registered with the common coordinator.

## Alternatives rejected

- Moving only native file calls leaves viewless background state in a feature.
- Moving only the model leaves feature-native acquisition and identity generation.
- A generic I/O or identity registry obscures these two narrow consumer contracts.
- A new persistence worker duplicates the shared ordered publication/capture driver.
- Preserving comments or changing expected-record matching silently changes existing
  data behavior under an ownership refactor.

## Consequences

This supersedes ADR0050's exact two-collaborator/library-entity installation shape and
its deferred native/service boundary. Its control, receipt and modal authority rules
remain. SnippetsLoadError::File contains typed SnippetStorageError rather than native
PersistError directly; the original error and native source allocation remain available
through the variant and standard error chain. Storage display and domain errors stay
unchanged. Formats, settings schema and release metadata do not change.
