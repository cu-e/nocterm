//! Display capabilities retain immutable original recipient/action snapshots.
use super::{KeyFlow, key_admission::KeySnapshot};
use nocterm_hubs_model::{
    HubError, HubFuture, RecoveryKeyConfirmation, RecoveryKeyIssuer, RecoveryKeyOffer,
    RecoveryKeyReissue,
};
use nocterm_session::Secret;
use std::sync::{Arc, Weak};

struct Acknowledgement(KeySnapshot);
impl RecoveryKeyConfirmation for Acknowledgement {
    fn check_current(&self) -> Result<(), HubError> {
        self.0.check()
    }
    fn confirm(&self) -> HubFuture<()> {
        let snapshot = self.0.clone();
        Box::pin(async move { snapshot.confirm().await })
    }
}
pub(super) fn offer(secret: Secret, snapshot: KeySnapshot) -> RecoveryKeyOffer {
    RecoveryKeyOffer::new(
        snapshot.member().hub().clone(),
        secret,
        Arc::new(Acknowledgement(snapshot)),
    )
}

struct Issuer {
    flow: Weak<KeyFlow>,
    snapshot: KeySnapshot,
}
impl RecoveryKeyIssuer for Issuer {
    fn check_current(&self) -> Result<(), HubError> {
        self.snapshot.check()?;
        self.flow.upgrade().map(|_| ()).ok_or(HubError::Stale)
    }
    fn issue(&self) -> HubFuture<RecoveryKeyOffer> {
        let Some(flow) = self.flow.upgrade() else {
            return Box::pin(async { Err(HubError::Stale) });
        };
        let captured = flow.bind_snapshot(self.snapshot.clone());
        let worker = flow.clone();
        flow.run(captured, move |owner| async move {
            let shown = worker.reissue_recovery_key(&owner).await?;
            worker.offer(&owner, shown)
        })
    }
}
pub(super) fn issuer(flow: Weak<KeyFlow>, snapshot: KeySnapshot) -> RecoveryKeyReissue {
    RecoveryKeyReissue::new(
        snapshot.member().hub().clone(),
        Arc::new(Issuer { flow, snapshot }),
    )
}
