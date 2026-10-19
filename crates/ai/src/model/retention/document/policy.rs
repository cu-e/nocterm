//! Pending semantic projections are promises, never native allocation receipts.
use super::{DocumentRetention, DocumentState, RetentionError, account::AccountIdentity};
use crate::model::json::lifetime::Shared;

/// Only closed domain certification can reserve this inseparable liability.
/// The surrounding envelope drops its actual payload before dropping this token.
pub(crate) struct PolicyLiability {
    account: Shared<AccountIdentity>,
    bytes: usize,
    pending: bool,
}
impl PolicyLiability {
    pub(super) fn new(scope: &DocumentRetention, bytes: usize) -> Self {
        Self {
            account: scope.1.clone(),
            bytes,
            pending: true,
        }
    }
    pub(super) fn checked_bytes(&self, scope: &DocumentRetention) -> Result<usize, RetentionError> {
        if !self.account.same(&scope.1) {
            return Err(RetentionError::ForeignAuthority);
        }
        if !self.pending {
            return Err(RetentionError::FrozenOwner);
        }
        Ok(self.bytes)
    }
    pub(super) fn consume(&mut self, state: &mut DocumentState) {
        state.pending_policy = state
            .pending_policy
            .checked_sub(self.bytes)
            .expect("authentic semantic handoff");
        self.pending = false;
    }
}
impl Drop for PolicyLiability {
    fn drop(&mut self) {
        if self.pending
            && let Some(scope) = AccountIdentity::upgrade(&self.account)
            && let Ok(mut state) = scope.state()
        {
            state.pending_policy = state
                .pending_policy
                .checked_sub(self.bytes)
                .expect("authentic semantic retirement");
        }
    }
}
