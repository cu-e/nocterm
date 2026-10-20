//! Root-owned Linux fingerprint key release. Never stores keys on disk.
//!
//! Owns: the system D-Bus service, caller identity from the bus, fprintd
//! verification and the bounded in-memory key store.
//! Does not own: vault formats, envelopes, desktop sessions or installation.
//! Ports: provides the `dev.nocterm.VaultBroker1` D-Bus interface that
//! `nocterm-device-unlock` calls; uses system fprintd. Settings: none. Globals: none.
//! Invariants: keys stay in memory, bound to the calling user; each entry has
//! bounded verification attempts and expires after its idle lifetime.
#[cfg(target_os = "linux")]
mod service;
#[cfg(target_os = "linux")]
mod store;
#[cfg(target_os = "linux")]
#[tokio::main(flavor = "multi_thread", worker_threads = 2)]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    service::run().await
}
#[cfg(not(target_os = "linux"))]
fn main() {}
