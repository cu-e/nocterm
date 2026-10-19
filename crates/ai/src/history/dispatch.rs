//! The original PromptDispatch capability is loaned through the request lifetime.
use crate::model::{
    json::lifetime::Shared,
    retention::{
        RetentionError,
        document::{AllocationPlan, DocumentRetention, PendingAllocation, RetainedRole},
    },
};
use std::sync::Mutex;

mod lifetime;
pub(in crate::history) use lifetime::DispatchLifetime;
pub use lifetime::{ActiveDispatch, ActivePromptScope};

/// Actual prepaid request framing, prepared while the original queue is intact.
pub struct DispatchFraming {
    session: crate::model::native::NativeOwned<crate::model::ids::SessionId>,
    context: crate::model::native::NativeOwned<Option<String>>,
    history: crate::model::native::NativeOwned<Option<String>>,
    command: bool,
}
pub(crate) type FramingParts = (
    (crate::model::ids::SessionId, Option<String>),
    Option<String>,
);
impl DispatchFraming {
    pub fn new(
        session: crate::model::native::NativeOwned<crate::model::ids::SessionId>,
        context: crate::model::native::NativeOwned<Option<String>>,
        history: crate::model::native::NativeOwned<Option<String>>,
        command: bool,
    ) -> Self {
        Self {
            session,
            context,
            history,
            command,
        }
    }
    pub(crate) fn into_source(
        self,
    ) -> Result<
        (crate::model::native::NativeOwned<FramingParts>, bool),
        crate::model::json::deferred::JsonDecodeError,
    > {
        Ok((
            self.session.join(self.context)?.join(self.history)?,
            self.command,
        ))
    }
}

#[derive(Debug, thiserror::Error)]
pub enum DispatchError {
    #[error("incorrect prompt dispatch capability")]
    WrongRole,
    #[error("the original prompt request is still alive")]
    Busy,
    #[error("prompt dispatch ownership is poisoned")]
    Poisoned,
    #[error(transparent)]
    Retention(#[from] RetentionError),
}
struct ProducerBody {
    available: Mutex<Option<RetainedRole>>,
    scope: DocumentRetention,
    _allocation: PendingAllocation,
}
#[derive(Clone)]
pub struct DispatchProducer(Shared<ProducerBody>);
impl DispatchProducer {
    pub fn new(original: RetainedRole) -> Result<Self, DispatchError> {
        if !original.is_prompt_dispatch() {
            return Err(DispatchError::WrongRole);
        }
        let scope = original.scope().clone();
        let allocation = scope.reserve_phase_metadata(AllocationPlan::shared::<ProducerBody>())?;
        Ok(Self(Shared::new(ProducerBody {
            available: Mutex::new(Some(original)),
            scope,
            _allocation: allocation,
        })))
    }
    pub fn capture(&self) -> Result<DispatchLease, DispatchError> {
        let role = self
            .0
            .available
            .lock()
            .map_err(|_| DispatchError::Poisoned)?
            .take()
            .ok_or(DispatchError::Busy)?;
        Ok(DispatchLease {
            role: Some(role),
            producer: self.clone(),
        })
    }
    pub fn same_document(&self, other: &Self) -> bool {
        self.0.scope.same_document(&other.0.scope)
    }
}
/// Not Clone. The native request future must drop before this original loan.
pub struct DispatchLease {
    role: Option<RetainedRole>,
    producer: DispatchProducer,
}
impl DispatchLease {
    pub fn payload(&self) -> Option<&crate::history::physical::DispatchPayload> {
        self.role.as_ref().and_then(RetainedRole::dispatch)
    }
    pub(crate) fn original_mut(&mut self) -> Result<&mut RetainedRole, DispatchError> {
        self.role.as_mut().ok_or(DispatchError::Busy)
    }
}
impl Drop for DispatchLease {
    fn drop(&mut self) {
        let Some(mut role) = self.role.take() else {
            return;
        };
        role.clear();
        if let Ok(mut available) = self.producer.0.available.lock()
            && available.is_none()
        {
            *available = Some(role);
        }
    }
}
