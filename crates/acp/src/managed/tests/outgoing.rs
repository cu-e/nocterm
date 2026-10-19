//! Original SDK request shapes are preserved without constructing SDK DTO graphs.
use super::fixture;
use crate::managed::outgoing;
use agent_client_protocol::managed::{Envelope, MessageId};
use agent_client_protocol_schema::v1 as wire;
use nocterm_ai::model::{ids::*, session::*};
fn payload(envelope: Envelope<crate::managed::budget::SourceBudget>) -> serde_json::Value {
    serde_json::from_str(envelope.payload().supplied().unwrap().text()).unwrap()
}
fn assert_open_matches_sdk(budget: &crate::managed::budget::SourceBudget) {
    let endpoints = vec![
        ExternalToolEndpoint::LocalCommand {
            name: "local Ω".into(),
            launch: nocterm_ai::BridgeLaunch {
                program: "/tmp/helper".into(),
                args: vec!["a b".into()],
                env: vec![("A".into(), "B".into())],
            },
        },
        ExternalToolEndpoint::Http {
            name: "http".into(),
            endpoint: "http://localhost".into(),
            headers: vec![("H".into(), "V".into())],
        },
        ExternalToolEndpoint::EventStream {
            name: "sse".into(),
            endpoint: "http://localhost/events".into(),
            headers: vec![],
        },
    ];
    let request = SessionOpen {
        working_directory: "/tmp/Ω".into(),
        endpoints,
    };
    assert_eq!(
        payload(
            outgoing::open(
                budget,
                &MessageId::Number(2),
                "session/new",
                &request.working_directory,
                &request.endpoints,
                None
            )
            .unwrap()
        ),
        serde_json::to_value(crate::mapping::outbound::open(request)).unwrap()
    );
}
#[test]
fn controlled_used_requests_match_original_sdk_wire_fields() {
    let fixture = fixture();
    let budget = &fixture.budget;
    for terminal in [false, true] {
        let original =
            wire::InitializeRequest::new(agent_client_protocol::schema::ProtocolVersion::V1)
                .client_capabilities(
                    wire::ClientCapabilities::default()
                        .auth(wire::AuthCapabilities::new().terminal(terminal))
                        .session(
                            wire::ClientSessionCapabilities::default().config_options(
                                wire::SessionConfigOptionsCapabilities::default()
                                    .boolean(wire::BooleanConfigOptionCapabilities::default()),
                            ),
                        ),
                )
                .client_info(wire::Implementation::new("nocterm", "0"));
        assert_eq!(
            payload(outgoing::initialize(budget, &MessageId::Number(1), terminal).unwrap()),
            serde_json::to_value(original).unwrap()
        );
    }
    assert_open_matches_sdk(budget);
    for value in [
        nocterm_ai::session_config::ConfigValue::Flag(true),
        nocterm_ai::session_config::ConfigValue::Value(" exact Ω ".into()),
    ] {
        let request = ConfigurationSelection {
            session: SessionId::new(" s "),
            option: ConfigId::new("opt"),
            value,
        };
        assert_eq!(
            payload(outgoing::configuration(budget, &MessageId::Number(3), &request).unwrap()),
            serde_json::to_value(crate::mapping::outbound::configuration(request)).unwrap()
        );
    }
    assert_eq!(
        payload(outgoing::mode(budget, &MessageId::Number(4), "s", " m ").unwrap()),
        serde_json::to_value(wire::SetSessionModeRequest::new("s", " m ")).unwrap()
    );
    assert_eq!(
        payload(outgoing::authenticate(budget, &MessageId::Number(5), " a ").unwrap()),
        serde_json::to_value(wire::AuthenticateRequest::new(" a ")).unwrap()
    );
    assert_eq!(
        payload(outgoing::session(budget, None, "session/cancel", " s ").unwrap()),
        serde_json::to_value(wire::CancelNotification::new(" s ")).unwrap()
    );
}

#[test]
fn actual_sdk_uuid_and_all_wire_identifier_spellings_are_preserved() {
    use agent_client_protocol::managed::{Driver, Outgoing, Source};
    let fixture = fixture();
    let (_driver, connection) = Driver::new(fixture.budget.clone()).unwrap();
    let uuid = connection.next_id().unwrap();
    let MessageId::String(text) = &uuid else {
        panic!("original UUID must be a string")
    };
    assert_eq!(text.text().len(), 36);
    assert_eq!(text.text().as_bytes()[14], b'4');
    for id in [
        MessageId::Null,
        MessageId::Number(-7),
        uuid,
        MessageId::String(
            Source::copy("escaped \" Ω", fixture.budget.clone())
                .unwrap()
                .full(),
        ),
    ] {
        let request = outgoing::authenticate(&fixture.budget, &id, "auth").unwrap();
        assert!(request.id() == Some(&id));
        let encoded = outgoing::encode(&fixture.budget, &Outgoing::Message(request)).unwrap();
        let expected = match &id {
            MessageId::Null => serde_json::Value::Null,
            MessageId::Number(n) => (*n).into(),
            MessageId::String(s) => s.text().into(),
        };
        let request: serde_json::Value = serde_json::from_str(encoded.text()).unwrap();
        assert_eq!(request["id"], expected);
        let cancel = outgoing::encode(&fixture.budget, &Outgoing::Cancel(id)).unwrap();
        let cancel: serde_json::Value = serde_json::from_str(cancel.text()).unwrap();
        assert_eq!(cancel["params"]["requestId"], expected);
        assert!(cancel.get("id").is_none());
    }
}

#[test]
fn actual_fault_and_batch_directives_match_original_sdk_error_models() {
    use agent_client_protocol::managed::{Buffer, Fault, Outgoing, Reply, Source};
    let fixture = fixture();
    let method = Source::copy("method \" Ω", fixture.budget.clone()).unwrap();
    let malformed = Source::copy("{ broken", fixture.budget.clone()).unwrap();
    let mut replies = Buffer::new(fixture.budget.clone());
    let original = [
        agent_client_protocol::Error::parse_error()
            .data(serde_json::json!({"line":malformed.text()})),
        agent_client_protocol::Error::invalid_request(),
        agent_client_protocol::Error::method_not_found().data(method.text()),
        agent_client_protocol::Error::internal_error().data(format!(
            "request handler dropped its responder for `{}`",
            method.text()
        )),
    ];
    for fault in [
        Fault::Parse,
        Fault::InvalidRequest,
        Fault::MethodNotFound,
        Fault::Abandoned,
    ] {
        replies
            .push(Reply::Fault {
                id: MessageId::Null,
                fault,
                method: Some(method.full()),
                original: Some(malformed.full()),
            })
            .map_err(|(error, _)| error)
            .unwrap();
    }
    let source = outgoing::encode(
        &fixture.budget,
        &Outgoing::Responses {
            batch: true,
            replies,
        },
    )
    .unwrap();
    let values: Vec<serde_json::Value> = serde_json::from_str(source.text()).unwrap();
    for (value, error) in values.into_iter().zip(original) {
        assert_eq!(
            value,
            serde_json::json!({"jsonrpc":"2.0","id":null,"error":error})
        );
    }
}
