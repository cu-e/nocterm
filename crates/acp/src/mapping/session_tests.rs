//! Session updates, requests and answers cross the boundary without changing meaning.
use super::{
    info::{auth_methods, capabilities, remote_error},
    requests::{
        new_session, opened, permission, prompt, prompt_outcome, servers, set_config, set_mode,
        wire_outcome, wire_session,
    },
    session::notice,
};
use agent_client_protocol_schema::v1 as wire;
use nocterm_ai::{
    BridgeLaunch,
    model::{
        auth::{AuthFlow, PermissionEffect, PermissionOutcome},
        content::Content,
        controls::{Change, ConfigSetting},
        ids::{ConfigChoiceId, ConfigId, ModeId, PermissionOptionId, SessionId},
        session::*,
    },
};
use serde_json::json;

fn update(value: serde_json::Value) -> wire::SessionNotification {
    let mut value = value;
    value["sessionId"] = json!("s1");
    serde_json::from_value(json!({"sessionId":"s1","update":value["update"]})).unwrap()
}

#[test]
fn every_update_kind_the_client_understands_is_mapped() {
    let notice_of = |update_json: serde_json::Value| {
        notice(update(json!({"update": update_json})))
            .unwrap()
            .update
    };
    assert_eq!(
        notice_of(
            json!({"sessionUpdate":"agent_message_chunk","content":{"type":"text","text":"hi"}})
        ),
        SessionUpdate::AgentMessageChunk(Content::text("hi"))
    );
    assert_eq!(
        notice_of(json!({"sessionUpdate":"current_mode_update","currentModeId":"plan"})),
        SessionUpdate::CurrentModeUpdate(ModeId::new("plan"))
    );
    let SessionUpdate::SessionInfoUpdate(info) =
        notice_of(json!({"sessionUpdate":"session_info_update","title":null}))
    else {
        panic!("info expected")
    };
    assert_eq!(info.title, Change::Clear);
    let SessionUpdate::SessionInfoUpdate(info) =
        notice_of(json!({"sessionUpdate":"session_info_update","title":"T"}))
    else {
        panic!("info expected")
    };
    assert_eq!(info.title, Change::Set("T".into()));
    let SessionUpdate::SessionInfoUpdate(info) =
        notice_of(json!({"sessionUpdate":"session_info_update"}))
    else {
        panic!("info expected")
    };
    assert_eq!(info.title, Change::Keep);
    let SessionUpdate::UsageUpdate(usage) = notice_of(
        json!({"sessionUpdate":"usage_update","used":3,"size":9,"cost":{"amount":1.5,"currency":"USD"},"_meta":{"k":1}}),
    ) else {
        panic!("usage expected")
    };
    assert_eq!((usage.used, usage.size), (3, 9));
    assert_eq!(usage.cost.unwrap().currency, "USD");
    assert_eq!(usage.meta.unwrap()["k"], 1);
    let SessionUpdate::AvailableCommandsUpdate(commands) = notice_of(json!({
        "sessionUpdate":"available_commands_update",
        "availableCommands":[{"name":"review","description":"Review","input":{"hint":"what"}},{"name":"plain","description":"Plain"}]
    })) else {
        panic!("commands expected")
    };
    assert_eq!(commands[0].hint.as_deref(), Some("what"));
    assert_eq!(commands[1].hint, None);
    let SessionUpdate::Plan(plan) = notice_of(json!({
        "sessionUpdate":"plan",
        "entries":[{"content":"step","priority":"high","status":"in_progress"}]
    })) else {
        panic!("plan expected")
    };
    assert_eq!(format!("{:?}", plan.entries[0].status), "InProgress");
    assert!(matches!(
        notice_of(json!({"sessionUpdate":"tool_call","toolCallId":"t","title":"x"})),
        SessionUpdate::ToolCall(_)
    ));
    assert!(matches!(
        notice_of(
            json!({"sessionUpdate":"tool_call_update","toolCallId":"t","status":"completed"})
        ),
        SessionUpdate::ToolCallUpdate(_)
    ));
}

#[test]
fn requests_carry_neutral_facts_to_the_protocol_unchanged() {
    let launch = BridgeLaunch {
        program: "/bin/relay".into(),
        args: vec!["--x".into()],
        env: vec![("A".into(), "1".into())],
    };
    let endpoints = vec![ExternalToolEndpoint {
        name: "nocterm-7".into(),
        launch,
    }];
    let wire_servers = servers(endpoints.clone());
    let wire::McpServer::Stdio(stdio) = &wire_servers[0] else {
        panic!("stdio expected")
    };
    assert_eq!(stdio.name, "nocterm-7");
    assert_eq!(stdio.args, ["--x"]);
    assert_eq!(stdio.env[0].value, "1");
    let request = new_session(SessionOpen {
        working_directory: "/work".into(),
        endpoints,
    });
    assert_eq!(request.cwd, std::path::PathBuf::from("/work"));
    assert_eq!(request.mcp_servers.len(), 1);
    let request = prompt(PromptRequest {
        session: SessionId::new("s"),
        content: vec![Content::text("go")],
    });
    assert_eq!(request.session_id, wire_session(&SessionId::new("s")));
    assert_eq!(request.prompt.len(), 1);
    let request = set_mode(SetMode {
        session: SessionId::new("s"),
        mode: ModeId::new("plan"),
    });
    assert_eq!(request.mode_id.to_string(), "plan");
    for (setting, boolean) in [
        (ConfigSetting::Boolean(true), true),
        (ConfigSetting::Choice(ConfigChoiceId::new("opus")), false),
    ] {
        let request = set_config(SetConfigOption {
            session: SessionId::new("s"),
            config: ConfigId::new("model"),
            setting,
        });
        assert_eq!(request.config_id.to_string(), "model");
        assert_eq!(request.value.as_bool().is_some(), boolean);
    }
}

#[test]
fn answers_and_errors_keep_their_meaning() {
    let opened = opened(
        wire::NewSessionResponse::new("s")
            .modes(wire::SessionModeState::new(
                "plan",
                vec![wire::SessionMode::new("plan", "Plan")],
            ))
            .config_options(vec![wire::SessionConfigOption::boolean("f", "Flag", true)]),
    );
    assert_eq!(opened.session, SessionId::new("s"));
    assert_eq!(opened.modes.unwrap().choices[0].name, "Plan");
    assert_eq!(opened.configuration.unwrap().len(), 1);
    let outcome = prompt_outcome(
        wire::PromptResponse::new(wire::StopReason::MaxTokens).usage(wire::Usage::new(3, 1, 2)),
    )
    .unwrap();
    assert_eq!(format!("{:?}", outcome.stop_reason), "MaxTokens");
    assert_eq!(outcome.usage.unwrap().total, 3);
    let error = remote_error(wire::Error::auth_required());
    assert_eq!(error.code, RemoteError::AUTH_REQUIRED);
    let error = remote_error(wire::Error::method_not_found());
    assert_eq!(error.code, RemoteError::METHOD_NOT_FOUND);
    assert_eq!(
        RemoteError {
            code: 7,
            message: String::new()
        }
        .to_string(),
        "7"
    );
}

#[test]
fn permission_requests_and_answers_round_trip() {
    let request = wire::RequestPermissionRequest::new(
        "s",
        wire::ToolCallUpdate::new("t", wire::ToolCallUpdateFields::new().title("Run")),
        vec![
            wire::PermissionOption::new("a", "Allow", wire::PermissionOptionKind::AllowOnce),
            wire::PermissionOption::new("b", "Always", wire::PermissionOptionKind::AllowAlways),
            wire::PermissionOption::new("c", "No", wire::PermissionOptionKind::RejectOnce),
            wire::PermissionOption::new("d", "Never", wire::PermissionOptionKind::RejectAlways),
        ],
    );
    let mapped = permission(request).unwrap();
    assert_eq!(mapped.tool.changes.title.as_deref(), Some("Run"));
    assert_eq!(
        mapped
            .choices
            .iter()
            .map(|choice| choice.effect)
            .collect::<Vec<_>>(),
        [
            PermissionEffect::AllowOnce,
            PermissionEffect::AllowRemembered,
            PermissionEffect::DenyOnce,
            PermissionEffect::DenyRemembered
        ]
    );
    assert_eq!(
        wire_outcome(PermissionOutcome::Cancelled),
        wire::RequestPermissionOutcome::Cancelled
    );
    assert_eq!(
        wire_outcome(PermissionOutcome::Selected(PermissionOptionId::new("a"))),
        wire::RequestPermissionOutcome::Selected(wire::SelectedPermissionOutcome::new("a"))
    );
}

#[test]
fn agent_introductions_keep_images_and_sign_in_methods() {
    let mut offered = wire::AgentCapabilities::default();
    assert!(!capabilities(&offered).image_prompts);
    offered.prompt_capabilities.image = true;
    assert!(capabilities(&offered).image_prompts);
    let methods = auth_methods(vec![
        wire::AuthMethod::Agent(wire::AuthMethodAgent::new("agent", "Agent")),
        wire::AuthMethod::Terminal(
            wire::AuthMethodTerminal::new("setup", "Setup").args(vec!["login".into()]),
        ),
    ]);
    assert_eq!(methods[0].flow, AuthFlow::AgentManaged);
    let AuthFlow::TerminalSetup(setup) = &methods[1].flow else {
        panic!("terminal expected")
    };
    assert_eq!(setup.args, ["login"]);
    assert_eq!(methods[1].label, "Setup");
}
