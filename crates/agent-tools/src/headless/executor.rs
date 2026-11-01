//! All eight catalog calls pass through original authority and native session owners.
use super::{
    HeadlessError, HeadlessTerminalOptions, SessionRegistry,
    commands::{BackgroundJobs, CommandSupervisor, RuntimeOwner, deadline},
    dispatch::DispatchBook,
    presentation,
    registry::{HeadlessSessionStatus, terminal_handle},
};
use futures::{FutureExt, future::BoxFuture};
use nocterm_ai::TerminalCall;
use nocterm_cloud_model::{
    authority::SessionReceipt,
    identity::RemoteSessionId,
    payload::BoundedJson,
    tool::{
        BackgroundCommandJournal, ScopedSessionOpener, ScopedToolExecutor, ScopedToolRequest,
        ToolAuthority, ToolEffectStatus, ToolExecutionError, ToolExecutionResult, ToolTaskSpawner,
        ToolTimer,
    },
};
use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex, PoisonError},
    time::Duration,
};

struct OfferedSession {
    receipt: SessionReceipt,
    connection: OfferedConnection,
}
enum OfferedConnection {
    Fresh {
        handle: RemoteSessionId,
        options: HeadlessTerminalOptions,
    },
    Existing {
        terminal: String,
    },
}

pub struct HeadlessToolServices {
    authority: Arc<dyn ToolAuthority>,
    timer: Arc<dyn ToolTimer>,
    journal: Arc<dyn BackgroundCommandJournal>,
    tasks: Arc<dyn ToolTaskSpawner>,
    opener: Option<Arc<dyn ScopedSessionOpener>>,
}

impl HeadlessToolServices {
    pub fn new(
        authority: Arc<dyn ToolAuthority>,
        timer: Arc<dyn ToolTimer>,
        journal: Arc<dyn BackgroundCommandJournal>,
        tasks: Arc<dyn ToolTaskSpawner>,
        opener: Arc<dyn ScopedSessionOpener>,
    ) -> Self {
        Self {
            authority,
            timer,
            journal,
            tasks,
            opener: Some(opener),
        }
    }
    /// Physical acquisition is exclusively owned by the initial-session broker.
    pub fn existing_only(
        authority: Arc<dyn ToolAuthority>,
        timer: Arc<dyn ToolTimer>,
        journal: Arc<dyn BackgroundCommandJournal>,
        tasks: Arc<dyn ToolTaskSpawner>,
    ) -> Self {
        Self {
            authority,
            timer,
            journal,
            tasks,
            opener: None,
        }
    }
}

struct ToolRuntime {
    registry: Arc<Mutex<SessionRegistry>>,
    authority: Arc<dyn ToolAuthority>,
    timer: Arc<dyn ToolTimer>,
    journal: Arc<dyn BackgroundCommandJournal>,
    tasks: Arc<dyn ToolTaskSpawner>,
    opener: Option<Arc<dyn ScopedSessionOpener>>,
    jobs: Arc<Mutex<BackgroundJobs>>,
    dispatches: Mutex<DispatchBook>,
    offered: Mutex<BTreeMap<String, OfferedSession>>,
    retained: Option<std::sync::Weak<super::retained::RetainedOwner>>,
    owner: RuntimeOwner,
}

/// Native composition retains this owner for one run, including background tasks.
/// The injected authority/journal remain authoritative across worker restart.
#[derive(Clone)]
pub struct HeadlessTools(Arc<ToolRuntime>);
impl HeadlessTools {
    pub fn new(registry: Arc<Mutex<SessionRegistry>>, services: HeadlessToolServices) -> Self {
        let (owner, fence) = {
            let registry = registry.lock().unwrap_or_else(PoisonError::into_inner);
            (registry.retained_owner(), registry.view_fence())
        };
        let jobs = owner.as_ref().map_or_else(
            || Arc::new(Mutex::new(BackgroundJobs::new())),
            |owner| owner.jobs(),
        );
        Self(Arc::new(ToolRuntime {
            registry,
            authority: Arc::new(super::view::ViewAuthority::new(fence, services.authority)),
            timer: services.timer,
            journal: services.journal,
            tasks: services.tasks,
            opener: services.opener,
            jobs,
            dispatches: Mutex::new(DispatchBook::default()),
            offered: Mutex::new(BTreeMap::new()),
            retained: owner.as_ref().map(Arc::downgrade),
            owner: RuntimeOwner::new(),
        }))
    }
    pub(super) fn check_session(
        &self,
        run: &nocterm_cloud_model::authority::RunBinding,
        receipt: &SessionReceipt,
    ) -> Result<(), ToolExecutionError> {
        let registry = self
            .0
            .registry
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let info = registry
            .list(run)?
            .into_iter()
            .find(|info| &info.receipt == receipt)
            .ok_or(ToolExecutionError::StaleSession)?;
        if let Some(error) = info.failure() {
            return Err(HeadlessError::Session(error.clone()).into());
        }
        match info.status {
            HeadlessSessionStatus::Connected => Ok(()),
            HeadlessSessionStatus::NeedsUnlock | HeadlessSessionStatus::Closed => {
                Err(ToolExecutionError::NeedsUnlock)
            }
            HeadlessSessionStatus::Connecting => Err(ToolExecutionError::OutcomeUnknown),
        }
    }
    /// Composition supplies only already selected targets with a fresh opaque handle.
    /// A model cannot add targets, select a different credential or supply a URL.
    pub fn offer(
        &self,
        handle: RemoteSessionId,
        receipt: SessionReceipt,
        options: HeadlessTerminalOptions,
    ) -> Result<String, HeadlessError> {
        options.validate()?;
        let id = terminal_handle(handle);
        self.0
            .registry
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .require_fresh_handle(&id)?;
        let mut offered = self
            .0
            .offered
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        if offered.len() >= 16 {
            return Err(HeadlessError::Capacity);
        }
        if offered.contains_key(&id) {
            return Err(HeadlessError::IdentityConflict);
        }
        offered.insert(
            id.clone(),
            OfferedSession {
                receipt,
                connection: OfferedConnection::Fresh { handle, options },
            },
        );
        Ok(id)
    }
    /// Publish a fresh run-local alias for one actual connected view. Repeated
    /// opens reuse that physical session and never create another reader/login.
    pub fn offer_existing(
        &self,
        handle: RemoteSessionId,
        receipt: SessionReceipt,
        terminal: String,
    ) -> Result<String, HeadlessError> {
        let id = terminal_handle(handle);
        {
            let registry = self
                .0
                .registry
                .lock()
                .unwrap_or_else(PoisonError::into_inner);
            if id != terminal {
                registry.require_fresh_handle(&id)?;
            }
            registry.require_existing(&terminal, &receipt)?;
        }
        let mut offered = self
            .0
            .offered
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        if offered.len() >= 16 {
            return Err(HeadlessError::Capacity);
        }
        if offered.contains_key(&id) {
            return Err(HeadlessError::IdentityConflict);
        }
        offered.insert(
            id.clone(),
            OfferedSession {
                receipt,
                connection: OfferedConnection::Existing { terminal },
            },
        );
        Ok(id)
    }
    /// Fence only this foreground view. Already admitted retained background
    /// observations keep their original authority and physical owner.
    pub fn detach(&self) {
        self.0
            .registry
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .close();
    }
    /// Invalidate session admission before cancelling owned work. Cancellation does
    /// not claim remote descendant termination or reconcile unknown effects.
    pub fn revoke(&self) {
        self.0
            .registry
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .close();
        if self.0.retained.is_some() {
            self.0
                .jobs
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .cancel_owner(&self.0.owner);
        } else {
            self.0
                .jobs
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .cancel_all();
        }
    }
}
impl ScopedToolExecutor for HeadlessTools {
    fn execute(
        &self,
        request: ScopedToolRequest,
    ) -> BoxFuture<'static, Result<ToolExecutionResult, ToolExecutionError>> {
        let runtime = self.0.clone();
        async move {
            runtime.authority.check(&request)?;
            {
                let registry = runtime
                    .registry
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner);
                registry.require_run(request.permit().intent().authority().run())?;
            }
            let cached = runtime
                .dispatches
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .claim(&request)?;
            if let Some(cached) = cached {
                return cached;
            }
            let result = runtime.dispatch(&request).await;
            runtime
                .dispatches
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .finish(request.permit().intent().id(), result.clone());
            result
        }
        .boxed()
    }
}
impl ToolRuntime {
    async fn dispatch(
        &self,
        request: &ScopedToolRequest,
    ) -> Result<ToolExecutionResult, ToolExecutionError> {
        let (output, effect) = match request.call() {
            TerminalCall::ListTerminals => (self.list(request)?, ToolEffectStatus::Observed),
            TerminalCall::OpenTerminal(call) => (
                self.open(request, &call.server_id).await?,
                ToolEffectStatus::Observed,
            ),
            TerminalCall::ReadTerminal(call) => {
                let entry = self
                    .registry
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .get(request)?;
                let entry = entry.lock().unwrap_or_else(PoisonError::into_inner);
                self.authority.check(request)?;
                (
                    presentation::text(entry.read(call.lines.unwrap_or(200), call.since)),
                    ToolEffectStatus::Observed,
                )
            }
            TerminalCall::SendInput(call) => {
                let entry = self
                    .registry
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .get(request)?;
                let text = format!("{}{}", call.text, if call.press_enter { "\r" } else { "" });
                let _ = entry.lock().unwrap_or_else(PoisonError::into_inner).send(
                    request,
                    self.authority.as_ref(),
                    &text,
                    false,
                )?;
                (
                    serde_json::json!({"accepted":true}),
                    ToolEffectStatus::Observed,
                )
            }
            TerminalCall::RunCommand(call) => self.live(request, call).await?,
            TerminalCall::ExecCommand(_) => self.start_command(request).await?,
            TerminalCall::ReadCommand(call) => {
                let (output, _) = self
                    .wait_command(request, &call.command_id, call.yield_ms.unwrap_or(0), false)
                    .await?;
                (output, ToolEffectStatus::Observed)
            }
            TerminalCall::CancelCommand(call) => {
                let (output, _) = self
                    .wait_command(request, &call.command_id, 0, true)
                    .await?;
                (output, ToolEffectStatus::Unknown)
            }
            // Interim: headless transfers are not composed into this executor yet.
            TerminalCall::TransferFiles(_)
            | TerminalCall::ReadTransfer(_)
            | TerminalCall::CancelTransfer(_) => {
                return Err(ToolExecutionError::UnsupportedCapability);
            }
        };
        ToolExecutionResult::completed(
            request,
            BoundedJson::new(output)?,
            request.call().target().map(str::to_owned),
        )
        .with_effect_status(request, effect)
    }
    fn list(&self, request: &ScopedToolRequest) -> Result<serde_json::Value, ToolExecutionError> {
        let authority = request.permit().intent().authority();
        let sessions = self
            .registry
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .list(authority.run())?;
        let offered = self.offered.lock().unwrap_or_else(PoisonError::into_inner);
        let terminals: Vec<_> = sessions.into_iter().map(|session| {
            let server = offered.iter().find_map(|(id, entry)| match &entry.connection {
                OfferedConnection::Existing { terminal } if terminal == &session.terminal_id
                    && entry.receipt == session.receipt => Some(id.as_str()),
                _ => None,
            });
            serde_json::json!({
            "terminal_id":session.terminal_id, "connected":session.status == HeadlessSessionStatus::Connected,
            "server_id":server,
            "capabilities":{"read":true,"input":true,"live":true,"exec":session.execution,"files":session.files},
        })}).collect();
        let offline: Vec<_> = offered
            .iter()
            .filter(|(_, session)| {
                matches!(session.connection, OfferedConnection::Fresh { .. })
                    && session.receipt.target() == authority.session().target()
            })
            .map(|(id, _)| serde_json::json!({"server_id":id}))
            .collect();
        Ok(serde_json::json!({"terminals":terminals,"offline_servers":offline}))
    }
    fn existing_terminal(
        &self,
        request: &ScopedToolRequest,
        server: &str,
    ) -> Result<Option<String>, ToolExecutionError> {
        let existing = {
            let offered = self.offered.lock().unwrap_or_else(PoisonError::into_inner);
            let entry = offered.get(server).ok_or(HeadlessError::StaleSession)?;
            if &entry.receipt != request.permit().intent().authority().session() {
                return Err(HeadlessError::StaleSession.into());
            }
            match &entry.connection {
                OfferedConnection::Existing { terminal } => {
                    Some((terminal.clone(), entry.receipt.clone()))
                }
                OfferedConnection::Fresh { .. } => None,
            }
        };
        if let Some((terminal, receipt)) = existing {
            self.authority.check(request)?;
            self.registry
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .require_existing(&terminal, &receipt)?;
            self.authority.check(request)?;
            return Ok(Some(terminal));
        }
        Ok(None)
    }
    async fn open(
        &self,
        request: &ScopedToolRequest,
        server: &str,
    ) -> Result<serde_json::Value, ToolExecutionError> {
        if let Some(terminal) = self.existing_terminal(request, server)? {
            return Ok(serde_json::json!({"terminal_id":terminal}));
        }
        let opener = self
            .opener
            .as_ref()
            .ok_or(ToolExecutionError::NeedsUnlock)?;
        let (handle, receipt, options) = {
            let mut offered = self.offered.lock().unwrap_or_else(PoisonError::into_inner);
            let entry = offered.remove(server).ok_or(HeadlessError::StaleSession)?;
            if &entry.receipt != request.permit().intent().authority().session() {
                return Err(HeadlessError::StaleSession.into());
            }
            let OfferedConnection::Fresh { handle, options } = entry.connection else {
                return Err(HeadlessError::IdentityConflict.into());
            };
            (handle, entry.receipt, options)
        };
        self.authority.check(request)?;
        let (actual, session) = opener.open(request.clone()).await?.into_parts();
        self.authority
            .check(request)
            .inspect_err(|_| session.close())?;
        if actual != receipt {
            session.close();
            return Err(HeadlessError::StaleSession.into());
        }
        let task = self
            .registry
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .attach_pending(handle, actual, session, options, self.timer.clone())?;
        let terminal = task.terminal().to_owned();
        let candidate = super::pending_open::PendingOpen::new(
            self.registry.clone(),
            terminal.clone(),
            task.original(),
        );
        self.tasks.spawn(
            async move {
                task.into_run().await;
                Ok(())
            }
            .boxed(),
        )?;
        self.wait_connected(request, &terminal).await?;
        candidate.connected()?;
        Ok(serde_json::json!({"terminal_id":terminal}))
    }
    async fn wait_connected(
        &self,
        request: &ScopedToolRequest,
        terminal: &str,
    ) -> Result<(), ToolExecutionError> {
        let started = self.timer.now();
        loop {
            self.authority.check(request)?;
            let sessions = self
                .registry
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .list(request.permit().intent().authority().run())?;
            let session = sessions
                .iter()
                .find(|session| session.terminal_id == terminal)
                .ok_or(HeadlessError::StaleSession)?;
            if let Some(error) = session.failure() {
                return Err(HeadlessError::Session(error.clone()).into());
            }
            match session.status {
                HeadlessSessionStatus::Connected => return Ok(()),
                HeadlessSessionStatus::NeedsUnlock => return Err(ToolExecutionError::NeedsUnlock),
                HeadlessSessionStatus::Closed => return Err(ToolExecutionError::StaleSession),
                HeadlessSessionStatus::Connecting => {}
            }
            let elapsed = self
                .timer
                .now()
                .elapsed_since(started)
                .ok_or(ToolExecutionError::Revoked)?;
            if elapsed >= 30_000 {
                return Err(ToolExecutionError::OutcomeUnknown);
            }
            self.timer.delay(Duration::from_millis(25)).await;
        }
    }
    async fn start_command(
        &self,
        request: &ScopedToolRequest,
    ) -> Result<(serde_json::Value, ToolEffectStatus), ToolExecutionError> {
        let entry = self
            .registry
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(request)?;
        let executor = entry
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .executor()?;
        self.authority.check(request)?;
        let retained = self.retained.as_ref().and_then(std::sync::Weak::upgrade);
        let prepared = self
            .jobs
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .start(
                request,
                executor,
                self.owner.clone(),
                deadline(
                    request.clone(),
                    self.authority.clone(),
                    self.timer.clone(),
                    retained.as_ref().map(|owner| owner.observations()),
                ),
            )?;
        let id = prepared.id().to_owned();
        prepared
            .launch(
                CommandSupervisor::new(
                    self.jobs.clone(),
                    self.journal.clone(),
                    self.authority.clone(),
                    entry,
                    retained.as_deref(),
                ),
                self.tasks.as_ref(),
                request.clone(),
            )
            .await?;
        let TerminalCall::ExecCommand(call) = request.call() else {
            return Err(ToolExecutionError::ArgumentMismatch);
        };
        self.wait_command(request, &id, call.yield_ms.unwrap_or(1000), false)
            .await
    }
    async fn wait_command(
        &self,
        request: &ScopedToolRequest,
        id: &str,
        yield_ms: u64,
        cancel: bool,
    ) -> Result<(serde_json::Value, ToolEffectStatus), ToolExecutionError> {
        let started = self.timer.now();
        let until = started.as_millis().saturating_add(yield_ms);
        loop {
            let entry = self
                .registry
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .get(request)?;
            entry
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .check(request.permit().intent().authority())?;
            self.authority.check(request)?;
            if self.timer.now() < started {
                return Err(ToolExecutionError::Revoked);
            }
            let (value, effect) = self
                .jobs
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .snapshot(id, request.permit().intent().authority().session(), cancel)?;
            if cancel
                || value["state"] != "starting" && value["state"] != "running"
                || self.timer.now().as_millis() >= until
            {
                return Ok((value, effect));
            }
            self.timer.delay(Duration::from_millis(25)).await;
        }
    }
    async fn live(
        &self,
        request: &ScopedToolRequest,
        call: &nocterm_ai::RunCommand,
    ) -> Result<(serde_json::Value, ToolEffectStatus), ToolExecutionError> {
        let entry = self
            .registry
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(request)?;
        let (since, owner) = {
            let mut terminal = entry.lock().unwrap_or_else(PoisonError::into_inner);
            let since = terminal.read(2000, None).resume_line;
            let owner = terminal
                .send(request, self.authority.as_ref(), &call.command, true)?
                .ok_or(ToolExecutionError::ArgumentMismatch)?;
            (since, owner)
        };
        super::live::LiveObservation::new(entry, since, call, self.timer.now(), owner)
            .observe(request, self.authority.as_ref(), self.timer.as_ref())
            .await
    }
}

impl Drop for ToolRuntime {
    fn drop(&mut self) {
        self.registry
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .close();
        if self.retained.is_none() {
            self.jobs
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .cancel_all();
        }
    }
}
