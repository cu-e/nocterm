//! The fingerprint reader as the verification pipeline sees it.
//!
//! [`super::fprintd`] adapts fprintd over D-Bus; tests script a reader
//! without a bus.
use super::denied;
use crate::store::Owner;
use zbus::fdo;

/// Finds readers for a desktop user and watches the client that asked.
pub(super) trait FingerprintBackend: Send + Sync {
    type Device: FingerprintDevice;
    /// A trusted reader with fingers enrolled for `owner`, not yet claimed.
    fn device(&self, owner: &Owner) -> impl Future<Output = fdo::Result<Self::Device>> + Send;
    /// Whether the client that asked for the scan is still connected.
    fn client_present(&self, owner: &Owner) -> impl Future<Output = bool> + Send;
}

/// One reader. Claim separates owners; each scan subscribes afresh.
pub(super) trait FingerprintDevice: Send + Sync {
    type Statuses: VerifyStatuses;
    fn claim(&self) -> impl Future<Output = fdo::Result<()>> + Send;
    /// Subscribes to statuses, then starts a scan, so completions of earlier
    /// scans never reach the returned stream.
    fn verify_start(&self) -> impl Future<Output = fdo::Result<Self::Statuses>> + Send;
    fn verify_stop(&self) -> impl Future<Output = fdo::Result<()>> + Send;
    fn release(&self) -> impl Future<Output = fdo::Result<()>> + Send;
}

/// The statuses of one scan, as `(status, done)` from a trusted sender.
pub(super) trait VerifyStatuses: Send {
    fn next(&mut self) -> impl Future<Output = fdo::Result<(String, bool)>> + Send;
}

#[derive(Debug, PartialEq, Eq)]
pub(super) enum ScanStatus {
    Pending,
    Match,
    NoMatch,
}

/// What one fprintd status means for the scan in progress.
pub(super) fn verification_status(status: &str, done: bool) -> fdo::Result<ScanStatus> {
    match status {
        "verify-match" if done => Ok(ScanStatus::Match),
        "verify-no-match" if done => Ok(ScanStatus::NoMatch),
        "verify-disconnected" => Err(denied("Fingerprint reader disconnected")),
        "verify-unknown-error" => Err(denied("Fingerprint reader reported an unknown error")),
        _ if !done => Ok(ScanStatus::Pending),
        "verify-retry-scan" => Err(denied("Fingerprint scan must be retried")),
        "verify-swipe-too-short" => Err(denied("Fingerprint swipe was too short")),
        "verify-finger-not-centered" => Err(denied("Finger was not centered on the reader")),
        "verify-remove-and-retry" => Err(denied("Remove your finger and try again")),
        "verify-too-fast" => Err(denied("Finger moved too quickly")),
        _ => Err(denied(format!("Fingerprint verification failed: {status}"))),
    }
}
