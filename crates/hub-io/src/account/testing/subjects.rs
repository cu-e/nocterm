//! One shared fake of the non-I/O account-subject reader, without a hub service.
use crate::account::Slot;
use crate::account::source::{AccountSubject, AccountSubjects};
use nocterm_hubs_model::{HubError, HubId};
use std::{
    collections::HashMap,
    sync::{Arc, Mutex, Weak},
};

#[derive(Default)]
pub(crate) struct ScriptedSubjects {
    subjects: Mutex<HashMap<HubId, Arc<AccountSubject>>>,
    slot: Option<Weak<Slot>>,
}
impl ScriptedSubjects {
    pub(crate) fn for_slot(slot: &Arc<Slot>) -> Self {
        Self {
            subjects: Mutex::default(),
            slot: Some(Arc::downgrade(slot)),
        }
    }

    fn begin(&self, hub: &HubId, subject: &Arc<AccountSubject>) {
        if let Some(slot) = self.slot.as_ref().and_then(Weak::upgrade)
            && slot.hub() == hub
        {
            slot.begin_membership(subject);
        }
    }

    pub(crate) fn sign_in(&self, hub: HubId, account: &str, device: &str) {
        let mut subjects = self.subjects.lock().unwrap();
        let subject = Arc::new(AccountSubject::new(account.into(), device.into()));
        self.begin(&hub, &subject);
        subjects.insert(hub, subject);
    }
    pub(crate) fn renew(&self, hub: HubId, account: &str, device: &str) {
        let mut subjects = self.subjects.lock().unwrap();
        let next = AccountSubject::new(account.into(), device.into());
        if subjects
            .get(&hub)
            .is_some_and(|previous| previous.as_ref() == &next)
        {
            return;
        }
        let next = Arc::new(next);
        self.begin(&hub, &next);
        subjects.insert(hub, next);
    }
    pub(crate) fn evict(&self, hub: &HubId) {
        self.subjects.lock().unwrap().remove(hub);
    }
}
impl AccountSubjects for ScriptedSubjects {
    fn with_current(
        &self,
        hub: &HubId,
        expected: &Arc<AccountSubject>,
        action: &mut dyn FnMut() -> Result<(), HubError>,
    ) -> Result<(), HubError> {
        let subjects = self.subjects.lock().unwrap();
        if !subjects
            .get(hub)
            .is_some_and(|current| Arc::ptr_eq(current, expected))
        {
            return Err(HubError::Stale);
        }
        action()
    }

    fn current(&self, hub: &HubId) -> Option<Arc<AccountSubject>> {
        self.subjects.lock().unwrap().get(hub).cloned()
    }
}
