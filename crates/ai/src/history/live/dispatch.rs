//! Pending dispatch keeps the original Foreground pinned until native preflight succeeds.
use super::{LiveAdmission, LiveError, LiveOwner};
use crate::{
    history::{
        dispatch::{ActiveDispatch, DispatchFraming, DispatchLifetime},
        physical::{
            DispatchPayload, DispatchStaging, PhysicalError, QueueSelection, SnapshotBuilder,
            SnapshotOwner, SnapshotStaging,
        },
    },
    model::retention::document::ConstructionRole,
};

struct PendingDispatch {
    snapshot: SnapshotOwner,
    payload: DispatchPayload,
    lifetime: DispatchLifetime,
}

/// Native preparation borrows the actual payload synchronously. Only commit
/// publishes the visible User and queue pop; abandoning this owner changes neither.
pub struct PreparedDispatch<'a> {
    pending: Option<PendingDispatch>,
    owner: &'a mut LiveOwner,
    admission: LiveAdmission,
}

impl<'a> PreparedDispatch<'a> {
    pub(super) fn prepare(
        owner: &'a mut LiveOwner,
        admission: LiveAdmission,
        framing: DispatchFraming,
        now: u64,
    ) -> Result<Self, LiveError> {
        owner.check(&admission)?;
        let lifetime = DispatchLifetime::new(owner.dispatch.capture()?)?;
        owner.foreground.ensure_live_phase()?;
        let mut source = DispatchBuildSource(Some(&mut owner.foreground));
        let role = source.0.as_deref_mut().expect("original foreground borrow");
        let mut builder = SnapshotBuilder::from_role(role, &owner.live)?;
        let payload = builder.prepare_dispatch(framing)?.into_payload();
        if !builder.append_prompt_user(0, now)? {
            return Err(PhysicalError::QueuedPromptMissing.into());
        }
        builder.select_queue(QueueSelection::WithoutFirst)?;
        let pinned = owner
            .live
            .snapshot()
            .ok_or(LiveError::MissingPayload)?
            .metadata()
            .pinned();
        builder.stamp_metadata(now, pinned)?;
        let next = builder.finish(None)?;
        crate::history::decision_managed(next.snapshot())?;
        let snapshot = next.into_owner();
        source.0.take();
        drop(source);
        Ok(Self {
            pending: Some(PendingDispatch {
                snapshot,
                payload,
                lifetime,
            }),
            owner,
            admission,
        })
    }

    pub fn payload(&self) -> &DispatchPayload {
        &self
            .pending
            .as_ref()
            .expect("live prepared dispatch")
            .payload
    }

    /// Admission and native serialization have completed; publish the SAME staged
    /// physical owners and return the original loan for the outer native future.
    pub fn commit(mut self) -> Result<ActiveDispatch, LiveError> {
        self.owner.check(&self.admission)?;
        let PendingDispatch {
            snapshot,
            payload,
            mut lifetime,
        } = self.pending.take().expect("live prepared dispatch");
        let next = SnapshotStaging::for_dispatch(snapshot, &self.owner.foreground);
        let request = DispatchStaging::for_dispatch(payload, self.owner.foreground.scope());
        self.owner.live.publish_dispatch(
            lifetime.original_mut()?.original_mut()?,
            next,
            request,
        )?;
        self.owner.revision += 1;
        Ok(lifetime.activate())
    }
}

impl Drop for PreparedDispatch<'_> {
    fn drop(&mut self) {
        drop(self.pending.take());
        self.owner.foreground.clear_source();
    }
}

// A failed/panicking constructor has no transaction yet to release this source.
struct DispatchBuildSource<'a>(Option<&'a mut ConstructionRole>);
impl Drop for DispatchBuildSource<'_> {
    fn drop(&mut self) {
        if let Some(role) = self.0.take() {
            role.clear_source();
        }
    }
}
