//! Pure Cloud run authority, effect ordering and bounded fair admission.
//!
//! Owns: runner/run/target authority coordinates, finite grant policy,
//! durable-effect transition rules, unknown-outcome fences and scheduling decisions.
//! Device-issued SSH login consent is finite and single-use; it binds exact checked
//! ssh-keys proof values through a constrained SessionLoginAuthorizer port.
//! Does not own: account authentication, cryptography, signed/wire formats, native
//! I/O, clocks, entropy, provider SDKs, GPUI, persistence execution or local history.
//! Ports provided: host-controlled engine, scoped tool execution and atomic journal
//! contracts, charged public publication and signed-baseline transfer ports, plus
//! explicit native-tool authority, timer, session opening, supervised
//! task retention, background effect journal, original-job liveness, cleanup and
//! durable model-call admission collaborators. RunToolRuntimeFactory attaches
//! immutable run views without reconnecting; cleanup cannot authorize a dispatch.
//! The mandatory JournalCatalogCipher binds a complete native catalog to one known
//! account/runner independently of chat keys. CurrentCloudHeads observes bounded
//! revision-bound native head metadata; JournalChatHeadCipher protects it after
//! content-key retirement. Neither port admits commands or acquires a writer.
//! Ports used: neutral AI content,
//! tool calls and tool presentation;
//! hubs-model captured descriptive hub/account identities.
//! Settings sections: none. Globals: none.
//! Invariants: IDs describe ownership, never prove authorization; every operation
//! retains its original source and epochs. Only durable dispatch confirmation may
//! release an effect. Unknown mutations fence fresh writes until explicit original-
//! intent reconciliation within a run; the host retains unresolved target effects across runs.
//! Signed event identity, durable transport delivery identity and positive relay
//! cursor remain distinct. Staging a baseline permits no prefix deletion; only
//! its matching stored receipt and purge completion permit native activation.
//! Time is supplied, limits are finite, and idle transcripts
//! allocate no run reservation. Canonical signed bytes belong to the hub contract.

pub mod admission;
pub mod authority;
pub mod coordinator;
pub mod engine;
pub mod grant;
pub mod identity;
pub mod inference;
pub mod journal;
pub mod login;
pub mod payload;
pub mod run;
pub mod tool;

#[cfg(any(test, feature = "test-support"))]
pub mod testing;

pub mod projection;
