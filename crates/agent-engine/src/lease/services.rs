//! Optional host services attached to every lease a catalog installs.
use super::RenewalSetup;
use futures::future::BoxFuture;
use nocterm_cloud_model::engine::{
    EngineError, ModelCallAdmission, ModelCallObservation, ModelCallRequest, UsageObservation,
};
use std::sync::Arc;

/// Receives accounting facts of model work done through a lease. It sees
/// counts only, never content or credentials.
pub trait ModelUsageObserver: Send + Sync {
    /// An engine for a new run was created.
    fn run_started(&self);
    /// The host durably accepted one provider call's observed usage.
    fn call_observed(&self, usage: &UsageObservation);
}

/// Renewal and usage reporting for the leases of one catalog.
#[derive(Clone, Default)]
pub struct LeaseServices {
    renewal: Option<RenewalSetup>,
    usage: Option<Arc<dyn ModelUsageObserver>>,
}
impl LeaseServices {
    /// Renews each lease in the background while the presence port allows it.
    #[must_use]
    pub fn renewing(mut self, setup: RenewalSetup) -> Self {
        self.renewal = Some(setup);
        self
    }
    #[must_use]
    pub fn reporting(mut self, observer: Arc<dyn ModelUsageObserver>) -> Self {
        self.usage = Some(observer);
        self
    }
    pub(super) fn renewal(&self) -> Option<&RenewalSetup> {
        self.renewal.as_ref()
    }
    pub(super) fn usage(&self) -> Option<&Arc<dyn ModelUsageObserver>> {
        self.usage.as_ref()
    }
}

/// Forwards to the host's admission and reports a call only after the host
/// accepted its observation.
pub(crate) struct ObservedAdmission {
    inner: Arc<dyn ModelCallAdmission>,
    observer: Arc<dyn ModelUsageObserver>,
}
impl ObservedAdmission {
    pub(crate) fn wrap(
        inner: Arc<dyn ModelCallAdmission>,
        observer: Option<Arc<dyn ModelUsageObserver>>,
    ) -> Arc<dyn ModelCallAdmission> {
        match observer {
            Some(observer) => Arc::new(Self { inner, observer }),
            None => inner,
        }
    }
}
impl ModelCallAdmission for ObservedAdmission {
    fn reserve(&self, request: ModelCallRequest) -> BoxFuture<'static, Result<(), EngineError>> {
        self.inner.reserve(request)
    }
    fn reconcile(
        &self,
        request: ModelCallRequest,
        observation: ModelCallObservation,
    ) -> BoxFuture<'static, Result<(), EngineError>> {
        let accepted = self.inner.reconcile(request, observation.clone());
        let observer = self.observer.clone();
        Box::pin(async move {
            accepted.await?;
            observer.call_observed(&observation.usage());
            Ok(())
        })
    }
    fn check_current(&self, request: &ModelCallRequest) -> Result<(), EngineError> {
        self.inner.check_current(request)
    }
}
