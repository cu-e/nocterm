use super::*;
use crate::{
    DevicePresence, LeaseServices, MAX_PROVIDER_LEASE, RenewalError, RenewalPolicy, RenewalSetup,
};
use std::{
    sync::atomic::{AtomicBool, AtomicUsize, Ordering},
    time::Duration,
};

const MINUTE: Duration = Duration::from_secs(60);

/// A scripted owner presence; the paused tokio clock stands in for real time.
struct Presence {
    online: AtomicBool,
    withdrawn: AtomicBool,
    renewals: AtomicUsize,
}
impl Presence {
    fn new(online: bool) -> Arc<Self> {
        Arc::new(Self {
            online: AtomicBool::new(online),
            withdrawn: AtomicBool::new(false),
            renewals: AtomicUsize::new(0),
        })
    }
}
impl DevicePresence for Presence {
    fn any_device_online(&self) -> bool {
        self.online.load(Ordering::SeqCst)
    }
    fn renew(&self, _: Duration) -> BoxFuture<'static, Result<(), RenewalError>> {
        let result = if self.withdrawn.load(Ordering::SeqCst) {
            Err(RenewalError::Withdrawn)
        } else if self.online.load(Ordering::SeqCst) {
            self.renewals.fetch_add(1, Ordering::SeqCst);
            Ok(())
        } else {
            Err(RenewalError::Offline)
        };
        futures::future::ready(result).boxed()
    }
}

fn renewed_lease(presence: &Arc<Presence>) -> ProviderLease {
    let lease = ProviderLease::new(
        binding(ProviderKind::OpenAiChat, "http://127.0.0.1:9"),
        Zeroizing::new(KEY.into()),
        MAX_PROVIDER_LEASE,
    )
    .unwrap();
    let setup = RenewalSetup::new(RenewalPolicy::standard(), presence.clone());
    lease.attach(&LeaseServices::default().renewing(setup));
    lease
}

#[tokio::test(start_paused = true)]
async fn lease_outlives_the_single_step_cap_while_a_device_is_online() {
    let presence = Presence::new(true);
    let lease = renewed_lease(&presence);
    for _ in 0..8 {
        tokio::time::sleep(10 * MINUTE).await;
        assert!(lease.is_available());
        assert!(lease.remaining().unwrap() <= MAX_PROVIDER_LEASE);
    }
    assert!(presence.renewals.load(Ordering::SeqCst) >= 2);
    assert!(!format!("{lease:?}").contains(KEY));
}

#[tokio::test(start_paused = true)]
async fn lease_lapses_at_its_step_when_no_device_is_online() {
    let presence = Presence::new(false);
    let lease = renewed_lease(&presence);
    tokio::time::sleep(MAX_PROVIDER_LEASE - MINUTE).await;
    assert!(lease.is_available());
    tokio::time::sleep(2 * MINUTE).await;
    assert!(!lease.is_available());
    assert_eq!(presence.renewals.load(Ordering::SeqCst), 0);
}

#[tokio::test(start_paused = true)]
async fn renewal_resumes_when_a_device_returns_before_the_lease_ends() {
    let presence = Presence::new(false);
    let lease = renewed_lease(&presence);
    tokio::time::sleep(27 * MINUTE).await;
    presence.online.store(true, Ordering::SeqCst);
    tokio::time::sleep(10 * MINUTE).await;
    assert!(lease.is_available());
    assert_eq!(presence.renewals.load(Ordering::SeqCst), 1);
}

#[tokio::test(start_paused = true)]
async fn stopping_renewal_lets_the_lease_lapse_at_its_current_deadline() {
    let presence = Presence::new(true);
    let lease = renewed_lease(&presence);
    tokio::time::sleep(10 * MINUTE).await;
    lease.stop_renewal();
    tokio::time::sleep(MAX_PROVIDER_LEASE).await;
    assert!(!lease.is_available());
    assert_eq!(presence.renewals.load(Ordering::SeqCst), 0);
}

#[tokio::test(start_paused = true)]
async fn withdrawn_access_ends_renewal_and_the_lease_lapses() {
    let presence = Presence::new(true);
    presence.withdrawn.store(true, Ordering::SeqCst);
    let lease = renewed_lease(&presence);
    tokio::time::sleep(MAX_PROVIDER_LEASE + MINUTE).await;
    assert!(!lease.is_available());
}

#[tokio::test(start_paused = true)]
async fn revoking_a_lease_stops_its_renewal() {
    let presence = Presence::new(true);
    let lease = renewed_lease(&presence);
    lease.revoke();
    tokio::time::sleep(2 * MAX_PROVIDER_LEASE).await;
    assert!(!lease.is_available());
    assert_eq!(presence.renewals.load(Ordering::SeqCst), 0);
}
