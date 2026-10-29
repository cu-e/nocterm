//! Scripted native tool lifetimes for coordinator behavior tests, never real I/O.
use crate::{authority::*, payload::BoundedJson, tool::*};
use futures::{FutureExt, future::BoxFuture};
use nocterm_session::testing::Gate;
use std::{
    collections::VecDeque,
    sync::{
        Arc, Mutex, PoisonError,
        atomic::{AtomicBool, Ordering},
    },
};

#[derive(Default)]
struct ToolScript {
    results: VecDeque<Result<BoundedJson, ToolExecutionError>>,
    requests: Vec<ScopedToolRequest>,
    retired: Vec<RunRetirement>,
    after_dispatch: Option<(Gate, Gate)>,
    attach_failure: Option<ToolExecutionError>,
}
#[derive(Clone, Default)]
pub struct ScriptedRunTools {
    script: Arc<Mutex<ToolScript>>,
}
impl ScriptedRunTools {
    pub fn require_unlock_on_next_attach(&self) {
        #[derive(Debug, thiserror::Error)]
        #[error("scripted signing provider is locked")]
        struct Locked;
        let cause =
            nocterm_session::SessionError::Connection(nocterm_session::ConnectionFailure::new(
                nocterm_session::ConnectionFailureKind::NeedsUnlock,
                Locked,
            ));
        self.script
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .attach_failure = Some(ToolExecutionError::Execution(
            nocterm_core::SharedCause::new(cause),
        ));
    }
    pub fn push(&self, result: Result<BoundedJson, ToolExecutionError>) {
        self.script
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .results
            .push_back(result);
    }
    pub fn requests(&self) -> Vec<ScopedToolRequest> {
        self.script
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .requests
            .clone()
    }
    pub fn retirements(&self) -> Vec<RunRetirement> {
        self.script
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .retired
            .clone()
    }
    pub fn pause_after_dispatch(&self) -> (Gate, Gate) {
        let entered = Gate::pending();
        let release = Gate::pending();
        self.script
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .after_dispatch = Some((entered.clone(), release.clone()));
        (entered, release)
    }
}
impl RunToolRuntimeFactory for ScriptedRunTools {
    fn attach(
        &self,
        run: RunBinding,
        receipt: SessionReceipt,
        authority: Arc<dyn ToolAuthority>,
    ) -> BoxFuture<'static, Result<Arc<dyn RunToolRuntime>, ToolExecutionError>> {
        if let Some(error) = self
            .script
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .attach_failure
            .take()
        {
            return futures::future::ready(Err(error)).boxed();
        }
        let runtime: Arc<dyn RunToolRuntime> = Arc::new(ScriptedRuntime {
            run,
            receipt,
            authority,
            script: self.clone(),
            current: Arc::new(AtomicBool::new(true)),
        });
        futures::future::ready(Ok(runtime)).boxed()
    }
}
struct ScriptedRuntime {
    run: RunBinding,
    receipt: SessionReceipt,
    authority: Arc<dyn ToolAuthority>,
    script: ScriptedRunTools,
    current: Arc<AtomicBool>,
}
impl ScopedToolExecutor for ScriptedRuntime {
    fn execute(
        &self,
        request: ScopedToolRequest,
    ) -> BoxFuture<'static, Result<ToolExecutionResult, ToolExecutionError>> {
        let check = self.check_current().and_then(|()| {
            if request.permit().intent().authority().run() != &self.run
                || request.permit().intent().authority().session() != &self.receipt
            {
                return Err(ToolExecutionError::StaleSession);
            }
            self.authority.check(&request)
        });
        let script = self.script.clone();
        let current = self.current.clone();
        async move {
            check?;
            let (result, pause) = {
                let mut state = script.script.lock().unwrap_or_else(PoisonError::into_inner);
                state.requests.push(request.clone());
                (
                    state
                        .results
                        .pop_front()
                        .unwrap_or(Err(ToolExecutionError::UnsupportedCapability)),
                    state.after_dispatch.take(),
                )
            };
            if let Some((entered, release)) = pause {
                entered.release();
                release.wait().await;
            }
            if !current.load(Ordering::Acquire) {
                return Err(ToolExecutionError::Revoked);
            }
            Ok(ToolExecutionResult::completed(&request, result?, None))
        }
        .boxed()
    }
}
impl RunToolRuntime for ScriptedRuntime {
    fn run(&self) -> &RunBinding {
        &self.run
    }
    fn receipt(&self) -> &SessionReceipt {
        &self.receipt
    }
    fn check_current(&self) -> Result<(), ToolExecutionError> {
        if self.current.load(Ordering::Acquire) {
            Ok(())
        } else {
            Err(ToolExecutionError::Revoked)
        }
    }
    fn fence(&self) {
        self.current.store(false, Ordering::Release);
    }
    fn retire(&self, reason: RunRetirement) -> BoxFuture<'static, Result<(), ToolExecutionError>> {
        self.fence();
        self.script
            .script
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .retired
            .push(reason);
        futures::future::ready(Ok(())).boxed()
    }
}
