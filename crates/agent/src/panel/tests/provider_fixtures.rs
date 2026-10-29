//! Shared provider and bridge fixtures: real ACP updates and bridge calls without display metadata.
use super::*;
use nocterm_ai::TerminalCall;
use nocterm_ai::model::ids::SessionId;
use nocterm_ai::model::session::{SessionNotice, SessionUpdate};
use serde_json::Value;
pub(super) type Program = nocterm_session::testing::FakeExec;

pub(super) fn bridge(f: &Fixture, call: TerminalCall, cx: &mut TestAppContext) -> Value {
    let registration_id = cx.update(|cx| {
        f.panel
            .read(cx)
            .current()
            .unwrap()
            .read(cx)
            .registration()
            .as_ref()
            .unwrap()
            .id
    });
    let (respond, mut receive) = oneshot::channel();
    f.calls
        .try_send(BridgeCall {
            arguments: None,
            display_token: None,
            registration_id,
            call,
            respond,
        })
        .unwrap();
    cx.run_until_parked();
    receive
        .try_recv()
        .unwrap()
        .expect("bridge answered")
        .unwrap()
}

pub(super) fn update(
    f: &Fixture,
    session: &SessionId,
    update: SessionUpdate,
    cx: &mut TestAppContext,
) {
    f.events
        .try_send(AgentEvent::Session(SessionNotice::new(
            session.clone(),
            update,
        )))
        .unwrap();
    cx.run_until_parked();
}
