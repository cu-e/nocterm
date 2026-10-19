//! Injected connection events: fences, unrouted permissions and usage notices.
use super::*;
use nocterm_ai::{
    AgentEvent,
    model::{
        auth::{PermissionOutcome, PermissionResponder, ProviderPermission},
        ids::{SessionId, ToolId},
        notice::{SessionNotice, SessionUpdate, UsageNotice},
        tools::ToolPatch,
        usage::ContextUsage,
    },
    testing::ScriptedCommands,
    usage::{RateLimit, Window},
};

/// One connected chat with an injected event stream; the fixture owners live as long as it.
struct Connected {
    runtime: Entity<Runtime>,
    client: Rc<ProbeClient>,
    events: async_channel::Sender<AgentEvent>,
    _commands: ScriptedCommands,
    _directory: tempfile::TempDir,
}
fn connected(cx: &mut TestAppContext) -> Connected {
    let connector = Arc::new(ScriptedConnector::default());
    let (runtime, directory, _repository) = fixture_with_connector(cx, connector.clone());
    let commands = ScriptedCommands::default();
    let client = super::closing::connected_client(&runtime, &connector, &commands, cx);
    let (events, receiver) = async_channel::unbounded();
    runtime.update(cx, |runtime, cx| runtime.inject_events(receiver, cx));
    Connected {
        runtime,
        client,
        events,
        _commands: commands,
        _directory: directory,
    }
}

#[gpui::test]
fn a_barrier_is_acknowledged_by_the_foreground_consumer(cx: &mut TestAppContext) {
    let f = connected(cx);
    let (fence, acknowledged) = futures::channel::oneshot::channel();
    f.events.try_send(AgentEvent::Barrier(fence)).unwrap();
    cx.run_until_parked();
    assert_eq!(
        futures::executor::block_on(acknowledged),
        Ok(()),
        "the fence is answered, not dropped"
    );
}

#[gpui::test]
fn a_permission_no_chat_owns_is_answered_cancelled(cx: &mut TestAppContext) {
    let f = connected(cx);
    let (respond, answer) = PermissionResponder::channel();
    let request = ProviderPermission {
        session: SessionId::new("foreign"),
        tool: ToolPatch {
            id: ToolId::new("tool"),
            changes: Default::default(),
            extensions: None,
            claims: Default::default(),
            literal: Default::default(),
        },
        choices: Vec::new(),
        extensions: None,
    };
    f.events
        .try_send(AgentEvent::Permission { request, respond })
        .unwrap();
    cx.run_until_parked();
    assert_eq!(
        futures::executor::block_on(answer),
        Ok(PermissionOutcome::Cancelled)
    );
    assert!(
        !f.client
            .events
            .borrow()
            .iter()
            .any(|event| matches!(event, SessionEvent::Permission { .. }))
    );
}

#[gpui::test]
fn a_usage_notice_merges_limits_and_still_reaches_the_chat(cx: &mut TestAppContext) {
    let f = connected(cx);
    let limit = RateLimit {
        window: Window::Weekly,
        used: Some(0.5),
        resets_at: None,
        reached: false,
    };
    f.events
        .try_send(AgentEvent::Session(SessionNotice {
            session: SessionId::new("session"),
            update: SessionUpdate::Usage(UsageNotice {
                context: ContextUsage {
                    used: 1,
                    size: 2,
                    cost: None,
                },
                rate_limits: vec![limit.clone()],
            }),
        }))
        .unwrap();
    cx.run_until_parked();
    f.runtime.read_with(cx, |runtime, _| {
        assert_eq!(
            runtime
                .limits("codex")
                .unwrap()
                .iter()
                .cloned()
                .collect::<Vec<_>>(),
            vec![limit]
        )
    });
    assert!(f.client.events.borrow().iter().any(|event| matches!(
        event,
        SessionEvent::Update(notice) if matches!(notice.notice().update, SessionUpdate::Usage(_))
    )));
}
