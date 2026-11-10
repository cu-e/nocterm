//! Nonpersistent native membership and key-lifecycle publication receipts.
use super::source::AccountSubject;
use nocterm_hubs_model::HubError;
use std::sync::{Arc, Weak};

#[derive(Clone)]
pub(crate) struct SlotReceipt(Arc<()>);

impl SlotReceipt {
    fn fresh() -> Self {
        Self(Arc::new(()))
    }
    fn matches(&self, expected: &Self) -> bool {
        Arc::ptr_eq(&self.0, &expected.0)
    }
}

pub(super) struct SlotAuthority {
    owner: Option<Weak<AccountSubject>>,
    receipt: SlotReceipt,
}

impl SlotAuthority {
    pub(super) fn new() -> Self {
        Self {
            owner: None,
            receipt: SlotReceipt::fresh(),
        }
    }
    pub(super) fn begin(&mut self, owner: &Arc<AccountSubject>) {
        self.owner = Some(Arc::downgrade(owner));
        self.rotate();
    }
    pub(super) fn rotate(&mut self) -> SlotReceipt {
        self.receipt = SlotReceipt::fresh();
        self.receipt.clone()
    }
    pub(super) fn receipt(&self, owner: &Arc<AccountSubject>) -> Result<SlotReceipt, HubError> {
        self.check_owner(owner)?;
        Ok(self.receipt.clone())
    }
    pub(super) fn observed(&self) -> SlotReceipt {
        self.receipt.clone()
    }
    pub(super) fn check_observed(&self, receipt: &SlotReceipt) -> Result<(), HubError> {
        if self.receipt.matches(receipt) {
            Ok(())
        } else {
            Err(HubError::Stale)
        }
    }
    pub(super) fn check_owner(&self, expected: &Arc<AccountSubject>) -> Result<(), HubError> {
        if self
            .owner
            .as_ref()
            .and_then(Weak::upgrade)
            .is_some_and(|current| Arc::ptr_eq(&current, expected))
        {
            Ok(())
        } else {
            Err(HubError::Stale)
        }
    }
    pub(super) fn check(
        &self,
        owner: &Arc<AccountSubject>,
        receipt: &SlotReceipt,
    ) -> Result<(), HubError> {
        self.check_owner(owner)?;
        if self.receipt.matches(receipt) {
            Ok(())
        } else {
            Err(HubError::Stale)
        }
    }
}
