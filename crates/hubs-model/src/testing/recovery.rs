//! Original recipient and action identities for the shared fake hub.
use super::{FakeCall, FakeHub, FakeOp, State, lock};
use crate::{
    HubError, HubFuture, HubId, RecoveryKeyConfirmation, RecoveryKeyIssuer, RecoveryKeyOffer,
    RecoveryKeyReissue,
};
use nocterm_session::Secret;
use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex, Weak},
};

#[derive(Default)]
pub(super) struct Receipts {
    actions: BTreeMap<HubId, Arc<()>>,
    members: BTreeMap<HubId, Arc<()>>,
}
impl Receipts {
    pub(super) fn begin(&mut self, hub: &HubId) {
        self.members.insert(hub.clone(), Arc::new(()));
        self.rotate(hub);
    }
    pub(super) fn member(&mut self, hub: &HubId) -> Arc<()> {
        self.members.entry(hub.clone()).or_default().clone()
    }
    pub(super) fn check_member(&self, hub: &HubId, expected: &Arc<()>) -> Result<(), HubError> {
        if self
            .members
            .get(hub)
            .is_some_and(|now| Arc::ptr_eq(now, expected))
        {
            Ok(())
        } else {
            Err(HubError::Stale)
        }
    }
    pub(super) fn rotate(&mut self, hub: &HubId) {
        self.actions.insert(hub.clone(), Arc::new(()));
    }
    pub(super) fn current(&mut self, hub: &HubId) -> Arc<()> {
        self.actions.entry(hub.clone()).or_default().clone()
    }
    fn check(&self, hub: &HubId, expected: &Arc<()>) -> Result<(), HubError> {
        if self
            .actions
            .get(hub)
            .is_some_and(|now| Arc::ptr_eq(now, expected))
        {
            Ok(())
        } else {
            Err(HubError::Stale)
        }
    }
}
#[derive(Clone)]
pub(super) struct Recipient {
    shared: Weak<Mutex<State>>,
    hub: HubId,
    receipt: Arc<()>,
}
impl Recipient {
    pub(super) fn capture(shared: Weak<Mutex<State>>, state: &mut State, hub: &HubId) -> Self {
        Self {
            shared,
            hub: hub.clone(),
            receipt: state.receipts.current(hub),
        }
    }
    pub(super) fn check(&self, state: &State) -> Result<(), HubError> {
        state.receipts.check(&self.hub, &self.receipt)?;
        if state.locked.contains(&self.hub) {
            return Err(HubError::KeysLocked);
        }
        Ok(())
    }
    pub(super) fn with_current<T>(
        &self,
        action: impl FnOnce(&mut State) -> Result<T, HubError>,
    ) -> Result<T, HubError> {
        let shared = self.shared.upgrade().ok_or(HubError::Stale)?;
        let mut state = lock(&shared);
        self.check(&state)?;
        action(&mut state)
    }
    pub(super) fn issuer(&self) -> RecoveryKeyReissue {
        RecoveryKeyReissue::new(self.hub.clone(), Arc::new(self.clone()))
    }
    pub(super) fn offer(&self, key: Secret) -> RecoveryKeyOffer {
        RecoveryKeyOffer::new(self.hub.clone(), key, Arc::new(self.clone()))
    }
}
impl RecoveryKeyConfirmation for Recipient {
    fn check_current(&self) -> Result<(), HubError> {
        self.with_current(|_| Ok(()))
    }
    fn confirm(&self) -> Result<(), HubError> {
        self.with_current(|state| {
            state.unconfirmed.remove(&self.hub);
            state.receipts.rotate(&self.hub);
            state.keys_changed();
            Ok(())
        })
    }
}
impl RecoveryKeyIssuer for Recipient {
    fn check_current(&self) -> Result<(), HubError> {
        RecoveryKeyConfirmation::check_current(self)
    }
    fn issue(&self) -> HubFuture<RecoveryKeyOffer> {
        let Some(shared) = self.shared.upgrade() else {
            return Box::pin(async { Err(HubError::Stale) });
        };
        if let Err(error) = self.with_current(|_| Ok(())) {
            return Box::pin(async move { Err(error) });
        }
        let original = self.clone();
        let fake = FakeHub {
            state: shared.clone(),
        };
        let action = fake.call(
            FakeOp::ReissueRecoveryKey,
            FakeCall::ReissueRecoveryKey(self.hub.clone()),
            move |state| {
                original.check(state)?;
                state.authenticate(&original.hub)?;
                let username = state.credentials[&original.hub].clone();
                if state.recovering.contains_key(&original.hub) {
                    return Err(HubError::KeysLocked);
                }
                let keyset = state
                    .servers
                    .get_mut(original.hub.address())
                    .and_then(|server| server.keysets.get_mut(&username))
                    .ok_or(HubError::KeysLocked)?;
                keyset.recovery.push('+');
                let key = Secret::new(keyset.recovery.clone());
                state.unconfirmed.insert(original.hub.clone());
                state.receipts.rotate(&original.hub);
                Ok(Recipient::capture(Arc::downgrade(&shared), state, &original.hub).offer(key))
            },
        );
        Box::pin(async move {
            let shown = action.await?;
            shown.check_current()?;
            Ok(shown)
        })
    }
}
