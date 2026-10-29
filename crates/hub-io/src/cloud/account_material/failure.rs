//! Opaque foreign error ownership without uncharged shared-cause extraction.
use super::AccountMaterialError;
use nocterm_core::{
    SharedCause,
    allocation::{Allocated, AllocationLease},
};
use nocterm_hubs_model::HubError;
use std::{error::Error, fmt};

/// The original typed cause and its reservation stay inseparable. Borrowed source
/// traversal remains available; every publicly cloneable SharedCause owns a guard.
#[derive(Debug)]
pub struct AccountMaterialFailure {
    retained: Allocated<FailureKind>,
}
#[derive(Debug)]
enum FailureKind {
    Account(HubError),
    Backend(SharedCause),
    Rejected(Box<AccountMaterialError>),
}
#[derive(Debug)]
struct ResidentCause {
    cause: SharedCause,
    _allocation: AllocationLease,
}
impl AccountMaterialFailure {
    pub(super) fn new(error: AccountMaterialError, allocation: AllocationLease) -> Self {
        let kind = match error {
            AccountMaterialError::Account(error) => {
                FailureKind::Account(account_cause(error, &allocation))
            }
            AccountMaterialError::Backend(cause) => {
                FailureKind::Backend(retain_cause(cause, allocation.clone()))
            }
            rejected => FailureKind::Rejected(Box::new(rejected)),
        };
        Self {
            retained: Allocated::new(kind, allocation),
        }
    }
}
impl fmt::Display for AccountMaterialFailure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self.retained.value(), f)
    }
}
impl Error for AccountMaterialFailure {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        Some(self.retained.value())
    }
}
impl fmt::Display for FailureKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Account(error) => fmt::Display::fmt(error, f),
            Self::Backend(_) => f.write_str("Cloud material authentication or transport failed"),
            Self::Rejected(error) => fmt::Display::fmt(error, f),
        }
    }
}
impl Error for FailureKind {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Account(error) => Some(error),
            Self::Backend(cause) => Some(cause),
            Self::Rejected(error) => Some(error.as_ref()),
        }
    }
}
impl fmt::Display for ResidentCause {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&self.cause, f)
    }
}
impl Error for ResidentCause {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        Some(self.cause.as_ref())
    }
}
fn retain_cause(cause: SharedCause, allocation: AllocationLease) -> SharedCause {
    SharedCause::new(ResidentCause {
        cause,
        _allocation: allocation,
    })
}
fn account_cause(error: HubError, allocation: &AllocationLease) -> HubError {
    match error {
        HubError::Unreachable { address, cause } => HubError::Unreachable {
            address,
            cause: retain_cause(cause, allocation.clone()),
        },
        HubError::KeysetDamaged { cause } => HubError::KeysetDamaged {
            cause: retain_cause(cause, allocation.clone()),
        },
        HubError::Credentials { cause } => HubError::Credentials {
            cause: retain_cause(cause, allocation.clone()),
        },
        HubError::Remote { cause } => HubError::Remote {
            cause: retain_cause(cause, allocation.clone()),
        },
        error => error,
    }
}
