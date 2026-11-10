//! A hub window's availability gate for new Local chat actions.
//! Native source identity remains authoritative; UI status can only withdraw access.
use nocterm_hubs_model::{
    HubError,
    source_identity::{
        LocalAccountPartition, LocalAccountSource, LocalAccountSourceError,
        LocalAccountSourceFactory,
    },
};
use std::sync::{
    Arc, Mutex, PoisonError,
    atomic::{AtomicBool, Ordering},
};

pub(super) struct AccountCapture(Arc<nocterm_hub_io::HubAccount>);
impl AccountCapture {
    pub(super) fn new(account: Arc<nocterm_hub_io::HubAccount>) -> Self {
        Self(account)
    }
}
impl LocalAccountSourceFactory for AccountCapture {
    fn capture(&self) -> Result<Arc<dyn LocalAccountSource>, HubError> {
        self.0.local_source()
    }
}

struct ActiveSource {
    source: Arc<dyn LocalAccountSource>,
    available: Arc<AtomicBool>,
}
#[derive(Default)]
struct AvailabilityState {
    allowed: bool,
    active: Option<ActiveSource>,
}
pub(super) struct AgentSourceAvailability {
    native: Arc<dyn LocalAccountSourceFactory>,
    state: Mutex<AvailabilityState>,
}
impl AgentSourceAvailability {
    pub(super) fn new(native: Arc<dyn LocalAccountSourceFactory>) -> Self {
        Self {
            native,
            state: Mutex::new(AvailabilityState::default()),
        }
    }
    pub(super) fn native_current(&self) -> bool {
        self.state
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .active
            .as_ref()
            .is_some_and(|active| active.source.check_current().is_ok())
    }
    /// Notices trigger rechecking the original native owner, never become a generation.
    pub(super) fn refresh(&self, allowed: bool) -> bool {
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        state.allowed = allowed;
        if allowed
            && state
                .active
                .as_ref()
                .is_some_and(|active| active.source.check_current().is_ok())
        {
            return false;
        }
        let replacement = if allowed {
            self.native.capture().ok()
        } else {
            None
        };
        let changed = state.active.is_some() || replacement.is_some();
        if let Some(previous) = state.active.take() {
            previous.available.store(false, Ordering::Release);
        }
        state.active = replacement.map(|source| ActiveSource {
            source,
            available: Arc::new(AtomicBool::new(true)),
        });
        changed
    }
}
impl LocalAccountSourceFactory for AgentSourceAvailability {
    fn capture(&self) -> Result<Arc<dyn LocalAccountSource>, HubError> {
        if !self
            .state
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .allowed
        {
            return Err(HubError::NotSignedIn);
        }
        let state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(active) = state
            .active
            .as_ref()
            .filter(|active| active.source.check_current().is_ok())
        {
            return Ok(Arc::new(AvailableSource {
                native: active.source.clone(),
                available: active.available.clone(),
            }));
        }
        drop(state);
        // Only Follow can replace the owner together with its account mirrors.
        // Preserve a native failure; a newly ready owner waits for that composition
        // transition rather than reading the previous account's directory.
        self.native
            .capture()
            .and_then(|_| Err(HubError::KeysLocked))
    }
}
struct AvailableSource {
    native: Arc<dyn LocalAccountSource>,
    available: Arc<AtomicBool>,
}
impl LocalAccountSource for AvailableSource {
    fn partition(&self) -> LocalAccountPartition {
        self.native.partition()
    }
    fn check_current(&self) -> Result<(), LocalAccountSourceError> {
        if !self.available.load(Ordering::Acquire) {
            return Err(LocalAccountSourceError::Stale);
        }
        self.native.check_current()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nocterm_hubs_model::{
        source_identity::{AccountIdentity, AccountScope, Fingerprint, HubIdentity, NamespaceId},
        testing::ScriptedLocalAccountFactory,
    };
    fn partition(name: &str) -> LocalAccountPartition {
        LocalAccountPartition::Stable(AccountScope::new(
            HubIdentity::new(
                NamespaceId::new("hub").unwrap(),
                Fingerprint::from_bytes([1; 32]),
            ),
            AccountIdentity::new(
                NamespaceId::new(name).unwrap(),
                Fingerprint::from_bytes([2; 32]),
            ),
        ))
    }
    #[test]
    fn unlock_and_unrelated_notices_preserve_the_current_source() {
        let factory = Arc::new(ScriptedLocalAccountFactory::default());
        let available = AgentSourceAvailability::new(factory.clone());
        assert!(!available.refresh(true));
        assert!(matches!(available.capture(), Err(HubError::KeysLocked)));
        factory.sign_in(partition("alice"));
        assert!(available.refresh(true));
        let original = available.capture().unwrap();
        assert!(
            !available.refresh(true),
            "an unrelated notification does not rotate a source"
        );
        assert!(original.check_current().is_ok());
    }
    #[test]
    fn a_b_a_capture_waits_for_mirror_rebinding_and_never_revives_old_handles() {
        let factory = Arc::new(ScriptedLocalAccountFactory::default());
        let available = AgentSourceAvailability::new(factory.clone());
        factory.sign_in(partition("alice"));
        available.refresh(true);
        let original = available.capture().unwrap();
        factory.sign_in(partition("bob"));
        assert!(
            original.check_current().is_err(),
            "native eviction is synchronous before UI refresh"
        );
        assert!(
            available.capture().is_err(),
            "new actions wait for mirror rebinding"
        );
        assert!(available.refresh(true));
        let replacement = available.capture().unwrap();
        assert_eq!(replacement.partition(), partition("bob"));
        factory.sign_in(partition("alice"));
        assert!(available.capture().is_err());
        assert!(available.refresh(true));
        let returned = available.capture().unwrap();
        assert_eq!(returned.partition(), original.partition());
        assert!(original.check_current().is_err());
        assert!(replacement.check_current().is_err());
        assert!(returned.check_current().is_ok());
        assert!(available.refresh(false));
        assert!(returned.check_current().is_err());
        assert!(matches!(available.capture(), Err(HubError::NotSignedIn)));
    }
}
