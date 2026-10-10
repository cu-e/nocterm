//! Durable vault adapter with one bounded worker.
//! Owns: files, locking, OS entropy, native registration orchestration and epochs.
//! Does not own: encrypted format, cryptographic rules, native authenticators or UI.
//! Ports provided: VaultAccess. Ports used: DeviceUnlockProvider.
//! Settings: none. Globals: none. Invariants: locks span staged publication;
//! canceled results cannot expose credentials; only committed candidates install.
mod access;
mod backend;
mod entropy;
mod storage;
mod worker;
pub use backend::Vault;
pub use worker::VaultService;

#[cfg(test)]
mod tests;
