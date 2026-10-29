//! Real journal declarations keep actual error results and never create missing ones.
use super::*;
use nocterm_ai::{ExecCommand, TerminalCall};
use nocterm_cloud_model::{payload::BoundedJson, tool::ToolExecutionError};

fn with_session(id: u8, control: ChatControlRevision) -> ReceivedCommand {
    let original = submit(id, control);
    let CommandBody::Submit(prompt) = original.body() else {
        unreachable!()
    };
    let destination = prompt.destination();
    let session = SessionReceipt::new(
        RemoteSessionId::from_bytes([0x73; 16]),
        SessionEpoch::new(1).unwrap(),
        destination.target().clone(),
        destination.authentication().clone(),
    );
    command(
        id,
        control,
        None,
        CommandBody::Submit(PromptRequest::new(
            prompt.prompt().clone(),
            selection(),
            RunDestination::new(
                destination.target().clone(),
                destination.authentication().clone(),
                Some(session),
            )
            .unwrap(),
            prompt.allowance(),
        )),
    )
}
pub(super) fn tool_turn() -> ScriptedEngineTurn {
    let step = StepId::from_bytes([0x74; 16]);
    let call = TerminalCall::ExecCommand(ExecCommand {
        terminal_id: "terminal".into(),
        program: "mutation".into(),
        args: Vec::new(),
        stdin: None,
        timeout_ms: None,
        yield_ms: None,
    });
    let mut turn = complete();
    turn.action = EngineAction::Tools(
        EngineToolBatch::new(
            step,
            vec![
                EngineToolCall::new(
                    ToolSlot::new(step, 0).unwrap(),
                    Some(NamespaceId::new("original-provider-call").unwrap()),
                    call,
                )
                .unwrap(),
            ],
        )
        .unwrap(),
    );
    turn
}
pub(super) async fn admitted(fixture: &Fixture) -> RunBinding {
    admitted_with_operations(fixture, 1).await
}
pub(super) async fn admitted_with_operations(fixture: &Fixture, operations: u32) -> RunBinding {
    let prompt = with_session(1, ChatControlRevision::default());
    let CommandBody::Submit(request) = prompt.body() else {
        unreachable!()
    };
    let session = request.destination().session().unwrap().clone();
    fixture.actor.accept(prompt).await.unwrap();
    let run = fixture.binding().await;
    fixture
        .actor
        .accept(inference(2, fixture.control().await, run.clone()))
        .await
        .unwrap();
    let control = fixture.control().await;
    let grant = GrantPolicy::new(
        ExecutionAuthority::new(
            run.clone(),
            session,
            GrantId::from_bytes([3; 16]),
            GrantEpoch::new(1).unwrap(),
        ),
        GrantOrigin::new(
            issuer(),
            CommandId::from_bytes([3; 16]),
            control.next().unwrap(),
        ),
        Capabilities::ALL,
        window(),
        operations,
    )
    .unwrap();
    fixture
        .actor
        .accept(command(
            3,
            control,
            Some(run.clone()),
            CommandBody::ToolGrant(grant),
        ))
        .await
        .unwrap();
    run
}
async fn approve(fixture: &Fixture, run: &RunBinding) -> nocterm_cloud_model::run::ToolIntent {
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        while fixture.actor.status(run).await.unwrap() != RunStatus::WaitingApproval {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    let intent = fixture
        .events
        .events()
        .into_iter()
        .find_map(|event| match event {
            CoordinatorEvent::ToolIntent { intent, .. } => Some(intent),
            _ => None,
        })
        .unwrap();
    fixture
        .actor
        .accept(command(
            4,
            fixture.control().await,
            Some(run.clone()),
            CommandBody::Approval(ApprovalAnswer::new(
                intent.id(),
                intent.authority().clone(),
                intent.arguments(),
                ApprovalChoice::Approve,
            )),
        ))
        .await
        .unwrap();
    intent
}
async fn reconcile(fixture: &Fixture, intent: &nocterm_cloud_model::run::ToolIntent) {
    fixture
        .actor
        .accept(command(
            5,
            fixture.control().await,
            Some(intent.authority().run().clone()),
            CommandBody::Approval(ApprovalAnswer::new(
                intent.id(),
                intent.authority().clone(),
                intent.arguments(),
                ApprovalChoice::ReconcileKnown(fp(88)),
            )),
        ))
        .await
        .unwrap();
}
#[tokio::test]
async fn reconciliation_preserves_an_actual_unknown_error_result_in_the_next_prompt_history() {
    let fixture = Fixture::new();
    fixture.engine.push(tool_turn());
    fixture.tools.push(Err(ToolExecutionError::OutcomeUnknown));
    let run = admitted(&fixture).await;
    let actor = fixture.actor.clone();
    let worker = tokio::spawn(async move { actor.drive_one().await });
    let intent = approve(&fixture, &run).await;
    assert!(worker.await.unwrap().is_err());
    assert_eq!(
        fixture.actor.status(&run).await.unwrap(),
        RunStatus::OutcomeUnknown
    );
    reconcile(&fixture, &intent).await;
    let receipt = fixture
        .actor
        .accept(submit(6, fixture.control().await))
        .await
        .unwrap();
    let next = submitted_run(&fixture, receipt.acceptance().id()).await;
    fixture
        .actor
        .accept(inference(7, fixture.control().await, next.clone()))
        .await
        .unwrap();
    fixture.engine.push(complete());
    fixture.actor.drive_one().await.unwrap();
    let configs = fixture.engine.configs();
    let result = configs
        .last()
        .unwrap()
        .history
        .messages()
        .iter()
        .find_map(|message| match message {
            EngineHistoryMessage::ToolResult(result) => Some(result),
            _ => None,
        })
        .unwrap();
    assert_eq!(result.intent(), intent.id());
    assert!(result.failed());
    assert!(result.output().value().get("error").is_some());
    assert_eq!(fixture.tools.requests().len(), 1);
    assert_eq!(fixture.engine.calls(), 2);
}
async fn submitted_run(fixture: &Fixture, id: CommandId) -> RunBinding {
    fixture
        .journal
        .read(stream(), JournalSequence::default(), limits())
        .await
        .unwrap()
        .records()
        .iter()
        .find_map(|stored| match stored.record().kind() {
            JournalRecordKind::CommandAccepted {
                id: actual,
                kind: CommandKind::SubmitPrompt,
            } if *actual == id => stored.record().context().run().cloned(),
            _ => None,
        })
        .unwrap()
}
#[tokio::test]
async fn a_reconciled_fingerprint_without_a_result_body_never_invents_history_or_redispatches() {
    let mut fixture = Fixture::new();
    fixture.engine.push(tool_turn());
    fixture
        .tools
        .push(Ok(BoundedJson::new(serde_json::json!({"exit":0})).unwrap()));
    let (entered, _release) = fixture.tools.pause_after_dispatch();
    let run = admitted(&fixture).await;
    let actor = fixture.actor.clone();
    let worker = tokio::spawn(async move { actor.drive_one().await });
    let intent = approve(&fixture, &run).await;
    entered.wait().await;
    worker.abort();
    assert!(worker.await.unwrap_err().is_cancelled());
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        while fixture.actor.status(&run).await.unwrap() != RunStatus::Interrupted {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    fixture.restart();
    fixture.actor.recover().await.unwrap();
    reconcile(&fixture, &intent).await;
    let receipt = fixture
        .actor
        .accept(submit(6, fixture.control().await))
        .await
        .unwrap();
    let next = submitted_run(&fixture, receipt.acceptance().id()).await;
    fixture
        .actor
        .accept(inference(7, fixture.control().await, next.clone()))
        .await
        .unwrap();
    fixture.engine.push(complete());
    assert!(fixture.actor.drive_one().await.is_err());
    assert_eq!(
        fixture.actor.status(&next).await.unwrap(),
        RunStatus::Interrupted
    );
    assert_eq!(
        fixture.actor.status(&run).await.unwrap(),
        RunStatus::Interrupted
    );
    assert_eq!(fixture.tools.requests().len(), 1);
    assert_eq!(fixture.engine.calls(), 1);
    assert!(!fixture.events.events().iter().any(|event|matches!(event,CoordinatorEvent::ToolResult{observation,..} if observation.intent()==intent.id())));
}
