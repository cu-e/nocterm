//! Trusted native observations and actual SDK publication, without an execution fake.
use super::*;
use nocterm_ai::{TerminalCall, tool_display::ToolDisplay};
use nocterm_cloud_model::{
    authority::ExecutionAuthority,
    coordinator::{PromptAllowance, PromptRequest, RunDestination},
    engine::{EnginePrompt, ModelBudget},
    grant::GrantWindow,
    inference::ModelSelection,
    journal::{CommandAcceptance, CommandIssuer, CommandKind},
    run::ToolIntent,
    tool::ArgumentFingerprint,
};
use nocterm_hub_model::cloud::events::{
    DisplayOutcome, ObservedEffect, ToolObservationPayload, ToolPresentation,
};

pub(super) struct OriginalTool {
    pub intent: ToolIntent,
    pub call: TerminalCall,
    pub dispatch_revision: u64,
}
pub(super) struct History<'a> {
    fixture: &'a Fixture,
    records: Vec<ResidentPublication>,
    position: JournalPosition,
    time: u64,
    work: PublicationBudget,
}
impl<'a> History<'a> {
    pub(super) fn new(fixture: &'a Fixture) -> Self {
        Self {
            fixture,
            records: Vec::new(),
            position: JournalPosition::default(),
            time: TIME,
            work: Fixture::budget(64 * 1024 * 1024),
        }
    }
    pub(super) fn source(&self, previous: Option<ResidentPublication>) -> SnapshotProjectionSource {
        source(self.fixture, self.position, previous, self.records.clone())
    }
    pub(super) fn next_snapshot(&self) -> JournalPosition {
        self.position.next(1, 1, false).unwrap()
    }
    pub(super) fn time(&self) -> u64 {
        self.time
    }
    pub(super) fn continue_after(&mut self, snapshot: &ResidentPublication) {
        self.records.clear();
        let at = snapshot.anchor().position();
        self.position = JournalPosition::new(
            at.revision(),
            self.position.control(),
            at.journal(),
            at.event(),
        );
        self.time += 1000;
    }
    pub(super) fn prompt(&mut self, run: &RunBinding, id: u8) {
        let authority = self.fixture.authority();
        let window = GrantWindow::new(
            Timestamp::from_millis(self.time),
            Timestamp::from_millis(self.time + 60000),
        )
        .unwrap();
        let request = PromptRequest::new(
            EnginePrompt::new(vec![format!("original prompt {id}").into()]).unwrap(),
            ModelSelection::new(
                NamespaceId::new("provider").unwrap(),
                NamespaceId::new("model").unwrap(),
                NamespaceId::new("pricing-v1").unwrap(),
            ),
            RunDestination::new(
                authority.session().target().clone(),
                authority.session().authentication().clone(),
                Some(authority.session().clone()),
            )
            .unwrap(),
            PromptAllowance::new(ModelBudget::new(2, 4096, 4096, 100).unwrap(), 20, window)
                .unwrap(),
        );
        let command = CommandAcceptance::new(
            CommandId::from_bytes([id; 16]),
            CommandKind::SubmitPrompt,
            Fingerprint::from_bytes([id; 32]),
            CommandIssuer::new(
                NamespaceId::new("device").unwrap(),
                Fingerprint::from_bytes([51; 32]),
                Fingerprint::from_bytes([52; 32]),
            ),
            self.position.control(),
        );
        self.push(
            CoordinatorEvent::PromptAccepted {
                command,
                run: run.clone(),
                request,
            },
            true,
        );
    }
    pub(super) fn status(&mut self, run: &RunBinding, status: RunStatus) {
        self.push(
            CoordinatorEvent::Status {
                run: run.clone(),
                status,
            },
            false,
        );
    }
    pub(super) fn usage(&mut self, run: &RunBinding, cost: Option<u64>) {
        self.push(
            CoordinatorEvent::Progress {
                run: run.clone(),
                event: EngineEvent::Usage(UsageObservation {
                    input_tokens: Some(7),
                    cost_micros: cost,
                    ..Default::default()
                }),
            },
            false,
        );
    }
    pub(super) fn text(&mut self, run: &RunBinding, text: &str) {
        self.push(
            CoordinatorEvent::Progress {
                run: run.clone(),
                event: EngineEvent::Text(
                    EngineText::new(TextChannel::Assistant, text.into()).unwrap(),
                ),
            },
            false,
        );
    }
    pub(super) fn tool(&mut self, run: &RunBinding, id: u8) -> OriginalTool {
        let (call, _) = super::super::tools::calls()
            .into_iter()
            .find(|(call, _)| call.name() == "exec_command")
            .unwrap();
        let original = self.fixture.authority();
        let authority = ExecutionAuthority::new(
            run.clone(),
            original.session().clone(),
            original.grant(),
            original.grant_epoch(),
        );
        let fingerprint = RunnerArgumentFingerprint::new(Arc::new(Fixture::budget(1024 * 1024)))
            .fingerprint(&call)
            .unwrap();
        let intent = ToolIntent::new(
            IntentId::from_bytes([id; 16]),
            StepId::from_bytes([id.wrapping_add(1); 16]),
            authority,
            fingerprint,
            &call,
        )
        .unwrap();
        self.push(
            CoordinatorEvent::ToolIntent {
                intent: intent.clone(),
                call: call.clone(),
                display: ToolDisplay::new(&call, Some("original target".into())),
            },
            false,
        );
        OriginalTool {
            intent,
            call,
            dispatch_revision: self.position.revision().get(),
        }
    }
    pub(super) fn approval(&mut self, tool: &OriginalTool) {
        self.push(
            CoordinatorEvent::Approval {
                intent: tool.intent.clone(),
                call: tool.call.clone(),
                expires: Timestamp::from_millis(self.time + 60000),
            },
            false,
        );
    }
    pub(super) fn background_result(&mut self, tool: &OriginalTool) {
        let job = job(tool);
        let arguments = serde_json::to_string(&tool.call.arguments().unwrap()).unwrap();
        let output = r#"{"command_id":"original-job","running":true}"#;
        let observation = ToolObservationPayload {
            intent: *tool.intent.id().as_bytes(),
            output_json: output,
            display: ToolPresentation {
                tool: tool.call.name(),
                arguments_json: &arguments,
                destination: Some("original target"),
                redacted: false,
                outcome: Some(DisplayOutcome::Ok(output)),
            },
            effect: ObservedEffect::Background(Box::new(job)),
            failed: false,
        };
        self.public(
            CloudEventPayload::ToolResult {
                run: values::run(tool.intent.authority().run()),
                observation: Box::new(observation),
            },
            Some(tool.intent.authority().run()),
            false,
        );
    }
    pub(super) fn background(&mut self, tool: &OriginalTool, outcome: BackgroundOutcome) {
        let fact = BackgroundLifecyclePayload {
            job: job(tool),
            step: *tool.intent.step().as_bytes(),
            arguments_digest: *tool.intent.arguments().as_bytes(),
            mutates: tool.intent.mutates(),
            dispatch_writer_epoch: self.fixture.lease.epoch().get(),
            dispatch_revision: tool.dispatch_revision,
            outcome,
        };
        self.public(
            CloudEventPayload::BackgroundLifecycle(Box::new(fact)),
            Some(tool.intent.authority().run()),
            false,
        );
    }
    pub(super) fn delete(&mut self) {
        self.push(
            CoordinatorEvent::Deleted {
                command: CommandId::from_bytes([200; 16]),
            },
            true,
        );
    }
    fn push(&mut self, event: CoordinatorEvent, advance_control: bool) {
        self.position = self.position.next(1, 1, advance_control).unwrap();
        self.time += 1000;
        let encoder = RunnerEventEncoder::new(
            self.fixture.material.clone(),
            Arc::new(FixedClock::new(WallTime {
                unix_seconds: i64::try_from(self.time / 1000).unwrap(),
                utc_offset_seconds: 0,
            })),
        );
        self.records.push(
            CoordinatorEventEncoder::encode(
                &encoder,
                &self.fixture.lease,
                self.position,
                &event,
                &self.work,
            )
            .unwrap(),
        );
    }
    fn public(
        &mut self,
        payload: CloudEventPayload<'_>,
        run: Option<&RunBinding>,
        advance_control: bool,
    ) {
        self.position = self.position.next(1, 1, advance_control).unwrap();
        self.time += 1000;
        self.records.push(
            RecordSealer::new(&self.fixture.material, &self.work)
                .seal(
                    &self.fixture.lease,
                    self.position,
                    run,
                    RecordPayload::Event(CloudEvent {
                        control_revision: self.position.control().get(),
                        timestamp_ms: self.time,
                        payload,
                    }),
                )
                .unwrap(),
        );
    }
}
fn job(tool: &OriginalTool) -> BackgroundJob<'_> {
    BackgroundJob {
        intent: *tool.intent.id().as_bytes(),
        execution: values::execution(tool.intent.authority()).unwrap(),
        terminal: "terminal",
        command: "original-job",
        window: None,
    }
}
