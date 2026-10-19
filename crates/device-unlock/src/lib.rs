//! Native key-release adapters, injected by the application composition root.
//!
//! Owns: platform registration, authenticated key release and native cancellation scopes.
//! Does not own: vault format, file publication, accepted key state or unlock views.
//! Ports provided: `DeviceUnlockProvider` from `nocterm-vault`.
//! Settings section: none. Globals: none.
//! Invariants: provider construction does not prompt; release checks binding and cancellation.
#[cfg(any(target_os = "macos", test, feature = "test-support"))]
mod cancellation;
#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "windows")]
mod windows;
use nocterm_vault::DeviceUnlockProvider;
use std::sync::Arc;
/// Creates the platform adapter without showing an authentication prompt.
pub fn provider() -> Arc<dyn DeviceUnlockProvider> {
    #[cfg(target_os = "linux")]
    {
        Arc::new(linux::Linux::default())
    }
    #[cfg(target_os = "macos")]
    {
        Arc::new(macos::MacOs)
    }
    #[cfg(target_os = "windows")]
    {
        Arc::new(windows::Windows)
    }
}
fn platform(error: impl std::fmt::Display) -> nocterm_vault::DeviceUnlockError {
    nocterm_vault::DeviceUnlockError::Platform(error.to_string())
}

#[cfg(all(test, any(target_os = "macos", target_os = "windows")))]
mod tests {
    use super::*;

    /// Retaining the provider's dynamic vtable forces a native executable to
    /// link the adapter methods, without evaluating policies or requesting keys.
    #[test]
    fn native_provider_constructs_without_authentication() {
        let provider = std::hint::black_box(provider());
        assert_eq!(Arc::strong_count(&provider), 1);
        let clone = std::hint::black_box(provider.clone());
        assert!(Arc::ptr_eq(&provider, &clone));
    }
}

#[cfg(any(test, feature = "test-support"))]
pub mod testing;
