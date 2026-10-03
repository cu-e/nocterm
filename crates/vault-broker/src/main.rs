//! Root-owned Linux fingerprint key release. Never stores keys on disk.
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
