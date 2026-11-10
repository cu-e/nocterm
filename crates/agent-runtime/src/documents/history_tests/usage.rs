//! Plan limits arrive through the connected agent's event stream.
use super::*;
use nocterm_ai::{
    AgentConnection,
    model::{session::SessionNotice, usage::ContextUsage},
    usage::Window,
};

type UsageFixture = (
    Entity<Runtime>,
    tempfile::TempDir,
    Rc<ProbeClient>,
    async_channel::Sender<AgentEvent>,
);
fn connected(cx: &mut TestAppContext, agent: &str) -> UsageFixture {
    let connector = Arc::new(ScriptedConnector::default());
    let (events, receiver) = async_channel::unbounded();
    connector.push(Ok(AgentConnection {
        commands: closing::BlockedCommands::new(false),
        events: receiver,
        info: AgentInfo {
            name: "Supplied agent".into(),
            version: "1".into(),
            capabilities: Default::default(),
            auth_methods: vec![],
        },
    }));
    let (runtime, directory, _) = fixture_with_connector(cx, connector);
    let client = ProbeClient::new(cx, snapshot());
    let launch = nocterm_agent_providers::catalog()
        .get(agent)
        .map(|profile| profile.default_launch())
        .unwrap_or_else(|| AgentLaunch {
            id: agent.into(),
            name: agent.into(),
            command: "not-executed".into(),
            args: vec![],
            env: Default::default(),
            inherit_env: vec![],
            hub: None,
        });
    runtime.update(cx, |runtime, cx| {
        ownership::register(runtime, client.clone(), cx).unwrap();
        runtime
            .connect(
                client.clone(),
                client.chat.borrow().header.id.clone(),
                launch,
                cx,
            )
            .unwrap();
    });
    cx.run_until_parked();
    assert!(
        client
            .events
            .borrow()
            .iter()
            .any(|event| matches!(event, SessionEvent::Connected { .. }))
    );
    (runtime, directory, client, events)
}
fn usage(meta: serde_json::Value) -> AgentEvent {
    AgentEvent::Session(SessionNotice::new(
        "usage-session",
        SessionUpdate::UsageUpdate(
            ContextUsage::new(50, 100).meta(
                meta.as_object()
                    .cloned()
                    .map(nocterm_ai::model::extensions::Extensions::from),
            ),
        ),
    ))
}
fn report(kind: &str, used: f64) -> AgentEvent {
    usage(serde_json::json!({"_claude/rateLimit": {
        "rateLimitType": kind, "utilization": used, "status": "allowed"
    }}))
}

#[gpui::test]
fn received_limit_windows_merge_and_missing_or_malformed_reports_retain_them(
    cx: &mut TestAppContext,
) {
    let (runtime, _directory, _client, events) = connected(cx, "claude");
    runtime.read_with(cx, |runtime, _| assert!(runtime.limits("claude").is_none()));
    for event in [
        report("seven_day", 0.38),
        report("five_hour", 0.12),
        report("seven_day", 0.42),
        usage(serde_json::json!({})),
        usage(serde_json::json!({"_claude/rateLimit": "invalid"})),
        report("unknown_window", 0.99),
    ] {
        events.try_send(event).unwrap();
    }
    cx.run_until_parked();
    runtime.read_with(cx, |runtime, _| {
        let limits: Vec<_> = runtime.limits("claude").unwrap().iter().collect();
        assert_eq!(limits.len(), 2);
        assert_eq!(limits[0].window, Window::FiveHour);
        assert_eq!(limits[0].used, Some(0.12));
        assert_eq!(limits[1].window, Window::Weekly);
        assert_eq!(limits[1].used, Some(0.42));
        assert!(runtime.limits("codex").is_none());
    });
}

#[gpui::test]
fn agents_without_a_limit_metadata_profile_do_not_fabricate_plan_limits(cx: &mut TestAppContext) {
    for agent in ["codex", "unknown-agent"] {
        let (runtime, _directory, client, events) = connected(cx, agent);
        events.try_send(report("seven_day", 0.99)).unwrap();
        cx.run_until_parked();
        runtime.read_with(cx, |runtime, _| assert!(runtime.limits(agent).is_none()));
        assert!(client.events.borrow().iter().any(|event| matches!(event,
            SessionEvent::Update(notice) if matches!(notice.update, SessionUpdate::UsageUpdate(_))
        )));
    }
}

#[gpui::test]
fn a_stopped_connection_does_not_accept_late_plan_limit_reports(cx: &mut TestAppContext) {
    let (runtime, _directory, _client, events) = connected(cx, "claude");
    let _drain = runtime.update(cx, |runtime, cx| runtime.shutdown(cx));
    let _ = events.try_send(report("seven_day", 0.99));
    cx.run_until_parked();
    runtime.read_with(cx, |runtime, _| assert!(runtime.limits("claude").is_none()));
}
