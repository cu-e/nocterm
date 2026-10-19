//! Own presentation behavior; provider spellings are tested in the adapter.
use super::*;
use crate::model::{
    controls::Presence,
    ids::ToolId,
    literal::{LiteralEvidence, LiteralIdentity, TextRange},
    notice::SessionUpdate,
    tools::ToolRecord,
};
fn plain() -> ToolRecord {
    let title = "3/run_command".to_owned();
    let owner = TextRange::supplied(&title, &title[..1]).unwrap();
    let tool = TextRange::supplied(&title, &title[2..]).unwrap();
    ToolRecord {
        id: ToolId::new("c1"),
        title,
        name: None,
        kind: Default::default(),
        status: Default::default(),
        output: Vec::new(),
        locations: Vec::new(),
        arguments: None,
        result: None,
        extensions: None,
        claims: Default::default(),
        literal: LiteralEvidence {
            title: LiteralIdentity::Registered { owner, tool },
            ..Default::default()
        },
        client_display: None,
    }
}
#[test]
fn local_metadata_survives_history_and_provider_metadata_is_removed() {
    let authority = crate::model::retention::RetentionAuthority::new(usize::MAX).unwrap();
    let mut call = plain();
    let request = parse(
        "run_command",
        json!({"terminal_id":"t1","command":"echo hello"}),
    )
    .unwrap();
    let display = ToolDisplay::new(
        &authority,
        &request,
        Some("Production · root@actual.example:22".into()),
    )
    .unwrap();
    display.attach(&mut call);
    let restored: ToolRecord =
        crate::model::decode::Decode::decode(&authority, serde_json::to_value(&call).unwrap())
            .unwrap();
    assert!(header(&restored).starts_with("Production · root@actual.example:22 · Run command"));
    let mut incoming = SessionUpdate::ToolStarted(call);
    strip_provider_meta(&mut incoming);
    let SessionUpdate::ToolStarted(incoming) = incoming else {
        unreachable!()
    };
    assert!(ToolDisplay::from_call(&incoming).is_none());
    assert!(header(&incoming).starts_with("Nocterm · Run command"));
    assert!(!header(&incoming).contains("actual.example"));
}

#[test]
fn redaction_expansion_keeps_a_verified_display_without_weakening_execution_validation() {
    let authority = crate::model::retention::RetentionAuthority::new(usize::MAX).unwrap();
    let command = "password=1 ".repeat(1400);
    assert!(command.len() < crate::MAX_INPUT_BYTES);
    let request = parse("run_command", json!({"terminal_id":"t1","command":command})).unwrap();
    let mut display = ToolDisplay::new(&authority, &request, Some("Original host".into())).unwrap();
    display.redact().unwrap();
    assert!(display.arguments["command"].as_str().unwrap().len() > crate::MAX_INPUT_BYTES);
    assert!(parse("run_command", display.arguments.as_value().clone()).is_none());
    let mut call = plain();
    display.attach(&mut call);
    let saved = ToolDisplay::from_call(&call).unwrap();
    assert_eq!(saved.redacted.value(), Some(&true));
    assert!(saved.title().starts_with("Original host"));
    assert!(saved.display_request().is_some());
    assert!(!saved.arguments.to_string().contains("password=1"));
}

#[test]
fn v1_metadata_and_unparseable_v2_requests_remain_readable() {
    let authority = crate::model::retention::RetentionAuthority::new(usize::MAX).unwrap();
    let mut call = plain();
    call.client_display = Some(crate::model::shared_display::SharedDisplay::from_untracked(
        ToolDisplay {
            version: 1,
            tool: "list_terminals".into(),
            arguments: crate::model::json::OwnedJson::from_untracked(&authority, json!({}))
                .unwrap(),
            destination: Presence::Null,
            redacted: Presence::Absent,
            outcome: Presence::Absent,
        },
    ));
    assert!(
        ToolDisplay::from_call(&call)
            .unwrap()
            .outcome
            .value()
            .is_none()
    );
    let display =
        ToolDisplay::requested(&authority, "unknown_tool".into(), json!({"bad":true})).unwrap();
    display.attach(&mut call);
    assert!(
        ToolDisplay::from_call(&call)
            .unwrap()
            .display_request()
            .is_none()
    );
}

#[test]
fn outcome_budget_counts_serialized_escaping_for_success_and_error() {
    let authority = crate::model::retention::RetentionAuthority::new(usize::MAX).unwrap();
    let mut display =
        ToolDisplay::requested(&authority, "list_terminals".into(), json!({})).unwrap();
    display.finish(Ok(json!({"context":"ok"}))).unwrap();
    assert!(matches!(
        display.outcome,
        Presence::Value(ToolOutcome::Succeeded(_))
    ));
    display.finish(Ok(json!("\n".repeat(140_000)))).unwrap();
    assert!(display.outcome.value().is_none());
    display.finish(Err("password=secret".into())).unwrap();
    let Presence::Value(ToolOutcome::Failed(error)) = &display.outcome else {
        panic!("error outcome");
    };
    assert!(!error.contains("secret"));
    display.finish(Err("\n".repeat(140_000).into())).unwrap();
    assert!(display.outcome.value().is_none());
}

#[test]
fn outcome_budget_accepts_the_exact_boundary_and_rejects_one_more_byte() {
    let authority = crate::model::retention::RetentionAuthority::new(usize::MAX).unwrap();
    let limit = 256 * 1024;
    for success in [false, true] {
        let empty = if success {
            ToolOutcome::Succeeded(
                crate::model::json::OwnedJson::from_untracked(&authority, json!("")).unwrap(),
            )
        } else {
            ToolOutcome::Failed(String::new())
        };
        let overhead = serde_json::to_vec(&empty).unwrap().len();
        let mut display =
            ToolDisplay::requested(&authority, "list_terminals".into(), json!({})).unwrap();
        for extra in [0, 1] {
            let text = "x".repeat(limit - overhead + extra);
            let result = if success {
                Ok(json!(text))
            } else {
                Err(text.into())
            };
            display.finish(result).unwrap();
            assert_eq!(display.outcome.value().is_some(), extra == 0);
            if let Presence::Value(outcome) = &display.outcome {
                assert_eq!(serde_json::to_vec(outcome).unwrap().len(), limit);
            }
        }
    }
}
