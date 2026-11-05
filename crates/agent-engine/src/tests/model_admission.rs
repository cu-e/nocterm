use super::{fixtures::*, *};
use nocterm_cloud_model::{engine::EngineAction, identity::StreamRevision};
use serde_json::json;

#[tokio::test]
async fn sdk_turn_ceiling_after_tool_results_makes_no_new_provider_request_or_reservation() {
    use nocterm_ai::model::session::StopReason;
    use nocterm_cloud_model::{
        engine::{EngineToolResult, EngineToolResults},
        payload::BoundedJson,
    };
    for kind in [ProviderKind::OpenAiChat, ProviderKind::AnthropicMessages] {
        let args = r#"{"terminal_id":"session","program":"printf"}"#;
        let body = match kind {
            ProviderKind::OpenAiChat => chat_tools(args, 1, true),
            ProviderKind::AnthropicMessages => anthropic(
                json!({"type":"tool_use","id":"call","name":"exec_command","input":{}}),
                json!({"type":"input_json_delta","partial_json":args}),
                "tool_use",
            ),
        };
        let server = Server::start(vec![http(&body)]).await;
        let mut settings = config();
        settings.budget = ModelBudget::new(1, 1024 * 1024, 256 * 1024, 1024).unwrap();
        let mut engine = factory(kind, server.root()).create(settings, None).unwrap();
        let first_admission = admission();
        let (_, cancellation) = AbortHandle::new_pair();
        let EngineAction::Tools(batch) = engine
            .advance(
                input(1),
                Arc::new(Events::default()),
                first_admission.clone(),
                cancellation,
            )
            .await
            .unwrap()
        else {
            panic!("tool turn")
        };
        let results = batch
            .calls()
            .iter()
            .map(|call| {
                EngineToolResult::new(
                    call.slot(),
                    BoundedJson::new(json!({"stdout":"done"})).unwrap(),
                    false,
                )
            })
            .collect();
        let next_admission = admission();
        let (_, cancellation) = AbortHandle::new_pair();
        let EngineAction::Complete(done) = engine
            .advance(
                EngineInput {
                    next_step: StepId::from_bytes([2; 16]),
                    request: EngineRequest::ToolResults(
                        EngineToolResults::new(batch.step(), results).unwrap(),
                    ),
                },
                Arc::new(Events::default()),
                next_admission.clone(),
                cancellation,
            )
            .await
            .unwrap()
        else {
            panic!("local completed ceiling")
        };
        assert_eq!(done.stop, StopReason::MaxTurnRequests);
        assert_eq!(server.requests().len(), 1);
        assert_eq!(first_admission.requests().len(), 1);
        assert!(next_admission.requests().is_empty());
    }
}

#[tokio::test]
async fn actual_provider_http_waits_for_the_original_bounded_reservation_ack() {
    for kind in [ProviderKind::OpenAiChat, ProviderKind::AnthropicMessages] {
        let response = match kind {
            ProviderKind::OpenAiChat => chat_text("answer", true),
            ProviderKind::AnthropicMessages => anthropic(
                json!({"type":"text","text":""}),
                json!({"type":"text_delta","text":"answer"}),
                "end_turn",
            ),
        };
        let server = Server::start(vec![http(&response)]).await;
        let mut engine = engine(kind, server.root());
        let admission = admission();
        let (entered, release) = admission.pause_next();
        let (_, registration) = AbortHandle::new_pair();
        let work = engine.advance(
            input(1),
            Arc::new(Events::default()),
            admission.clone(),
            registration,
        );
        tokio::pin!(work);
        tokio::select! { result=&mut work=>panic!("reservation must wait: {result:?}"), _=entered.wait()=>{} }
        assert!(server.requests().is_empty());
        let requests = admission.requests();
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].run(), &config().run);
        assert_eq!(requests[0].step(), StepId::from_bytes([1; 16]));
        release.release();
        assert!(matches!(work.await.unwrap(), EngineAction::Complete(_)));
        assert_eq!(server.request_lengths(), vec![requests[0].request_bytes()]);
        let wire = &server.requests()[0].1;
        let cap = wire
            .get("max_tokens")
            .or_else(|| wire.get("max_completion_tokens"))
            .and_then(|n| n.as_u64());
        assert_eq!(cap, Some(requests[0].output_token_limit()));
    }
}

#[tokio::test]
async fn a_host_storage_refusal_keeps_its_typed_cause_and_sends_no_http() {
    use std::error::Error as _;
    let server = Server::start(vec![]).await;
    let mut engine = engine(ProviderKind::OpenAiChat, server.root());
    let admission = admission();
    admission.fail_next(EngineError::Admission(nocterm_core::SharedCause::new(
        std::io::Error::from(std::io::ErrorKind::StorageFull),
    )));
    let (_, registration) = AbortHandle::new_pair();
    let error = engine
        .advance(
            input(1),
            Arc::new(Events::default()),
            admission,
            registration,
        )
        .await
        .unwrap_err();
    let EngineError::Admission(cause) = error else {
        panic!("original host admission cause");
    };
    assert_eq!(
        cause
            .source()
            .unwrap()
            .downcast_ref::<std::io::Error>()
            .unwrap()
            .kind(),
        std::io::ErrorKind::StorageFull
    );
    assert!(server.requests().is_empty());
    assert!(engine.checkpoint(StreamRevision::default()).is_err());
}

#[tokio::test]
async fn current_host_authority_is_checked_again_after_the_reservation_ack() {
    let server = Server::start(vec![]).await;
    let mut engine = engine(ProviderKind::OpenAiChat, server.root());
    let admission = admission();
    admission.revoke_on_ack();
    let (_, registration) = AbortHandle::new_pair();
    assert!(matches!(
        engine
            .advance(
                input(1),
                Arc::new(Events::default()),
                admission,
                registration
            )
            .await,
        Err(EngineError::Interrupted)
    ));
    assert!(server.requests().is_empty());
}

#[tokio::test]
async fn stop_during_a_pending_reservation_prevents_the_http_send() {
    let server = Server::start(vec![]).await;
    let mut engine = engine(ProviderKind::OpenAiChat, server.root());
    let admission = admission();
    let (entered, _) = admission.pause_next();
    let (stop, registration) = AbortHandle::new_pair();
    let work = engine.advance(
        input(1),
        Arc::new(Events::default()),
        admission.clone(),
        registration,
    );
    tokio::pin!(work);
    tokio::select! { result=&mut work=>panic!("reservation must wait: {result:?}"), _=entered.wait()=>{} }
    stop.abort();
    assert!(matches!(work.await, Err(EngineError::Interrupted)));
    assert_eq!(admission.requests().len(), 1);
    assert!(server.requests().is_empty());
}

#[tokio::test]
async fn provider_revocation_during_reservation_does_not_wait_for_the_host_ack() {
    let server = Server::start(vec![]).await;
    let lease = ProviderLease::new(
        binding(ProviderKind::OpenAiChat, server.root()),
        Zeroizing::new(KEY.into()),
        std::time::Duration::from_secs(30),
    )
    .unwrap();
    let mut engine = RigEngineFactory::new(lease.clone())
        .unwrap()
        .create(config(), None)
        .unwrap();
    let admission = admission();
    let (entered, _) = admission.pause_next();
    let (_, registration) = AbortHandle::new_pair();
    let work = engine.advance(
        input(1),
        Arc::new(Events::default()),
        admission,
        registration,
    );
    tokio::pin!(work);
    tokio::select! { result=&mut work=>panic!("reservation must wait: {result:?}"), _=entered.wait()=>{} }
    lease.revoke();
    assert!(matches!(work.await, Err(EngineError::NeedsProviderAccess)));
    assert!(server.requests().is_empty());
}

#[tokio::test]
async fn engines_from_one_factory_have_independent_original_call_scopes() {
    let server = Server::start(vec![
        http(&chat_text("one", true)),
        http(&chat_text("two", true)),
    ])
    .await;
    let factory = factory(ProviderKind::OpenAiChat, server.root());
    let a = config();
    let mut b = config();
    let original = b.run.clone();
    b.run = RunBinding::new(
        original.scope().clone(),
        original.runner(),
        original.runner_epoch(),
        original.chat(),
        RunId::from_bytes([8; 16]),
        original.epoch(),
    );
    let admission_a = Arc::new(ScriptedModelAdmission::new(a.run.clone()));
    let admission_b = Arc::new(ScriptedModelAdmission::new(b.run.clone()));
    let mut a = factory.create(a, None).unwrap();
    let mut b = factory.create(b, None).unwrap();
    let (_, cancel_a) = AbortHandle::new_pair();
    let (_, cancel_b) = AbortHandle::new_pair();
    let (a, b) = futures::join!(
        a.advance(
            input(1),
            Arc::new(Events::default()),
            admission_a.clone(),
            cancel_a
        ),
        b.advance(
            input(1),
            Arc::new(Events::default()),
            admission_b.clone(),
            cancel_b
        )
    );
    assert!(matches!(a.unwrap(), EngineAction::Complete(_)));
    assert!(matches!(b.unwrap(), EngineAction::Complete(_)));
    assert_eq!(admission_a.requests().len(), 1);
    assert_eq!(admission_b.requests().len(), 1);
    assert_eq!(server.requests().len(), 2);
}

#[tokio::test]
async fn a_verified_tool_turn_waits_for_its_durable_exact_output_observation() {
    let arguments = r#"   {"terminal_id":"t","program":"true","args":[]}  "#;
    let server = Server::start(vec![http(&chat_tools(arguments, 1, true))]).await;
    let mut engine = engine(ProviderKind::OpenAiChat, server.root());
    let admission = admission();
    let (entered, release) = admission.pause_reconcile();
    let (_, registration) = AbortHandle::new_pair();
    let work = engine.advance(
        input(1),
        Arc::new(Events::default()),
        admission.clone(),
        registration,
    );
    tokio::pin!(work);
    tokio::select! { result=&mut work=>panic!("verified result must wait for observation ACK: {result:?}"), _=entered.wait()=>{} }
    assert_eq!(server.requests().len(), 1);
    assert!(admission.observations().is_empty());
    release.release();
    assert!(matches!(work.await.unwrap(), EngineAction::Tools(_)));
    let observations = admission.observations();
    assert_eq!(observations.len(), 1);
    assert_eq!(observations[0].0, admission.requests()[0]);
    assert_eq!(observations[0].1.output_bytes(), arguments.len());
    assert_eq!(observations[0].1.usage().output_tokens, Some(7));
}

#[tokio::test]
async fn a_failed_final_observation_exposes_no_tool_batch_or_checkpoint() {
    let arguments = r#"{"terminal_id":"t","program":"true","args":[]}"#;
    let server = Server::start(vec![http(&chat_tools(arguments, 1, true))]).await;
    let mut engine = engine(ProviderKind::OpenAiChat, server.root());
    let admission = admission();
    admission.fail_reconcile(EngineError::Admission(nocterm_core::SharedCause::new(
        std::io::Error::from(std::io::ErrorKind::StorageFull),
    )));
    let (_, registration) = AbortHandle::new_pair();
    assert!(matches!(
        engine
            .advance(
                input(1),
                Arc::new(Events::default()),
                admission.clone(),
                registration
            )
            .await,
        Err(EngineError::Admission(_))
    ));
    assert_eq!(admission.requests().len(), 1);
    assert!(admission.observations().is_empty());
    assert!(engine.checkpoint(StreamRevision::default()).is_err());
}

#[tokio::test]
async fn malformed_provider_finish_cannot_produce_a_refundable_observation() {
    let arguments = r#"{"terminal_id":"t","program":"true","args":[]}"#;
    let server = Server::start(vec![http(&chat_tools(arguments, 1, false))]).await;
    let mut engine = engine(ProviderKind::OpenAiChat, server.root());
    let admission = admission();
    let (_, registration) = AbortHandle::new_pair();
    assert!(
        engine
            .advance(
                input(1),
                Arc::new(Events::default()),
                admission.clone(),
                registration
            )
            .await
            .is_err()
    );
    assert_eq!(admission.requests().len(), 1);
    assert!(admission.observations().is_empty());
}

#[tokio::test]
async fn source_revocation_at_observation_ack_still_refuses_final_action() {
    let server = Server::start(vec![http(&chat_text("answer", true))]).await;
    let mut engine = engine(ProviderKind::OpenAiChat, server.root());
    let admission = admission();
    admission.revoke_on_reconcile_ack();
    let (_, registration) = AbortHandle::new_pair();
    assert!(matches!(
        engine
            .advance(
                input(1),
                Arc::new(Events::default()),
                admission.clone(),
                registration
            )
            .await,
        Err(EngineError::Interrupted)
    ));
    assert_eq!(admission.observations().len(), 1);
    assert!(engine.checkpoint(StreamRevision::default()).is_err());
}

#[tokio::test]
async fn provider_revoke_interrupts_a_pending_final_observation() {
    let server = Server::start(vec![http(&chat_text("answer", true))]).await;
    let lease = ProviderLease::new(
        binding(ProviderKind::OpenAiChat, server.root()),
        Zeroizing::new(KEY.into()),
        std::time::Duration::from_secs(30),
    )
    .unwrap();
    let mut engine = RigEngineFactory::new(lease.clone())
        .unwrap()
        .create(config(), None)
        .unwrap();
    let admission = admission();
    let (entered, _) = admission.pause_reconcile();
    let (_, registration) = AbortHandle::new_pair();
    let work = engine.advance(
        input(1),
        Arc::new(Events::default()),
        admission.clone(),
        registration,
    );
    tokio::pin!(work);
    tokio::select! { result=&mut work=>panic!("final observation must wait: {result:?}"), _=entered.wait()=>{} }
    lease.revoke();
    assert!(matches!(work.await, Err(EngineError::NeedsProviderAccess)));
    assert!(admission.observations().is_empty());
}

#[tokio::test]
async fn source_revocation_during_final_usage_ack_refuses_a_tool_action() {
    let arguments = r#"{"terminal_id":"t","program":"true","args":[]}"#;
    let server = Server::start(vec![http(&chat_tools(arguments, 1, true))]).await;
    let mut engine = engine(ProviderKind::OpenAiChat, server.root());
    let admission = admission();
    let events = Arc::new(Events::default());
    let (entered, release) = events.pause_next();
    let (_, registration) = AbortHandle::new_pair();
    let result = {
        let work = engine.advance(input(1), events, admission.clone(), registration);
        tokio::pin!(work);
        tokio::select! { result=&mut work=>panic!("usage ACK must wait: {result:?}"), _=entered.wait()=>{} }
        assert_eq!(admission.observations().len(), 1);
        admission.revoke();
        release.release();
        work.await
    };
    assert!(matches!(result, Err(EngineError::Interrupted)));
    assert!(engine.checkpoint(StreamRevision::default()).is_err());
}

#[tokio::test]
async fn an_inline_image_over_the_full_request_limit_is_typed_before_any_http_or_reservation() {
    use base64::{Engine as _, engine::general_purpose::STANDARD};
    use nocterm_ai::model::content::{Content, Image};
    // This boundary checks MIME and request encoding; pixel admission is tested upstream.
    let mut bytes = b"\x89PNG\r\n\x1a\n".to_vec();
    bytes.resize(1024 * 1024, 0);
    for kind in [ProviderKind::OpenAiChat, ProviderKind::AnthropicMessages] {
        let server = Server::start(vec![]).await;
        let mut engine = engine(kind, server.root());
        let admission = admission();
        let input = EngineInput {
            next_step: StepId::from_bytes([1; 16]),
            request: EngineRequest::Prompt(
                EnginePrompt::new(vec![Content::Image(Image::new(
                    STANDARD.encode(&bytes),
                    "image/png",
                ))])
                .unwrap(),
            ),
        };
        let (_, cancellation) = AbortHandle::new_pair();
        assert!(matches!(
            engine
                .advance(
                    input,
                    Arc::new(Events::default()),
                    admission.clone(),
                    cancellation
                )
                .await,
            Err(EngineError::RequestTooLarge { limit: 1_048_576 })
        ));
        assert!(server.requests().is_empty());
        assert!(
            admission.requests().is_empty(),
            "no request was billed or reserved"
        );
    }
}
