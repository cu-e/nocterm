//! What a known hub is doing, and what to make of the outcome of reaching it.

use std::time::{Duration, Instant};

use nocterm_hubs_model::{Fingerprint, HubError, HubInfo};

/// The state of one known hub, shared by every window.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HubStatus {
    /// Credentials are missing, so the user has to sign in.
    SignedOut,
    /// A connection is being made.
    Connecting,
    /// Connected with working credentials.
    Online,
    /// The hub did not answer; the next attempt starts at `retry_at`.
    Offline { retry_at: Instant },
    /// The hub revoked this device; its credentials are gone.
    Revoked,
    /// The hub's certificate is not the trusted one. Never retried, never accepted.
    TrustFailed {
        /// The pinned fingerprint; `None` when a public authority was trusted instead.
        pinned: Option<Fingerprint>,
        presented: Fingerprint,
    },
    /// The hub and this client speak different protocol versions.
    UpgradeRequired { hub: u32, client: u32 },
}

impl HubStatus {
    pub const fn is_online(&self) -> bool {
        matches!(self, Self::Online)
    }

    /// Signing in again is the way forward.
    pub const fn needs_sign_in(&self) -> bool {
        matches!(self, Self::SignedOut | Self::Revoked)
    }
}

/// What reaching a hub leads to.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Settled {
    /// A resting status.
    Status(HubStatus),
    /// The hub could not be reached for now; try again later.
    Retry,
}

/// Reads the outcome of connecting to a hub.
pub(crate) fn settle_connect(result: &Result<HubInfo, HubError>) -> Settled {
    match result {
        Ok(info) if info.signed_in => Settled::Status(HubStatus::Online),
        Ok(_) => Settled::Status(HubStatus::SignedOut),
        Err(error) => settle_failure(error).unwrap_or(Settled::Retry),
    }
}

/// Reads a failure of a call on a hub that was online; `None` leaves the status alone.
pub(crate) fn settle_failure(error: &HubError) -> Option<Settled> {
    Some(match error {
        HubError::Unreachable { .. } => Settled::Retry,
        HubError::CertificateChanged {
            pinned, presented, ..
        } => Settled::Status(HubStatus::TrustFailed {
            pinned: Some(*pinned),
            presented: *presented,
        }),
        HubError::CertificateUntrusted { presented, .. } => {
            Settled::Status(HubStatus::TrustFailed {
                pinned: None,
                presented: *presented,
            })
        }
        HubError::UpgradeRequired { hub, client } => Settled::Status(HubStatus::UpgradeRequired {
            hub: *hub,
            client: *client,
        }),
        HubError::NotSignedIn => Settled::Status(HubStatus::SignedOut),
        HubError::Revoked => Settled::Status(HubStatus::Revoked),
        // Credentials that cannot be kept or any other trouble of the hub client: the hub is
        // not usable now, and trying again later is the harmless answer.
        HubError::Credentials { .. } | HubError::Remote { .. } => Settled::Retry,
        HubError::InvalidCredentials
        | HubError::MethodUnavailable
        | HubError::SignInTimedOut
        | HubError::SignInCancelled
        | HubError::PermissionDenied
        | HubError::NotFound => return None,
    })
}

/// Waits between attempts to reach a hub that does not answer; the last one repeats.
const RETRY_DELAYS: [Duration; 5] = [
    Duration::from_secs(2),
    Duration::from_secs(5),
    Duration::from_secs(10),
    Duration::from_secs(30),
    Duration::from_secs(60),
];

/// How long to wait after `failures` unsuccessful attempts in a row (at least one).
pub(crate) fn retry_delay(failures: u32) -> Duration {
    let index = usize::try_from(failures.saturating_sub(1)).unwrap_or(usize::MAX);
    RETRY_DELAYS[index.min(RETRY_DELAYS.len() - 1)]
}

#[cfg(test)]
mod tests {
    use nocterm_core::error::SharedCause;
    use nocterm_hubs_model::HubAddress;

    use super::*;

    fn info(signed_in: bool) -> HubInfo {
        HubInfo {
            name: "Home".into(),
            password_login: true,
            oidc_providers: Vec::new(),
            bootstrapped: true,
            signed_in,
        }
    }

    fn unreachable() -> HubError {
        #[derive(Debug, thiserror::Error)]
        #[error("down")]
        struct Down;
        HubError::Unreachable {
            address: HubAddress::parse("hub.home").unwrap(),
            cause: SharedCause::new(Down),
        }
    }

    #[test]
    fn a_signed_in_device_is_online_and_an_unknown_one_is_signed_out() {
        assert_eq!(
            settle_connect(&Ok(info(true))),
            Settled::Status(HubStatus::Online)
        );
        assert_eq!(
            settle_connect(&Ok(info(false))),
            Settled::Status(HubStatus::SignedOut)
        );
    }

    #[test]
    fn an_unreachable_hub_is_retried_but_a_changed_certificate_never_is() {
        assert_eq!(settle_connect(&Err(unreachable())), Settled::Retry);
        let changed = HubError::CertificateChanged {
            address: HubAddress::parse("hub.home").unwrap(),
            pinned: Fingerprint::from_bytes([1; 32]),
            presented: Fingerprint::from_bytes([2; 32]),
        };
        assert_eq!(
            settle_connect(&Err(changed)),
            Settled::Status(HubStatus::TrustFailed {
                pinned: Some(Fingerprint::from_bytes([1; 32])),
                presented: Fingerprint::from_bytes([2; 32]),
            })
        );
    }

    #[test]
    fn revoked_and_outdated_hubs_rest_without_retries() {
        assert_eq!(
            settle_connect(&Err(HubError::Revoked)),
            Settled::Status(HubStatus::Revoked)
        );
        assert_eq!(
            settle_connect(&Err(HubError::UpgradeRequired { hub: 2, client: 1 })),
            Settled::Status(HubStatus::UpgradeRequired { hub: 2, client: 1 })
        );
        assert!(settle_failure(&HubError::PermissionDenied).is_none());
    }

    #[test]
    fn retries_back_off_and_the_last_delay_repeats() {
        let delays: Vec<u64> = (1..=7).map(|n| retry_delay(n).as_secs()).collect();
        assert_eq!(delays, [2, 5, 10, 30, 60, 60, 60]);
        assert_eq!(retry_delay(0).as_secs(), 2);
    }
}
