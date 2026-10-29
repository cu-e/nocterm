//! Membership ownership is independent from the fake's displayed key action receipts.
use super::{State, lock};
use crate::{HubError, HubId, LoginOwner, LoginReceipt};
use std::sync::{Arc, Mutex, Weak};
struct Owner {
    state: Weak<Mutex<State>>,
    hub: HubId,
    member: Arc<()>,
}
impl Owner {
    fn with_current<T>(
        &self,
        action: impl FnOnce(&mut State) -> Result<T, HubError>,
    ) -> Result<T, HubError> {
        let shared = self.state.upgrade().ok_or(HubError::Stale)?;
        let mut state = lock(&shared);
        state.receipts.check_member(&self.hub, &self.member)?;
        if !state.credentials.contains_key(&self.hub) {
            return Err(HubError::Stale);
        }
        action(&mut state)
    }
}
impl LoginOwner for Owner {
    fn check_current(&self) -> Result<(), HubError> {
        self.with_current(|_| Ok(()))
    }
    fn discard(&self) -> Result<(), HubError> {
        self.with_current(|state| state.discard(&self.hub))
    }
}
pub(super) fn receipt(shared: Weak<Mutex<State>>, state: &mut State, hub: &HubId) -> LoginReceipt {
    LoginReceipt::new(
        hub.clone(),
        Arc::new(Owner {
            state: shared,
            hub: hub.clone(),
            member: state.receipts.member(hub),
        }),
    )
}
