//! Fingerprint verification as a pipeline over a [`FingerprintBackend`]:
//! attempt accounting, scanner exclusivity, claim with bounded cleanup and
//! the scan loop under one deadline.
use super::{
    denied,
    fingerprint::{
        FingerprintBackend, FingerprintDevice, ScanStatus, VerifyStatuses, verification_status,
    },
};
use crate::store::{MAX_ATTEMPTS, Owner, Store};
use std::{
    sync::atomic::{AtomicBool, Ordering},
    time::Duration,
};
use tokio::{sync::Mutex, time::Instant};
use zbus::fdo;

/// Bounds discovery, claim and scanning together, whatever the scan deadline.
const SERVICE_LIMIT: Duration = Duration::from_secs(35);
/// Bounds each cleanup call, so a hung reader cannot keep the broker waiting.
pub(super) const CLEANUP_LIMIT: Duration = Duration::from_secs(1);
/// How often a scan checks for cancellation and a departed client.
const WATCHDOG: Duration = Duration::from_millis(50);
const LOCKED: &str = "Fingerprint unlock is locked after 3 failed attempts";

/// The registration whose attempts a verification spends.
pub(super) struct Attempts<'a> {
    pub store: &'a Mutex<Store>,
    pub owner: &'a Owner,
    pub binding: &'a str,
    pub token: &'a str,
}

impl Attempts<'_> {
    async fn ensure_unlocked(&self) -> fdo::Result<()> {
        let mut store = self.store.lock().await;
        let entry = store
            .entry(self.owner, self.binding, self.token)
            .map_err(denied)?;
        if entry.attempts_remaining == 0 {
            return Err(denied(LOCKED));
        }
        Ok(())
    }

    /// Persists the outcome of a completed scan before anything else happens.
    async fn record(&self, matched: bool) -> fdo::Result<Next> {
        let mut store = self.store.lock().await;
        let entry = store
            .entry(self.owner, self.binding, self.token)
            .map_err(denied)?;
        let (remaining, next) = after_scan(entry.attempts_remaining, matched);
        entry.attempts_remaining = remaining;
        Ok(next)
    }
}

#[derive(Debug, PartialEq, Eq)]
pub(super) enum Next {
    Accept,
    Retry,
    Locked,
}

/// The attempts left after a completed scan, and what follows it.
pub(super) fn after_scan(remaining: u8, matched: bool) -> (u8, Next) {
    if matched {
        return (MAX_ATTEMPTS, Next::Accept);
    }
    match remaining.saturating_sub(1) {
        0 => (0, Next::Locked),
        left => (left, Next::Retry),
    }
}

/// One verification request.
pub(super) struct Request<'a> {
    pub attempts: Attempts<'a>,
    /// Held for the whole verification: one reader serves one request.
    pub scanner: &'a Mutex<()>,
    pub cancel: &'a AtomicBool,
    /// Covers every scan and the setup between them.
    pub scan_timeout: Duration,
}

impl Request<'_> {
    fn ensure_active(&self) -> fdo::Result<()> {
        if self.cancel.load(Ordering::SeqCst) {
            return Err(denied("Authentication cancelled"));
        }
        Ok(())
    }
}

/// Verifies a fingerprint for `request`, spending its attempts.
pub(super) async fn verify<B: FingerprintBackend>(
    backend: &B,
    request: &Request<'_>,
) -> fdo::Result<()> {
    request.attempts.ensure_unlocked().await?;
    let _scanner = request
        .scanner
        .try_lock()
        .map_err(|_| fdo::Error::Failed("Fingerprint reader is busy".into()))?;
    let service = Instant::now() + SERVICE_LIMIT;
    let device = within(service, backend.device(request.attempts.owner)).await?;
    with_claim(&device, service, scan(backend, &device, request)).await
}

async fn within<T>(
    deadline: Instant,
    work: impl Future<Output = fdo::Result<T>>,
) -> fdo::Result<T> {
    tokio::time::timeout_at(deadline, work)
        .await
        .unwrap_or_else(|_| Err(denied("Fingerprint service timed out")))
}

/// The result of work done on a claimed reader.
pub(super) struct Claimed {
    pub result: fdo::Result<()>,
    /// A reader that reported disconnection is not stopped or released.
    pub reader_gone: bool,
}

/// Claims `device`, runs `body`, then stops and releases the reader on every
/// outcome of `body`, each call bounded by [`CLEANUP_LIMIT`]. A failed claim
/// has nothing to clean up.
pub(super) async fn with_claim<D: FingerprintDevice>(
    device: &D,
    service: Instant,
    body: impl Future<Output = Claimed>,
) -> fdo::Result<()> {
    within(service, device.claim()).await?;
    let claimed = tokio::time::timeout_at(service, body)
        .await
        .unwrap_or_else(|_| Claimed {
            result: Err(denied("Fingerprint service timed out")),
            reader_gone: false,
        });
    if !claimed.reader_gone {
        let _ = tokio::time::timeout(CLEANUP_LIMIT, device.verify_stop()).await;
        let _ = tokio::time::timeout(CLEANUP_LIMIT, device.release()).await;
    }
    claimed.result
}

async fn scan<B: FingerprintBackend>(
    backend: &B,
    device: &B::Device,
    request: &Request<'_>,
) -> Claimed {
    let mut reader_gone = false;
    let result = scan_until_decided(backend, device, request, &mut reader_gone).await;
    Claimed {
        result,
        reader_gone,
    }
}

/// Scans until a match, lockout, fault or the deadline.
async fn scan_until_decided<B: FingerprintBackend>(
    backend: &B,
    device: &B::Device,
    request: &Request<'_>,
    reader_gone: &mut bool,
) -> fdo::Result<()> {
    // One deadline covers all scans and retry setup. Claim separates owners;
    // each new subscription separates completed attempts.
    let deadline = tokio::time::sleep(request.scan_timeout);
    tokio::pin!(deadline);
    let timed_out = || denied("Fingerprint authentication timed out");
    loop {
        let matched = tokio::select! {
            () = &mut deadline => return Err(timed_out()),
            result = scan_once(backend, device, request, reader_gone) => result?,
        };
        match request.attempts.record(matched).await? {
            Next::Accept => return Ok(()),
            Next::Locked => return Err(denied(LOCKED)),
            Next::Retry => {}
        }
        // The old stream is dropped before stopping. Subscribe afresh only
        // after VerifyStop finishes, excluding stale completion signals.
        tokio::select! {
            () = &mut deadline => return Err(timed_out()),
            result = device.verify_stop() => result?,
        }
    }
}

/// One scan: whether the finger matched.
async fn scan_once<B: FingerprintBackend>(
    backend: &B,
    device: &B::Device,
    request: &Request<'_>,
    reader_gone: &mut bool,
) -> fdo::Result<bool> {
    request.ensure_active()?;
    let mut statuses = device.verify_start().await?;
    loop {
        tokio::select! {
            () = tokio::time::sleep(WATCHDOG) => {
                request.ensure_active()?;
                if !backend.client_present(request.attempts.owner).await {
                    return Err(denied("Client disconnected"));
                }
            }
            status = statuses.next() => {
                let (status, done) = status?;
                request.ensure_active()?;
                *reader_gone |= status == "verify-disconnected";
                match verification_status(&status, done)? {
                    ScanStatus::Pending => {}
                    ScanStatus::Match => return Ok(true),
                    ScanStatus::NoMatch => return Ok(false),
                }
            }
        }
    }
}

#[cfg(test)]
mod tests;
