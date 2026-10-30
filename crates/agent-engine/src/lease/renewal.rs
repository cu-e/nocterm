//! Background renewal of a provider lease while a device of its owner is online.
//!
//! The lease keeps its finite step cap: [`MAX_PROVIDER_LEASE`] bounds one
//! renewal step, not the lifetime of a chat. The decision rules are the pure
//! [`RenewalPolicy`]; [`LeaseRenewal`] only applies them with the host timer.
use super::{LeaseOwner, MAX_PROVIDER_LEASE, ProviderLease};
use futures::future::BoxFuture;
use std::{
    sync::{Arc, Weak},
    time::Duration,
};

/// Why a renewal step was not granted.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum RenewalError {
    /// No device could confirm the step right now; asking again later may work.
    #[error("no device is online to authorise the renewal")]
    Offline,
    /// The owner withdrew access; the lease must lapse and never be asked again.
    #[error("the owner withdrew access to the provider")]
    Withdrawn,
}

/// An invalid renewal policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum RenewalPolicyError {
    #[error("the renewal step must be non-zero and at most the provider lease cap")]
    Step,
    #[error("the renewal margin must be non-zero and shorter than the step")]
    Margin,
    #[error("the poll interval must be non-zero and no longer than the margin")]
    Poll,
}

/// Whether a device of the owner is online, and the means to confirm one more
/// step. The host supplies the real source (hub presence); this crate owns none.
pub trait DevicePresence: Send + Sync {
    /// True while at least one device of the owner is online.
    fn any_device_online(&self) -> bool;
    /// Confirms one more step of `step` from a device of the owner.
    fn renew(&self, step: Duration) -> BoxFuture<'static, Result<(), RenewalError>>;
}

/// What the renewal loop does next.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RenewalDecision {
    /// Look again after this long.
    Wait(Duration),
    /// Ask the presence port for one more step of this length.
    Renew(Duration),
    /// The lease ran out; it can no longer be renewed.
    Lapse,
}

/// Pure renewal rules over the remaining life of a lease.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RenewalPolicy {
    step: Duration,
    margin: Duration,
    poll: Duration,
}
impl RenewalPolicy {
    pub fn new(
        step: Duration,
        margin: Duration,
        poll: Duration,
    ) -> Result<Self, RenewalPolicyError> {
        if step.is_zero() || step > MAX_PROVIDER_LEASE {
            return Err(RenewalPolicyError::Step);
        }
        if margin.is_zero() || margin >= step {
            return Err(RenewalPolicyError::Margin);
        }
        if poll.is_zero() || poll > margin {
            return Err(RenewalPolicyError::Poll);
        }
        Ok(Self { step, margin, poll })
    }
    /// A full-cap step renewed five minutes before it ends, polled every 15 s.
    pub fn standard() -> Self {
        Self {
            step: MAX_PROVIDER_LEASE,
            margin: Duration::from_secs(5 * 60),
            poll: Duration::from_secs(15),
        }
    }
    pub fn step(&self) -> Duration {
        self.step
    }
    pub fn decide(&self, remaining: Duration, device_online: bool) -> RenewalDecision {
        if remaining.is_zero() {
            return RenewalDecision::Lapse;
        }
        if remaining > self.margin {
            return RenewalDecision::Wait(remaining - self.margin);
        }
        if device_online {
            RenewalDecision::Renew(self.step)
        } else {
            RenewalDecision::Wait(self.poll.min(remaining))
        }
    }
}

/// A policy with the port that confirms steps.
#[derive(Clone)]
pub struct RenewalSetup {
    policy: RenewalPolicy,
    presence: Arc<dyn DevicePresence>,
}
impl RenewalSetup {
    pub fn new(policy: RenewalPolicy, presence: Arc<dyn DevicePresence>) -> Self {
        Self { policy, presence }
    }
}

/// The running renewal task of one lease. Dropping it stops renewal.
pub struct LeaseRenewal(tokio::task::AbortHandle);
impl LeaseRenewal {
    pub(super) fn start(
        runtime: &tokio::runtime::Handle,
        owner: Weak<LeaseOwner>,
        setup: &RenewalSetup,
    ) -> Self {
        let setup = setup.clone();
        Self(runtime.spawn(run(owner, setup)).abort_handle())
    }
}
impl Drop for LeaseRenewal {
    fn drop(&mut self) {
        self.0.abort();
    }
}

async fn run(owner: Weak<LeaseOwner>, setup: RenewalSetup) {
    loop {
        let Some(remaining) = owner
            .upgrade()
            .and_then(|owner| ProviderLease { owner }.remaining())
        else {
            return;
        };
        match setup
            .policy
            .decide(remaining, setup.presence.any_device_online())
        {
            RenewalDecision::Lapse => return,
            RenewalDecision::Wait(delay) => tokio::time::sleep(delay).await,
            RenewalDecision::Renew(step) => match setup.presence.renew(step).await {
                Ok(()) => {
                    let Some(owner) = owner.upgrade() else { return };
                    if (ProviderLease { owner }).extend(step).is_err() {
                        return;
                    }
                }
                Err(RenewalError::Offline) => tokio::time::sleep(setup.policy.poll).await,
                Err(RenewalError::Withdrawn) => return,
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    const MINUTE: Duration = Duration::from_secs(60);
    fn policy() -> RenewalPolicy {
        RenewalPolicy::new(30 * MINUTE, 5 * MINUTE, Duration::from_secs(15)).unwrap()
    }
    #[test]
    fn invalid_policies_are_refused() {
        let ok = Duration::from_secs(15);
        assert_eq!(
            RenewalPolicy::new(Duration::ZERO, 5 * MINUTE, ok),
            Err(RenewalPolicyError::Step)
        );
        assert_eq!(
            RenewalPolicy::new(MAX_PROVIDER_LEASE + ok, 5 * MINUTE, ok),
            Err(RenewalPolicyError::Step)
        );
        assert_eq!(
            RenewalPolicy::new(30 * MINUTE, 30 * MINUTE, ok),
            Err(RenewalPolicyError::Margin)
        );
        assert_eq!(
            RenewalPolicy::new(30 * MINUTE, 5 * MINUTE, 6 * MINUTE),
            Err(RenewalPolicyError::Poll)
        );
    }
    #[test]
    fn it_waits_until_the_margin_then_renews_one_step_while_online() {
        let policy = policy();
        assert_eq!(
            policy.decide(30 * MINUTE, true),
            RenewalDecision::Wait(25 * MINUTE)
        );
        assert_eq!(
            policy.decide(5 * MINUTE, true),
            RenewalDecision::Renew(30 * MINUTE)
        );
    }
    #[test]
    fn it_does_not_renew_offline_and_lapses_at_zero() {
        let policy = policy();
        assert_eq!(
            policy.decide(2 * MINUTE, false),
            RenewalDecision::Wait(Duration::from_secs(15))
        );
        assert_eq!(
            policy.decide(Duration::from_secs(3), false),
            RenewalDecision::Wait(Duration::from_secs(3))
        );
        assert_eq!(policy.decide(Duration::ZERO, true), RenewalDecision::Lapse);
    }
    #[test]
    fn the_step_never_exceeds_the_single_lease_cap() {
        assert_eq!(RenewalPolicy::standard().step(), MAX_PROVIDER_LEASE);
        assert!(RenewalPolicy::new(MAX_PROVIDER_LEASE, MINUTE, Duration::from_secs(1)).is_ok());
    }
}
