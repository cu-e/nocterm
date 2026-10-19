//! Shared provider and bridge fixtures: real ACP updates and bridge calls without display metadata.
use super::*;
use nocterm_ai::TerminalCall;
use nocterm_session::{ExecFuture, ExecOutput, ExecRequest, ExecSink, HostExec};
use serde_json::Value;

#[derive(Default)]
pub(super) struct Program {
    pub(super) request: Mutex<Option<ExecRequest>>,
    pub(super) sink: Mutex<Option<ExecSink>>,
}

impl HostExec for Program {
    fn exec(&self, request: ExecRequest) -> ExecFuture<ExecOutput> {
        *self.request.lock().unwrap() = Some(request);
        let (sink, output) = ExecOutput::channel();
        *self.sink.lock().unwrap() = Some(sink);
        async move { Ok(output) }.boxed()
    }
}

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
    session: &acp::SessionId,
    update: acp::SessionUpdate,
    cx: &mut TestAppContext,
) {
    f.events
        .try_send(AgentEvent::Session(acp::SessionNotification::new(
            session.clone(),
            update,
        )))
        .unwrap();
    cx.run_until_parked();
}
