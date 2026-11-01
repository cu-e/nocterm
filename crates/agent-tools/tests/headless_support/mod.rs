use futures::{FutureExt, executor::LocalPool, task::LocalSpawnExt};
use nocterm_agent_tools::headless::{
    ChatTransferBook, HeadlessTerminalOptions, HeadlessToolServices, HeadlessTools,
    RetainedTerminals, SessionRegistry,
};
use nocterm_cloud_model::{
    authority::{
        AuthenticationBinding, ExecutionAuthority, RunBinding, SessionReceipt, TargetBinding,
    },
    identity::*,
    run::{CommitStamp, RunState, ToolIntent},
    testing::{
        ManualToolTimer, QueuedToolTasks, ScriptedBackgroundJournal, ScriptedSessionOpener,
        ScriptedToolAuthority,
    },
    tool::{ScopedToolExecutor, ScopedToolRequest, ToolExecutionError, ToolExecutionResult},
};
use nocterm_session::{
    Charset, Event, RemoteFs, SessionDriver,
    testing::{FakeExec, InMemoryFs},
};
use nocterm_transfers::{TransferRequests, testing::FakeTransfers};
use std::sync::{Arc, Mutex};

pub(crate) fn fingerprint(byte: u8) -> Fingerprint {
    Fingerprint::from_bytes([byte; 32])
}
pub(crate) fn authority() -> ExecutionAuthority {
    let scope = AccountScope::new(
        HubIdentity::new(NamespaceId::new("hub").unwrap(), fingerprint(1)),
        AccountIdentity::new(NamespaceId::new("account").unwrap(), fingerprint(2)),
    );
    let run = RunBinding::new(
        scope,
        RunnerId::from_bytes([1; 16]),
        RunnerKeyEpoch::new(1).unwrap(),
        CloudChatId::from_bytes([2; 16]),
        RunId::from_bytes([3; 16]),
        RunEpoch::new(1).unwrap(),
    );
    let target = TargetBinding::new(
        fingerprint(3),
        fingerprint(4),
        NamespaceId::new("user").unwrap(),
        fingerprint(5),
    );
    let receipt = SessionReceipt::new(
        RemoteSessionId::from_bytes([4; 16]),
        SessionEpoch::new(1).unwrap(),
        target,
        AuthenticationBinding::new(fingerprint(6), fingerprint(7)),
    );
    ExecutionAuthority::new(
        run,
        receipt,
        GrantId::from_bytes([5; 16]),
        GrantEpoch::new(1).unwrap(),
    )
}
pub(crate) fn request(
    authority: &ExecutionAuthority,
    id: u8,
    call: nocterm_ai::TerminalCall,
) -> ScopedToolRequest {
    let arguments = fingerprint(id);
    let mut state = RunState::new(
        authority.run().clone(),
        authority.session().target().clone(),
        WriterEpoch::new(1).unwrap(),
        Revision::default(),
    );
    let intent = ToolIntent::new(
        IntentId::from_bytes([id; 16]),
        StepId::from_bytes([id; 16]),
        authority.clone(),
        arguments,
        &call,
    )
    .unwrap();
    state
        .prepare_intent(intent.clone(), state.revision())
        .unwrap();
    let stamp = |revision| {
        CommitStamp::new(
            authority.run().clone(),
            WriterEpoch::new(1).unwrap(),
            revision,
        )
    };
    state
        .intent_committed(intent.id(), stamp(state.revision().next().unwrap()))
        .unwrap();
    let permit = state
        .dispatch_committed(intent.id(), stamp(state.revision().next().unwrap()))
        .unwrap();
    ScopedToolRequest::new(permit, call, arguments).unwrap()
}

/// What the attached session offers beyond reading and execution.
#[derive(Default)]
struct Capabilities {
    files: Option<Arc<dyn RemoteFs>>,
    transfers: Option<ChatTransferBook>,
}

pub(crate) type PendingTool =
    futures::channel::oneshot::Receiver<Result<ToolExecutionResult, ToolExecutionError>>;
pub(crate) struct Harness {
    pub(crate) pool: LocalPool,
    pub(crate) tools: HeadlessTools,
    pub(crate) registry: Arc<Mutex<SessionRegistry>>,
    pub(crate) authority: ExecutionAuthority,
    pub(crate) checks: Arc<ScriptedToolAuthority>,
    pub(crate) timer: Arc<ManualToolTimer>,
    pub(crate) journal: Arc<ScriptedBackgroundJournal>,
    pub(crate) tasks: Arc<QueuedToolTasks>,
    pub(crate) opener: Arc<ScriptedSessionOpener>,
    pub(crate) executor: Arc<FakeExec>,
    pub(crate) driver: SessionDriver,
    pub(crate) terminal: String,
    pub(crate) task_errors: Arc<Mutex<Vec<ToolExecutionError>>>,
}
impl Harness {
    pub(crate) fn new(charset: Charset) -> Self {
        Self::configured(charset, None, Capabilities::default())
    }
    /// A session with a file system (unless `files` is false) whose transfers go
    /// to the explicitly driven `port`.
    pub(crate) fn transferring(port: &Arc<FakeTransfers>, files: bool) -> Self {
        Self::configured(
            Charset::Utf8,
            None,
            Capabilities {
                files: files.then(|| Arc::new(InMemoryFs::default()) as Arc<dyn RemoteFs>),
                transfers: Some(ChatTransferBook::new(
                    port.clone() as Arc<dyn TransferRequests>
                )),
            },
        )
    }
    pub(crate) fn retained(
        charset: Charset,
    ) -> (Self, RetainedTerminals, Arc<ScriptedToolAuthority>) {
        Self::retained_with(charset, Capabilities::default())
    }
    /// A retained session that has a file system; transfers come from the factory.
    pub(crate) fn retained_files() -> (Self, RetainedTerminals, Arc<ScriptedToolAuthority>) {
        Self::retained_with(
            Charset::Utf8,
            Capabilities {
                files: Some(Arc::new(InMemoryFs::default())),
                transfers: None,
            },
        )
    }
    fn retained_with(
        charset: Charset,
        capabilities: Capabilities,
    ) -> (Self, RetainedTerminals, Arc<ScriptedToolAuthority>) {
        let observations = Arc::new(ScriptedToolAuthority::new(authority()));
        let journal = Arc::new(ScriptedBackgroundJournal::default());
        let retained =
            RetainedTerminals::new(authority().run(), observations.clone(), journal.clone());
        (
            Self::configured(charset, Some((&retained, journal)), capabilities),
            retained,
            observations,
        )
    }
    fn configured(
        charset: Charset,
        retained: Option<(&RetainedTerminals, Arc<ScriptedBackgroundJournal>)>,
        capabilities: Capabilities,
    ) -> Self {
        let authority = authority();
        let registry = Arc::new(Mutex::new(match &retained {
            Some((retained, _)) => retained.view(authority.run().clone()).unwrap(),
            None => SessionRegistry::new(authority.run().clone()),
        }));
        let checks = Arc::new(ScriptedToolAuthority::new(authority.clone()));
        let timer = Arc::new(ManualToolTimer::default());
        let journal = retained.map_or_else(
            || Arc::new(ScriptedBackgroundJournal::default()),
            |(_, journal)| journal,
        );
        let tasks = Arc::new(QueuedToolTasks::default());
        let opener = Arc::new(ScriptedSessionOpener::default());
        let executor = Arc::new(FakeExec::default());
        let (session, driver) = nocterm_session::channel(None);
        let session = session
            .with_exec(executor.clone())
            .with_fs(capabilities.files);
        let task = registry
            .lock()
            .unwrap()
            .attach(
                RemoteSessionId::from_bytes([8; 16]),
                authority.session().clone(),
                session,
                HeadlessTerminalOptions {
                    charset,
                    ..HeadlessTerminalOptions::default()
                },
                timer.clone(),
            )
            .unwrap();
        let terminal = task.terminal().to_owned();
        let mut pool = LocalPool::new();
        pool.spawner().spawn_local(task.into_run()).unwrap();
        pool.run_until(driver.emit(Event::Connected));
        pool.run_until_stalled();
        let mut services = HeadlessToolServices::new(
            checks.clone(),
            timer.clone(),
            journal.clone(),
            tasks.clone(),
            opener.clone(),
        );
        if let Some(book) = capabilities.transfers {
            services = services.with_transfers(book);
        }
        let tools = HeadlessTools::new(registry.clone(), services);
        Self {
            pool,
            tools,
            registry,
            authority,
            checks,
            timer,
            journal,
            tasks,
            opener,
            executor,
            driver,
            terminal,
            task_errors: Arc::new(Mutex::new(Vec::new())),
        }
    }
    pub(crate) fn next_run(
        &mut self,
        retained: &RetainedTerminals,
        run_id: u8,
    ) -> (HeadlessTools, Arc<Mutex<SessionRegistry>>) {
        let run = RunBinding::new(
            self.authority.run().scope().clone(),
            self.authority.run().runner(),
            self.authority.run().runner_epoch(),
            self.authority.run().chat(),
            RunId::from_bytes([run_id; 16]),
            RunEpoch::new(1).unwrap(),
        );
        self.authority = ExecutionAuthority::new(
            run.clone(),
            self.authority.session().clone(),
            GrantId::from_bytes([run_id; 16]),
            GrantEpoch::new(1).unwrap(),
        );
        self.checks = Arc::new(ScriptedToolAuthority::new(self.authority.clone()));
        let mut registry = retained.view(run).unwrap();
        self.terminal = registry
            .attach_retained(
                RemoteSessionId::from_bytes([run_id; 16]),
                self.authority.session(),
            )
            .unwrap();
        let registry = Arc::new(Mutex::new(registry));
        let tools = HeadlessTools::new(
            registry.clone(),
            HeadlessToolServices::new(
                self.checks.clone(),
                self.timer.clone(),
                self.journal.clone(),
                self.tasks.clone(),
                self.opener.clone(),
            ),
        );
        let old_tools = std::mem::replace(&mut self.tools, tools);
        let old_registry = std::mem::replace(&mut self.registry, registry);
        (old_tools, old_registry)
    }
    pub(crate) fn request(&self, id: u8, call: nocterm_ai::TerminalCall) -> ScopedToolRequest {
        request(&self.authority, id, call)
    }
    pub(crate) fn start(&mut self, request: ScopedToolRequest) -> PendingTool {
        let (send, receive) = futures::channel::oneshot::channel();
        let work = self.tools.execute(request);
        self.pool
            .spawner()
            .spawn_local(async move {
                let _ = send.send(work.await);
            })
            .unwrap();
        self.pump();
        receive
    }
    pub(crate) fn pump(&mut self) {
        for _ in 0..8 {
            self.pool.run_until_stalled();
            let tasks = self.tasks.take();
            if tasks.is_empty() {
                return;
            }
            for task in tasks {
                let errors = self.task_errors.clone();
                self.pool
                    .spawner()
                    .spawn_local(async move {
                        if let Err(error) = task.await {
                            errors.lock().unwrap().push(error);
                        }
                    })
                    .unwrap();
            }
        }
        panic!("tool scheduling did not reach a bounded wait");
    }
    pub(crate) fn finish(
        &mut self,
        pending: PendingTool,
    ) -> Result<ToolExecutionResult, ToolExecutionError> {
        self.pump();
        pending
            .now_or_never()
            .expect("tool should have completed")
            .unwrap()
    }
    pub(crate) fn call(
        &mut self,
        id: u8,
        call: nocterm_ai::TerminalCall,
    ) -> Result<ToolExecutionResult, ToolExecutionError> {
        let request = self.request(id, call);
        let pending = self.start(request);
        self.finish(pending)
    }
    pub(crate) fn emit(&mut self, bytes: impl Into<Vec<u8>>) {
        assert!(
            self.pool
                .run_until(self.driver.emit(Event::Output(bytes.into())))
        );
        self.pump();
    }
    pub(crate) fn tick(&mut self, millis: u64) {
        self.timer.advance(millis);
        self.pump();
    }
    pub(crate) fn read(&mut self, id: u8, since: Option<u64>) -> serde_json::Value {
        self.call(
            id,
            nocterm_ai::TerminalCall::ReadTerminal(nocterm_ai::ReadTerminal {
                terminal_id: self.terminal.clone(),
                lines: None,
                since,
            }),
        )
        .unwrap()
        .output()
        .value()
        .clone()
    }
    pub(crate) fn exec(&self, program: &str) -> nocterm_ai::TerminalCall {
        nocterm_ai::TerminalCall::ExecCommand(nocterm_ai::ExecCommand {
            terminal_id: self.terminal.clone(),
            program: program.into(),
            args: Vec::new(),
            stdin: None,
            timeout_ms: None,
            yield_ms: Some(0),
        })
    }
}
