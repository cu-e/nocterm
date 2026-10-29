use super::{fixtures::*, *};
use nocterm_cloud_model::{
    engine::{EngineAction, EngineCheckpoint, EngineToolResult, EngineToolResults},
    identity::{StepId, StreamRevision},
    payload::{BoundedJson, OpaqueBytes},
};
use serde_json::json;

#[tokio::test]
async fn reverse_same_named_tool_results_keep_original_call_ids_after_restore() {
    let args = r#"{"terminal_id":"session","program":"printf","args":["hello"]}"#;
    let server = Server::start(vec![
        http(&chat_tools(args, 2, true)),
        http(&chat_text("finished", true)),
    ])
    .await;
    let factory = factory(ProviderKind::OpenAiChat, server.root());
    let mut original = factory.create(config(), None).unwrap();
    let EngineAction::Tools(batch) = advance(original.as_mut()).await.unwrap() else {
        panic!("tools");
    };
    let saved = original.checkpoint(StreamRevision::from_u64(12)).unwrap();
    drop(original);
    let mut resumed = factory.create(config(), Some(saved)).unwrap();
    let results = batch
        .calls()
        .iter()
        .enumerate()
        .rev()
        .map(|(index, call)| {
            EngineToolResult::new(
                call.slot(),
                BoundedJson::new(json!({"result":format!("answer-{index}")})).unwrap(),
                index == 1,
            )
        })
        .collect();
    let input = EngineInput {
        next_step: StepId::from_bytes([2; 16]),
        request: EngineRequest::ToolResults(EngineToolResults::new(batch.step(), results).unwrap()),
    };
    let (_, registration) = AbortHandle::new_pair();
    assert!(matches!(
        resumed
            .advance(
                input,
                Arc::new(Events::default()),
                admission(),
                registration
            )
            .await
            .unwrap(),
        EngineAction::Complete(_)
    ));
    let requests = server.requests();
    let messages = requests[1].1["messages"].as_array().unwrap();
    let replies: Vec<_> = messages.iter().filter(|m| m["role"] == "tool").collect();
    assert_eq!(replies.len(), 2);
    assert_eq!(replies[0]["tool_call_id"], "call-0");
    assert_eq!(replies[1]["tool_call_id"], "call-1");
    assert!(replies[0]["content"].as_str().unwrap().contains("answer-0"));
    assert!(replies[1]["content"].as_str().unwrap().contains("answer-1"));
}
#[tokio::test]
async fn checkpoint_refuses_unknown_sdk_format_wrong_model_budget_and_original_run() {
    let server = Server::start(vec![http(&chat_text("complete", true))]).await;
    let factory = factory(ProviderKind::OpenAiChat, server.root());
    let mut engine = factory.create(config(), None).unwrap();
    advance(engine.as_mut()).await.unwrap();
    let saved = engine.checkpoint(StreamRevision::from_u64(10)).unwrap();
    let mut changed = config();
    changed.budget = ModelBudget::new(1, 1024 * 1024, 256 * 1024, 1024).unwrap();
    assert!(factory.create(changed, Some(saved.clone())).is_err());
    changed = config();
    changed.run = changed.run.with_epoch(RunEpoch::new(2).unwrap());
    assert!(factory.create(changed, Some(saved.clone())).is_err());
    let mut body: serde_json::Value = serde_json::from_slice(saved.bytes().as_bytes()).unwrap();
    body["state"]["sdk"]["format"] = json!(u32::MAX);
    let unknown = EngineCheckpoint::new(
        saved.run().clone(),
        saved.format().clone(),
        saved.journal_revision(),
        OpaqueBytes::new(serde_json::to_vec(&body).unwrap()).unwrap(),
    )
    .unwrap();
    assert!(matches!(
        factory.create(config(), Some(unknown)),
        Err(EngineError::UnsupportedFormat)
    ));
    assert!(
        saved
            .require_context(&config().run, StreamRevision::from_u64(11))
            .is_err()
    );
}
#[tokio::test]
async fn cancellation_interrupts_backpressured_progress_without_success_or_checkpoint() {
    struct Blocked;
    impl EngineEventSink for Blocked {
        fn publish(
            &self,
            _: StepId,
            _: EngineEvent,
        ) -> BoxFuture<'static, Result<(), EngineError>> {
            futures::future::pending().boxed()
        }
    }
    let server = Server::start(vec![http(&chat_text("partial", true))]).await;
    let mut engine = engine(ProviderKind::OpenAiChat, server.root());
    let (handle, registration) = AbortHandle::new_pair();
    let result = engine.advance(input(1), Arc::new(Blocked), admission(), registration);
    tokio::pin!(result);
    assert!(
        tokio::time::timeout(std::time::Duration::from_millis(100), &mut result)
            .await
            .is_err()
    );
    handle.abort();
    assert!(matches!(result.await, Err(EngineError::Interrupted)));
}
#[tokio::test]
async fn unknown_usage_consumes_reservation_and_blocks_an_unbudgeted_second_call() {
    let args = r#"{"terminal_id":"session","program":"printf"}"#;
    let body = chat_tools(args, 1, true).replace(&data(json!({"choices":[],"usage":{"prompt_tokens":11,"completion_tokens":7,"total_tokens":18}})), "");
    let server = Server::start(vec![http(&body)]).await;
    let mut engine = engine(ProviderKind::OpenAiChat, server.root());
    let EngineAction::Tools(batch) = advance(engine.as_mut()).await.unwrap() else {
        panic!("tools");
    };
    let result = EngineToolResult::new(
        batch.calls()[0].slot(),
        BoundedJson::new(json!({"ok":true})).unwrap(),
        false,
    );
    let input = EngineInput {
        next_step: StepId::from_bytes([2; 16]),
        request: EngineRequest::ToolResults(
            EngineToolResults::new(batch.step(), vec![result]).unwrap(),
        ),
    };
    let (_, registration) = AbortHandle::new_pair();
    assert!(matches!(
        engine
            .advance(
                input,
                Arc::new(Events::default()),
                admission(),
                registration
            )
            .await,
        Err(EngineError::BudgetExceeded)
    ));
    assert_eq!(server.requests().len(), 1);
}
#[tokio::test]
async fn reused_host_step_is_refused_even_with_valid_pending_tool_answers() {
    let server = Server::start(vec![http(&chat_tools(
        r#"{"terminal_id":"session","program":"printf"}"#,
        1,
        true,
    ))])
    .await;
    let mut engine = engine(ProviderKind::OpenAiChat, server.root());
    let EngineAction::Tools(batch) = advance(engine.as_mut()).await.unwrap() else {
        panic!("tools");
    };
    let results = EngineToolResults::new(
        batch.step(),
        vec![EngineToolResult::new(
            batch.calls()[0].slot(),
            BoundedJson::new(json!({})).unwrap(),
            false,
        )],
    )
    .unwrap();
    let (_, registration) = AbortHandle::new_pair();
    let input = EngineInput {
        next_step: batch.step(),
        request: EngineRequest::ToolResults(results),
    };
    assert!(matches!(
        engine
            .advance(
                input,
                Arc::new(Events::default()),
                admission(),
                registration
            )
            .await,
        Err(EngineError::InvalidStream)
    ));
    assert_eq!(server.requests().len(), 1);
}

#[tokio::test]
async fn dropping_an_incomplete_advance_refuses_partial_checkpoint_and_implicit_retry() {
    struct Blocked;
    impl EngineEventSink for Blocked {
        fn publish(
            &self,
            _: StepId,
            _: EngineEvent,
        ) -> BoxFuture<'static, Result<(), EngineError>> {
            futures::future::pending().boxed()
        }
    }
    let server = Server::start(vec![http(&chat_text("partial", true))]).await;
    let mut engine = engine(ProviderKind::OpenAiChat, server.root());
    let (_, registration) = AbortHandle::new_pair();
    {
        let work = engine.advance(input(1), Arc::new(Blocked), admission(), registration);
        tokio::pin!(work);
        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(100), &mut work)
                .await
                .is_err()
        );
    }
    assert!(matches!(
        engine.checkpoint(Default::default()),
        Err(EngineError::Interrupted)
    ));
    assert!(matches!(
        advance(engine.as_mut()).await,
        Err(EngineError::Interrupted)
    ));
}

#[tokio::test]
async fn checkpoint_binds_independent_calls_and_refuses_the_previous_coupled_sdk_format() {
    let server = Server::start(vec![http(&chat_text("answer", true))]).await;
    let factory = factory(ProviderKind::OpenAiChat, server.root());
    let mut engine = factory.create(config(), None).unwrap();
    advance(engine.as_mut()).await.unwrap();
    let saved = engine.checkpoint(StreamRevision::from_u64(1)).unwrap();
    let mut changed = config();
    changed.budget = changed.budget.with_calls(1).unwrap();
    assert!(matches!(
        factory.create(changed, Some(saved.clone())),
        Err(EngineError::UnsupportedFormat)
    ));
    let legacy = EngineCheckpoint::new(
        saved.run().clone(),
        NamespaceId::new("nocterm.rig.a0efbf90.v1").unwrap(),
        saved.journal_revision(),
        saved.bytes().clone(),
    )
    .unwrap();
    assert!(matches!(
        factory.create(config(), Some(legacy)),
        Err(EngineError::UnsupportedFormat)
    ));
}
#[tokio::test]
async fn independent_http_call_limit_stops_before_a_second_provider_request() {
    let server = Server::start(vec![http(&chat_tools(
        r#"{"terminal_id":"session","program":"printf"}"#,
        1,
        true,
    ))])
    .await;
    let factory = factory(ProviderKind::OpenAiChat, server.root());
    let mut configuration = config();
    configuration.budget = configuration.budget.with_calls(1).unwrap();
    let mut engine = factory.create(configuration, None).unwrap();
    let EngineAction::Tools(batch) = advance(engine.as_mut()).await.unwrap() else {
        panic!("tool batch")
    };
    let result = EngineToolResult::new(
        batch.calls()[0].slot(),
        BoundedJson::new(json!({"ok":true})).unwrap(),
        false,
    );
    let (_, registration) = AbortHandle::new_pair();
    let next = EngineInput {
        next_step: StepId::from_bytes([2; 16]),
        request: EngineRequest::ToolResults(
            EngineToolResults::new(batch.step(), vec![result]).unwrap(),
        ),
    };
    assert!(matches!(
        engine
            .advance(next, Arc::new(Events::default()), admission(), registration)
            .await,
        Err(EngineError::BudgetExceeded)
    ));
    assert_eq!(server.requests().len(), 1);
}
