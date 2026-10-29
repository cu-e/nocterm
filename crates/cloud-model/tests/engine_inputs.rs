mod support;
use nocterm_ai::{
    ExecCommand, TerminalCall,
    model::content::{Content, Image},
};
use nocterm_cloud_model::{engine::*, identity::*, payload::*, run::*, tool::*};
use nocterm_core::SharedCause;
use serde_json::json;
use std::{error::Error, fmt};
use support::*;

const SECRET: &str = "private-prompt-argument-stdout";
fn call() -> TerminalCall {
    TerminalCall::ExecCommand(ExecCommand {
        terminal_id: "t".into(),
        program: "printf".into(),
        args: vec![SECRET.into()],
        stdin: Some(SECRET.into()),
        timeout_ms: None,
        yield_ms: None,
    })
}
fn permit() -> DispatchPermit {
    let binding = run(1, 1, 1, 1);
    let mut state = RunState::new(
        binding.clone(),
        target(1),
        WriterEpoch::new(1).unwrap(),
        Revision::default(),
    );
    let intent = ToolIntent::new(
        IntentId::from_bytes([1; 16]),
        StepId::from_bytes([1; 16]),
        authority(binding.clone()),
        fingerprint(1),
        &call(),
    )
    .unwrap();
    let stamp = |revision| {
        CommitStamp::new(
            binding.clone(),
            state_writer(),
            Revision::from_u64(revision),
        )
    };
    state
        .prepare_intent(intent.clone(), state.revision())
        .unwrap();
    state.intent_committed(intent.id(), stamp(1)).unwrap();
    state.dispatch_committed(intent.id(), stamp(2)).unwrap()
}
fn state_writer() -> WriterEpoch {
    WriterEpoch::new(1).unwrap()
}

#[test]
fn json_admission_counts_escaped_bytes_rather_than_raw_character_count() {
    let count = (MAX_TOOL_RESULT_BYTES - 2) / 6;
    let bounded = BoundedJson::new(json!("\0".repeat(count))).unwrap();
    assert!(bounded.encoded_bytes() <= MAX_TOOL_RESULT_BYTES);
    assert!(matches!(
        BoundedJson::new(json!("\0".repeat(count + 1))),
        Err(PayloadError::Capacity { .. })
    ));
}

#[test]
fn prompt_content_cannot_add_native_resource_or_url_fetch_capabilities() {
    let prompt = EnginePrompt::new(vec![Content::text(SECRET)]).unwrap();
    assert_eq!(prompt.content().len(), 1);
    let mut external = Image::new("bytes", "image/png");
    external.uri = Some("file:///private/path".into());
    assert!(matches!(
        EnginePrompt::new(vec![Content::Image(external)]),
        Err(EngineError::UnsupportedContent)
    ));
    assert!(matches!(
        EnginePrompt::new(vec![]),
        Err(EngineError::InvalidPrompt)
    ));
    assert!(matches!(
        EnginePrompt::new(vec![Content::text("x"); 257]),
        Err(EngineError::InvalidPrompt)
    ));
}

#[test]
fn progress_and_checkpoint_values_enforce_boundaries_without_disclosing_contents() {
    let at = StreamRevision::from_u64(3);
    let checkpoint = EngineCheckpoint::new(
        run(1, 1, 1, 1),
        NamespaceId::new("fixture-v1").unwrap(),
        at,
        OpaqueBytes::new(SECRET.as_bytes()).unwrap(),
    )
    .unwrap();
    assert!(checkpoint.require_context(&run(1, 1, 1, 1), at).is_ok());
    assert!(matches!(
        checkpoint.require_context(&run(2, 1, 1, 1), at),
        Err(EngineError::InvalidStream)
    ));
    assert!(matches!(
        checkpoint.require_context(&run(1, 1, 1, 1), StreamRevision::from_u64(4)),
        Err(EngineError::InvalidStream)
    ));
    assert!(EngineText::new(TextChannel::Assistant, "x".repeat(MAX_PROGRESS_BYTES)).is_ok());
    assert!(matches!(
        EngineText::new(TextChannel::Thought, "x".repeat(MAX_PROGRESS_BYTES + 1)),
        Err(EngineError::Payload(PayloadError::Capacity { .. }))
    ));
    assert!(!format!("{checkpoint:?}").contains(SECRET));
    assert!(
        !format!(
            "{:?}",
            EngineText::new(TextChannel::Thought, SECRET.into()).unwrap()
        )
        .contains(SECRET)
    );
}

#[test]
fn host_slots_correlate_same_named_calls_even_when_provider_ids_repeat() {
    let step = StepId::from_bytes([1; 16]);
    let a = ToolSlot::new(step, 0).unwrap();
    let b = ToolSlot::new(step, 1).unwrap();
    let first =
        EngineToolCall::new(a, Some(NamespaceId::new("repeated").unwrap()), call()).unwrap();
    let second =
        EngineToolCall::new(b, Some(NamespaceId::new("repeated").unwrap()), call()).unwrap();
    assert_eq!(
        EngineToolBatch::new(step, vec![first.clone(), second])
            .unwrap()
            .calls()
            .len(),
        2
    );
    assert!(matches!(
        EngineToolBatch::new(step, vec![first.clone(), first]),
        Err(EngineError::InvalidStream)
    ));
    let results = EngineToolResults::new(
        step,
        vec![
            EngineToolResult::new(b, BoundedJson::new(json!("second")).unwrap(), false),
            EngineToolResult::new(a, BoundedJson::new(json!("first")).unwrap(), false),
        ],
    )
    .unwrap();
    assert_eq!(results.results()[0].slot(), b);
    assert_eq!(results.results()[1].slot(), a);
    assert!(ToolSlot::new(step, MAX_ENGINE_TOOL_CALLS as u32).is_err());
}

#[test]
fn unknown_usage_is_preserved_and_does_not_become_observed_zero() {
    let unknown = UsageObservation::default();
    assert_eq!(unknown.input_tokens, None);
    assert_eq!(unknown.cost_micros, None);
    assert_ne!(
        unknown,
        UsageObservation {
            input_tokens: Some(0),
            ..unknown
        }
    );
}

#[test]
fn malformed_or_replaced_tool_arguments_never_become_a_scoped_request() {
    assert!(matches!(
        ScopedToolRequest::new(permit(), call(), fingerprint(2)),
        Err(ToolExecutionError::ArgumentMismatch)
    ));
    assert!(matches!(
        ScopedToolRequest::new(permit(), TerminalCall::ListTerminals, fingerprint(1)),
        Err(ToolExecutionError::ArgumentMismatch)
    ));
    let request = ScopedToolRequest::new(permit(), call(), fingerprint(1)).unwrap();
    let result = ToolExecutionResult::completed(
        &request,
        BoundedJson::new(json!({"stdout":SECRET})).unwrap(),
        Some("private-host".into()),
    );
    assert_eq!(result.intent(), request.permit().intent().id());
    assert_eq!(
        result.display().destination.as_deref(),
        Some("private-host")
    );
    for debug in [
        format!("{request:?}"),
        format!("{result:?}"),
        format!(
            "{:?}",
            EnginePrompt::new(vec![Content::text(SECRET)]).unwrap()
        ),
        format!(
            "{:?}",
            EngineToolCall::new(
                ToolSlot::new(StepId::from_bytes([1; 16]), 0).unwrap(),
                None,
                call()
            )
            .unwrap()
        ),
    ] {
        assert!(!debug.contains(SECRET));
        assert!(!debug.contains("private-host"));
    }
}

#[derive(Debug)]
struct PrivateFailure;
impl fmt::Display for PrivateFailure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(SECRET)
    }
}
impl Error for PrivateFailure {}

#[test]
fn native_error_causes_remain_typed_without_default_debug_or_display_disclosure() {
    for error in [
        EngineError::Provider(SharedCause::new(PrivateFailure)),
        EngineError::Admission(SharedCause::new(PrivateFailure)),
    ] {
        assert!(!format!("{error:?} {error}").contains(SECRET));
        assert!(
            error
                .source()
                .unwrap()
                .source()
                .unwrap()
                .is::<PrivateFailure>()
        );
    }
    let error = ToolExecutionError::Execution(SharedCause::new(PrivateFailure));
    assert!(!format!("{error:?} {error}").contains(SECRET));
    assert!(
        error
            .source()
            .unwrap()
            .source()
            .unwrap()
            .is::<PrivateFailure>()
    );
}

#[test]
fn per_model_limits_refuse_zero_or_unbounded_inputs_before_engine_work() {
    for (turns, input, output, tokens) in [
        (0, 1, 1, 1),
        (1, 0, 1, 1),
        (1, 1, 0, 1),
        (1, 1, 1, 0),
        (1025, 1, 1, 1),
        (1, MAX_LOGICAL_PAYLOAD_BYTES + 1, 1, 1),
        (1, 1, 1, 1_000_001),
    ] {
        assert!(matches!(
            ModelBudget::new(turns, input, output, tokens),
            Err(EngineError::BudgetExceeded)
        ));
    }
}

#[test]
fn model_call_requests_describe_the_exact_original_bounded_http_attempt() {
    let run = run(1, 1, 1, 1);
    let step = StepId::from_bytes([9; 16]);
    let request = ModelCallRequest::new(
        run.clone(),
        step,
        MAX_LOGICAL_PAYLOAD_BYTES,
        MAX_MODEL_OUTPUT_TOKENS,
    )
    .unwrap();
    assert_eq!(request.run(), &run);
    assert_eq!(request.step(), step);
    assert_eq!(request.request_bytes(), MAX_LOGICAL_PAYLOAD_BYTES);
    assert_eq!(request.output_token_limit(), MAX_MODEL_OUTPUT_TOKENS);
    for (bytes, tokens) in [
        (0, 1),
        (1, 0),
        (MAX_LOGICAL_PAYLOAD_BYTES + 1, 1),
        (1, MAX_MODEL_OUTPUT_TOKENS + 1),
    ] {
        assert!(matches!(
            ModelCallRequest::new(run.clone(), step, bytes, tokens),
            Err(EngineError::BudgetExceeded)
        ));
    }
}
