//! Immutable per-run views of a separately supervised physical owner.
use super::{HeadlessToolServices, HeadlessTools, RetainedTerminals};
use futures::future::BoxFuture;
use nocterm_cloud_model::{
    authority::{RunBinding, SessionReceipt},
    tool::{
        BackgroundCommandJournal, RunRetirement, RunToolRuntime, RunToolRuntimeFactory,
        ScopedSessionOpener, ScopedToolExecutor, ScopedToolRequest, ToolAuthority,
        ToolExecutionError, ToolExecutionResult, ToolSessionHandleSource, ToolTaskSpawner,
        ToolTimer,
    },
};
use std::sync::{Arc, Mutex};

/// Application composition supplies the original retained owner and its native
/// collaborators. Attach never opens SSH or renews an old execution grant.
pub struct RetainedToolsFactory {
    terminals: RetainedTerminals,
    timer: Arc<dyn ToolTimer>,
    journal: Arc<dyn BackgroundCommandJournal>,
    tasks: Arc<dyn ToolTaskSpawner>,
    opener: Option<Arc<dyn ScopedSessionOpener>>,
    handles: Arc<dyn ToolSessionHandleSource>,
}
impl RetainedToolsFactory {
    pub fn new(
        terminals: RetainedTerminals,
        timer: Arc<dyn ToolTimer>,
        journal: Arc<dyn BackgroundCommandJournal>,
        tasks: Arc<dyn ToolTaskSpawner>,
        opener: Arc<dyn ScopedSessionOpener>,
        handles: Arc<dyn ToolSessionHandleSource>,
    ) -> Self {
        Self {
            terminals,
            timer,
            journal,
            tasks,
            opener: Some(opener),
            handles,
        }
    }
    /// Initial physical creation is owned by a separate authenticated broker.
    pub fn existing_only(
        terminals: RetainedTerminals,
        timer: Arc<dyn ToolTimer>,
        journal: Arc<dyn BackgroundCommandJournal>,
        tasks: Arc<dyn ToolTaskSpawner>,
        handles: Arc<dyn ToolSessionHandleSource>,
    ) -> Self {
        Self {
            terminals,
            timer,
            journal,
            tasks,
            opener: None,
            handles,
        }
    }
}
impl RunToolRuntimeFactory for RetainedToolsFactory {
    fn attach(
        &self,
        run: RunBinding,
        receipt: SessionReceipt,
        authority: Arc<dyn ToolAuthority>,
    ) -> BoxFuture<'static, Result<Arc<dyn RunToolRuntime>, ToolExecutionError>> {
        let result = (|| {
            let mut registry = self.terminals.view(run.clone())?;
            let handle = self.handles.next(&run)?;
            let terminal =
                registry
                    .attach_retained(handle, &receipt)
                    .map_err(|error| match error {
                        super::HeadlessError::StaleSession => ToolExecutionError::NeedsUnlock,
                        error => error.into(),
                    })?;
            let services = if let Some(opener) = &self.opener {
                HeadlessToolServices::new(
                    authority,
                    self.timer.clone(),
                    self.journal.clone(),
                    self.tasks.clone(),
                    opener.clone(),
                )
            } else {
                HeadlessToolServices::existing_only(
                    authority,
                    self.timer.clone(),
                    self.journal.clone(),
                    self.tasks.clone(),
                )
            };
            let tools = HeadlessTools::new(Arc::new(Mutex::new(registry)), services);
            tools.offer_existing(handle, receipt.clone(), terminal)?;
            let runtime = Arc::new(RetainedRunTools {
                run,
                receipt,
                tools,
            });
            runtime.check_current()?;
            Ok(runtime as Arc<dyn RunToolRuntime>)
        })();
        Box::pin(async move { result })
    }
}
struct RetainedRunTools {
    run: RunBinding,
    receipt: SessionReceipt,
    tools: HeadlessTools,
}
impl ScopedToolExecutor for RetainedRunTools {
    fn execute(
        &self,
        request: ScopedToolRequest,
    ) -> BoxFuture<'static, Result<ToolExecutionResult, ToolExecutionError>> {
        self.tools.execute(request)
    }
}
impl RunToolRuntime for RetainedRunTools {
    fn run(&self) -> &RunBinding {
        &self.run
    }
    fn receipt(&self) -> &SessionReceipt {
        &self.receipt
    }
    fn check_current(&self) -> Result<(), ToolExecutionError> {
        self.tools.check_session(&self.run, &self.receipt)
    }
    fn fence(&self) {
        self.tools.detach();
    }
    fn retire(&self, reason: RunRetirement) -> BoxFuture<'static, Result<(), ToolExecutionError>> {
        match reason {
            RunRetirement::Completed => self.tools.detach(),
            RunRetirement::Stopped | RunRetirement::Revoked | RunRetirement::Interrupted => {
                self.tools.revoke()
            }
        }
        Box::pin(async { Ok(()) })
    }
}
