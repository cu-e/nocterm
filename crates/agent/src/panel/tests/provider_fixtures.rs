//! Provider tool calls, bridged terminal calls and a recording host program.
use super::*;
use nocterm_ai::TerminalCall;
use nocterm_ai::model::{
    literal::{LiteralIdentity, TextRange},
    tools::{Claim, ToolRecord},
};
use nocterm_session::{ExecFuture, ExecOutput, ExecRequest, ExecSink, HostExec};
use serde_json::Value;
/// Actual neutral provider facts before any optional payload or bridge evidence is supplied.
pub(super) fn provider_tool(
    id: impl Into<String>,
    title: impl Into<String>,
    status: nocterm_ai::model::tools::ToolStatus,
) -> nocterm_ai::model::tools::ToolRecord {
    use nocterm_ai::model::{
        ids::ToolId,
        tools::{ToolKind, ToolRecord},
    };
    ToolRecord {
        id: ToolId::new(id),
        title: title.into(),
        name: None,
        kind: ToolKind::Other,
        status,
        output: Vec::new(),
        locations: Vec::new(),
        arguments: None,
        result: None,
        extensions: None,
        claims: Default::default(),
        literal: Default::default(),
        client_display: None,
    }
}
/// These fixtures supply known neutral facts and genuine ranges in their retained title.
/// No wire parser or execution authority is created by this test constructor.
pub(super) fn known_title(
    call: &mut ToolRecord,
    owner: std::ops::Range<usize>,
    tool: std::ops::Range<usize>,
) {
    let registration = call.title[owner.clone()].to_owned();
    let name = call.title[tool.clone()].to_owned();
    call.literal.title = LiteralIdentity::Registered {
        owner: TextRange::supplied(&call.title, &call.title[owner]).unwrap(),
        tool: TextRange::supplied(&call.title, &call.title[tool]).unwrap(),
    };
    call.claims.title = Claim::Known {
        registration,
        tool: name,
    };
}
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
