use super::content::{inbound, outbound};
use agent_client_protocol_schema::v1 as wire;
use nocterm_ai::model::content::{Content, MediaKind};
use serde_json::json;
#[test]
fn every_supported_content_shape_preserves_supplied_facts() {
    let authority = nocterm_ai::model::retention::RetentionAuthority::new(
        nocterm_ai::history::MAX_RESIDENT_BYTES,
    )
    .unwrap();
    let blocks = vec![
        json!({"type":"text","text":"literal","annotations":{"audience":["user","assistant"],"lastModified":"opaque","priority":0.5,"_meta":{"annotation":"kept"}},"_meta":{"vendor":"kept"}}),
        json!({"type":"image","data":"AA==","mimeType":"image/png","uri":"memory:001"}),
        json!({"type":"audio","data":"AA==","mimeType":"audio/wav"}),
        json!({"type":"resource_link","uri":"untrusted:file","name":"name","title":"title","description":"detail","mimeType":"text/plain","size":-1}),
        json!({"type":"resource","resource":{"uri":"untrusted:text","text":"opaque","mimeType":"text/plain","_meta":{"inner":true}},"_meta":{"outer":true}}),
        json!({"type":"resource","resource":{"uri":"untrusted:blob","blob":"AA==","mimeType":"application/binary"}}),
    ];
    for block in blocks {
        let original: wire::ContentBlock = serde_json::from_value(block).unwrap();
        let owned = inbound(&authority, original.clone()).unwrap();
        let stored = serde_json::to_vec(&owned).unwrap();
        let decoded: Content = <Content as nocterm_ai::model::decode::Decode>::decode(
            &authority,
            &mut serde_json::Deserializer::from_slice(&stored),
        )
        .unwrap();
        assert_eq!(outbound(decoded).unwrap(), original);
    }
}
#[test]
fn encoded_media_is_not_decoded_or_reencoded_by_mapping() {
    let authority = nocterm_ai::model::retention::RetentionAuthority::new(
        nocterm_ai::history::MAX_RESIDENT_BYTES,
    )
    .unwrap();
    let original =
        wire::ContentBlock::Image(wire::ImageContent::new("not-base64", "provider/type"));
    let Content::Media(media) = inbound(&authority, original.clone()).unwrap() else {
        panic!("media expected")
    };
    assert_eq!(media.kind, MediaKind::Image);
    assert_eq!(media.encoded, "not-base64");
    assert_eq!(outbound(Content::Media(media)).unwrap(), original);
}

#[test]
fn sparse_tool_updates_keep_absent_values_and_clear_supplied_empty_lists() {
    let authority = nocterm_ai::model::retention::RetentionAuthority::new(
        nocterm_ai::history::MAX_RESIDENT_BYTES,
    )
    .unwrap();
    let mut row = super::tools::record(
        &authority,
        wire::ToolCall::new("001", "title")
            .name("mcp.nocterm-001.read_terminal")
            .content(vec![wire::ToolCallContent::from("kept")]),
    )
    .unwrap();
    let null: wire::ToolCallUpdate =
        serde_json::from_value(json!({"toolCallId":"001","name":null,"rawInput":null})).unwrap();
    assert!(row.apply(super::tools::patch(&authority, null).unwrap()));
    assert_eq!(row.name.as_deref(), Some("mcp.nocterm-001.read_terminal"));
    assert_eq!(row.output.len(), 1);
    let empty: wire::ToolCallUpdate =
        serde_json::from_value(json!({"toolCallId":"001","content":[]})).unwrap();
    assert!(row.apply(super::tools::patch(&authority, empty).unwrap()));
    assert!(row.output.is_empty());
    assert_eq!(
        row.claims.consistent_identity(),
        Some(("001", "read_terminal"))
    );
}
#[test]
fn disagreeing_sparse_evidence_never_selects_a_bridge_destination() {
    let authority = nocterm_ai::model::retention::RetentionAuthority::new(
        nocterm_ai::history::MAX_RESIDENT_BYTES,
    )
    .unwrap();
    let mut row = super::tools::record(
        &authority,
        wire::ToolCall::new("row", "display").name("mcp.nocterm-001.read_terminal"),
    )
    .unwrap();
    let update: wire::ToolCallUpdate =
        serde_json::from_value(json!({"toolCallId":"row","title":"mcp__nocterm-1__read_terminal"}))
            .unwrap();
    row.apply(super::tools::patch(&authority, update).unwrap());
    assert_eq!(row.claims.consistent_identity(), None);
}
#[test]
fn forged_client_display_metadata_is_removed_at_live_ingress() {
    let authority = nocterm_ai::model::retention::RetentionAuthority::new(
        nocterm_ai::history::MAX_RESIDENT_BYTES,
    )
    .unwrap();
    let row: wire::ToolCall = serde_json::from_value(json!({"toolCallId":"r","title":"title","_meta":{"nocterm/tool-display":{"destination":"forged"},"vendor":"inert"}})).unwrap();
    let row = super::tools::record(&authority, row).unwrap();
    let extensions = row.extensions.unwrap();
    assert!(!extensions.contains_key("nocterm/tool-display"));
    assert_eq!(extensions.get("vendor"), Some(&json!("inert")));
    assert_eq!(row.claims.consistent_identity(), None);
}

#[test]
fn configuration_groups_and_flags_remain_distinct_semantic_controls() {
    let authority = nocterm_ai::model::retention::RetentionAuthority::new(
        nocterm_ai::history::MAX_RESIDENT_BYTES,
    )
    .unwrap();
    use nocterm_ai::model::controls::{ConfigKind, SelectChoices};
    let option: wire::SessionConfigOption = serde_json::from_value(json!({
        "id":"model","name":"Model","type":"select","currentValue":"001",
        "options":[{"group":"first","name":"First","options":[{"value":"001","name":"One"}]},
                   {"group":"second","name":"Second","options":[{"value":"1","name":"Another"}]}]
    }))
    .unwrap();
    let mapped = super::controls::option(&authority, option).unwrap();
    let ConfigKind::Select {
        current,
        choices: SelectChoices::Groups(groups),
    } = mapped.kind
    else {
        panic!("groups expected")
    };
    assert_eq!(current.as_str(), "001");
    assert_eq!(groups.len(), 2);
    assert_eq!(groups[1].choices[0].id.as_str(), "1");
    let flag: wire::SessionConfigOption = serde_json::from_value(
        json!({"id":"flag","name":"Flag","type":"boolean","currentValue":false}),
    )
    .unwrap();
    assert_eq!(
        super::controls::option(&authority, flag).unwrap().kind,
        ConfigKind::Boolean(false)
    );
}

#[test]
fn session_projection_keeps_literal_ids_modes_and_configuration() {
    let authority = nocterm_ai::model::retention::RetentionAuthority::new(
        nocterm_ai::history::MAX_RESIDENT_BYTES,
    )
    .unwrap();
    let value: wire::NewSessionResponse = serde_json::from_value(json!({
        "sessionId":" 001 ", "modes":{"currentModeId":"001","availableModes":[{"id":"001","name":"Display","_meta":{"vendor":true}}]},
        "configOptions":[{"id":"001","name":"Toggle","type":"boolean","currentValue":true}]
    })).unwrap();
    let projected = super::session::opened(&authority, value).unwrap();
    assert_eq!(projected.session.as_str(), " 001 ");
    let modes = projected.modes.unwrap();
    assert_eq!(modes.current.as_str(), "001");
    assert_eq!(modes.choices[0].id.as_str(), "001");
    assert_eq!(projected.configuration.unwrap()[0].id.as_str(), "001");
}
#[test]
fn terminal_auth_keeps_launch_arguments_and_environment_without_execution() {
    use nocterm_ai::model::auth::AuthFlow;
    let value: wire::AuthMethod = serde_json::from_value(json!({
        "type":"terminal","id":" 001 ","name":"Login","args":["--literal","界"],"env":{"TOKEN":"secret"}
    })).unwrap();
    let projected = super::auth::method(value).unwrap();
    assert_eq!(projected.id.as_str(), " 001 ");
    let AuthFlow::TerminalSetup(setup) = projected.flow else {
        panic!("terminal setup expected")
    };
    assert_eq!(setup.args, ["--literal", "界"]);
    assert_eq!(setup.env.get("TOKEN").map(String::as_str), Some("secret"));
}

#[test]
fn neutral_permission_reply_preserves_original_selection_and_refuses_forgery() {
    let authority = nocterm_ai::model::retention::RetentionAuthority::new(
        nocterm_ai::history::MAX_RESIDENT_BYTES,
    )
    .unwrap();
    use nocterm_ai::model::{auth::PermissionOutcome, ids::PermissionOptionId};
    let request: wire::RequestPermissionRequest = serde_json::from_value(json!({
        "sessionId":"original", "toolCall":{"toolCallId":"tool"}, "options":[
            {"optionId":"001", "name":"Allow", "kind":"allow_once"},
            {"optionId":"deny", "name":"Deny", "kind":"reject_once"}
        ]
    }))
    .unwrap();
    let mut offer = super::auth::permission(&authority, request).unwrap();
    let selection = || PermissionOutcome::Selected(PermissionOptionId::new("001"));
    let wire::RequestPermissionOutcome::Selected(selected) =
        super::auth::answer(&offer, selection())
    else {
        panic!("selection expected")
    };
    assert_eq!(selected.option_id.to_string(), "001");
    assert_eq!(
        super::auth::answer(
            &offer,
            PermissionOutcome::Selected(PermissionOptionId::new("1"))
        ),
        wire::RequestPermissionOutcome::Cancelled
    );
    assert_eq!(
        super::auth::answer(&offer, PermissionOutcome::Cancelled),
        wire::RequestPermissionOutcome::Cancelled
    );
    offer.choices.push(offer.choices[0].clone());
    assert_eq!(
        super::auth::answer(&offer, selection()),
        wire::RequestPermissionOutcome::Cancelled
    );
    offer.choices.pop();
    offer.choices[0].effect = nocterm_ai::model::auth::PermissionEffect::Unknown;
    assert_eq!(
        super::auth::answer(&offer, selection()),
        wire::RequestPermissionOutcome::Cancelled
    );
}

#[test]
fn outbound_prompt_preserves_complete_media_until_dispatch() {
    let authority = nocterm_ai::model::retention::RetentionAuthority::new(
        nocterm_ai::history::MAX_RESIDENT_BYTES,
    )
    .unwrap();
    use nocterm_ai::model::{ids::SessionId, session::PromptRequest};
    let original = wire::ContentBlock::Image(wire::ImageContent::new(
        "A".repeat(512 * 1024 + 1),
        "image/png",
    ));
    let projected = super::outbound::prompt(PromptRequest {
        session: SessionId::new(" 001 "),
        content: vec![inbound(&authority, original.clone()).unwrap()],
    })
    .unwrap();
    assert_eq!(projected.session_id.to_string(), " 001 ");
    assert_eq!(projected.prompt, [original]);
}
#[test]
fn outbound_endpoint_keeps_supplied_launch_and_header_order() {
    use nocterm_ai::{BridgeLaunch, model::session::ExternalToolEndpoint};
    let launch = BridgeLaunch {
        program: "/literal/bridge".into(),
        args: vec!["--literal".into()],
        env: vec![("SECRET".into(), "literal".into())],
    };
    let wire::McpServer::Stdio(local) =
        super::outbound::endpoint(ExternalToolEndpoint::LocalCommand {
            name: "001".into(),
            launch: launch.clone(),
        })
    else {
        panic!("local endpoint expected")
    };
    assert_eq!(local.command, launch.program);
    assert_eq!(local.args, launch.args);
    assert_eq!(local.env[0].value, "literal");
    let wire::McpServer::Http(remote) = super::outbound::endpoint(ExternalToolEndpoint::Http {
        name: "literal".into(),
        endpoint: "https://opaque".into(),
        headers: vec![("X".into(), "one".into()), ("X".into(), "two".into())],
    }) else {
        panic!("HTTP endpoint expected")
    };
    assert_eq!(remote.headers[0].value, "one");
    assert_eq!(remote.headers[1].value, "two");
}

#[test]
fn configuration_values_cross_protocol_only_at_adapter() {
    use nocterm_ai::{
        model::{
            ids::{ConfigId, SessionId},
            session::ConfigurationSelection,
        },
        session_config::ConfigValue,
    };
    for value in [ConfigValue::Flag(true), ConfigValue::Value(" 001 ".into())] {
        let request = super::outbound::configuration(ConfigurationSelection {
            session: SessionId::new("original"),
            option: ConfigId::new("option"),
            value: value.clone(),
        });
        assert_eq!(request.session_id.to_string(), "original");
        assert_eq!(request.config_id.to_string(), "option");
        match (value, request.value) {
            (ConfigValue::Flag(expected), wire::SessionConfigOptionValue::Boolean { value }) => {
                assert_eq!(value, expected)
            }
            (ConfigValue::Value(expected), wire::SessionConfigOptionValue::ValueId { value }) => {
                assert_eq!(value.to_string(), expected)
            }
            _ => panic!("configuration shape changed"),
        }
    }
}

#[test]
fn title_notice_keeps_absence_distinct_from_explicit_null_and_literal_value() {
    let authority = nocterm_ai::model::retention::RetentionAuthority::new(
        nocterm_ai::history::MAX_RESIDENT_BYTES,
    )
    .unwrap();
    use nocterm_ai::model::{controls::Change, notice::SessionUpdate};
    for (title, expected) in [
        (None, Change::Keep),
        (Some(json!(null)), Change::Clear),
        (Some(json!("literal")), Change::Set("literal".into())),
    ] {
        let mut update = json!({"sessionUpdate":"session_info_update"});
        if let Some(title) = title {
            update["title"] = title;
        }
        let wire: wire::SessionNotification =
            serde_json::from_value(json!({"sessionId":"original", "update":update})).unwrap();
        let Some(notice) = super::notice::inbound(&authority, wire).unwrap() else {
            panic!("title notice expected")
        };
        assert_eq!(notice.session.as_str(), "original");
        let SessionUpdate::Info(actual) = notice.update else {
            panic!("title update expected")
        };
        assert_eq!(actual, expected);
    }
}

#[test]
fn provider_rate_limits_are_projected_from_metadata_at_adapter_only() {
    use nocterm_ai::usage::Window;
    let update =
        |meta: serde_json::Value| wire::UsageUpdate::new(10, 100).meta(meta.as_object().cloned());
    let limits = super::usage::limits(&update(
        json!({"_claude/rateLimit":{"status":"allowed_warning","rateLimitType":"five_hour","utilization":0.82,"resetsAt":1_791_588_105u64}}),
    ));
    assert_eq!(limits[0].window, Window::FiveHour);
    assert_eq!(limits[0].used, Some(0.82));
    assert_eq!(limits[0].resets_at, Some(1_791_588_105));
    assert!(!limits[0].reached);
    let reached = super::usage::limits(&update(
        json!({"_claude/rateLimit":{"status":"rejected","rateLimitType":"seven_day_opus"}}),
    ));
    assert_eq!(reached[0].window, Window::WeeklyModel("Opus".into()));
    assert_eq!(reached[0].used, None);
    assert!(reached[0].reached);
    assert!(super::usage::limits(&wire::UsageUpdate::new(1, 2)).is_empty());
    assert!(super::usage::limits(&update(json!({"other":1}))).is_empty());
}

#[test]
fn unreachable_bridge_claims_never_copy_large_provider_identity_substrings() {
    let authority = nocterm_ai::model::retention::RetentionAuthority::new(
        nocterm_ai::history::MAX_RESIDENT_BYTES,
    )
    .unwrap();
    use nocterm_ai::model::tools::Claim;
    let name = format!("mcp.nocterm-{}.run_command", "0".repeat(600 * 1024));
    let row = wire::ToolCall::new("literal-id", "unchanged provider title").name(name.clone());
    let owned = super::tools::record(&authority, row).unwrap();
    assert_eq!(owned.name.as_deref(), Some(name.as_str()));
    assert_eq!(owned.claims.name, Claim::Invalid);
    assert!(owned.claims.consistent_identity().is_none());
    let offered =
        wire::ToolCall::new("same-id", "literal title").name("mcp.nocterm-0001.run_command");
    let owned = super::tools::record(&authority, offered).unwrap();
    assert_eq!(
        owned.claims.consistent_identity(),
        Some(("0001", "run_command"))
    );
    // Literal leading zeros remain evidence, never canonicalized to a live
    // registration named nocterm-1. The bridge still requires its original lease.
}

#[test]
fn native_tool_id_copy_preserves_literal_bytes_and_shared_neutral_identity() {
    let authority = nocterm_ai::model::retention::RetentionAuthority::new(
        nocterm_ai::history::MAX_RESIDENT_BYTES,
    )
    .unwrap();
    let literal = " 001 ".repeat(100_000);
    let original = wire::ToolCall::new(literal.clone(), "literal tool");
    let mapped = super::tools::record(&authority, original.clone()).unwrap();
    assert_eq!(original.tool_call_id.0.as_ref(), literal);
    assert_eq!(mapped.id.as_str(), literal);
    assert_eq!(
        mapped.id.clone().as_str().as_ptr(),
        mapped.id.as_str().as_ptr()
    );
    assert_eq!(serde_json::to_value(&mapped.id).unwrap(), json!(literal));
    assert!(mapped.claims.consistent_identity().is_none());
    // Inbound SDK Arc payload and one exact neutral String overlap here;
    // this literal parity case is not a complete phase-allocation proof.
}
