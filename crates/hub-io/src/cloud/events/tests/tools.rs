//! Public canonical fixture digests validate the actual native call mapping.
use super::*;
use nocterm_ai::{TerminalCall, tool_display::ToolDisplay};
use nocterm_cloud_model::{run::ToolIntent, tool::ArgumentFingerprint};

pub(super) fn calls() -> Vec<(TerminalCall, &'static str)> {
    [
        ("list_terminals", r#"{}"#, "366f56d5da331cac345d6e6672cf976e8fbfdc5ed2518bb67302374f3558905e"),
        ("open_terminal", r#"{"server_id":"server"}"#, "e3b82792f66b4afaf588dc2d208d6e34b8efe387397fa0e0c4ae6b122987bc7c"),
        ("read_terminal", r#"{"terminal_id":"terminal","lines":10}"#, "0ffc9b812237c2631b1ed303b32b679cc87538a34ae5249004ec7ce7256e0797"),
        ("send_input", r#"{"terminal_id":"terminal","text":"literal input","press_enter":true}"#, "b4d3654bf3b8890d8177d8a496a42ef12006b99fd38f109b64654d3e6742b2b0"),
        ("run_command", r#"{"terminal_id":"terminal","command":"pwd","timeout_ms":1000}"#, "0eb50ed2268a8bf4b3b3f50b1e60bd65b604542b6aacb1ce19a46abaa341632e"),
        ("exec_command", r#"{"terminal_id":"terminal","program":"/bin/printf","args":["%s","$(literal); *"],"stdin":"","yield_ms":0}"#, "e43ad7d1d1b39fd48672f44698faa39a3bedbee82184a7f3734cfdfe58b02489"),
        ("read_command", r#"{"terminal_id":"terminal","command_id":"command","yield_ms":50}"#, "854abae88b35cdfe8e36e8aff6760734aae4fc636e1e1a5ed96e5046f89d3085"),
        ("cancel_command", r#"{"terminal_id":"terminal","command_id":"command"}"#, "028a90da9afbdbad28ebc945939095fe5d5d252cce7ab81eea86db023b657b96"),
    ].into_iter().map(|(name, arguments, digest)| {
        let spec = nocterm_ai::tools::catalog::get(name).unwrap();
        ((spec.parse)(serde_json::from_str(arguments).unwrap()).unwrap(), digest)
    }).collect()
}
#[test]
fn runner_events_all_eight_native_tools_match_public_canonical_fixtures() {
    let fixture = Fixture::new();
    let encoder = fixture.encoder();
    let work = Arc::new(Fixture::budget(8 * 1024 * 1024));
    let fingerprints = RunnerArgumentFingerprint::new(work.clone());
    let authority = fixture.authority();
    for (index, (call, expected)) in calls().into_iter().enumerate() {
        let digest = fingerprints.fingerprint(&call).unwrap();
        let hex: String = digest
            .as_bytes()
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect();
        assert_eq!(hex, expected, "{}", call.name());
        let intent = ToolIntent::new(
            IntentId::from_bytes([u8::try_from(index).unwrap(); 16]),
            StepId::from_bytes([20; 16]),
            authority.clone(),
            digest,
            &call,
        )
        .unwrap();
        let display = ToolDisplay {
            version: 2,
            tool: call.name().into(),
            arguments: call.arguments().unwrap(),
            destination: Some("original target".into()),
            redacted: false,
            outcome: None,
        };
        let event = CoordinatorEvent::ToolIntent {
            intent,
            call,
            display,
        };
        let next = JournalPosition::default().next(1, 1, false).unwrap();
        let publication =
            CoordinatorEventEncoder::encode(&encoder, &fixture.lease, next, &event, work.as_ref())
                .unwrap();
        let context = crate::cloud::runner_transport::record_context(
            publication.stream(),
            publication.key_epoch(),
            publication.anchor(),
        )
        .unwrap();
        let public = fixture.identity.public_identity().unwrap();
        let opened = open_event(
            &fixture.key,
            public.signing_key(),
            &context,
            publication.sealed().as_bytes(),
        )
        .unwrap();
        let record = opened.record().unwrap();
        let decoded = CloudEvent::from_record(&record).unwrap();
        let CloudEventPayload::ToolIntent(intent) = decoded.payload else {
            panic!("wrong event")
        };
        assert_eq!(intent.arguments_digest, *digest.as_bytes());
        assert_eq!(intent.arguments.digest().unwrap(), *digest.as_bytes());
        assert_eq!(intent.display.destination, Some("original target"));
        assert_eq!(intent.execution.run.id, *fixture.run.run().as_bytes());
    }
}
