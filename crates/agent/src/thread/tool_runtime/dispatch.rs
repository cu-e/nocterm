//! One accepted terminal dispatch uses its captured capability and ExpectedSession.
use gpui_kit::App;
use nocterm_ai::{BridgeCall, TerminalCall, ToolFailure};
use nocterm_workspace::{
    ExpectedSession, TerminalCapability, TerminalEntry, TerminalError, TextRequest,
};
pub(in crate::thread) struct TerminalDispatch {
    call: BridgeCall,
    entry: TerminalEntry,
    expected: ExpectedSession,
}
pub(in crate::thread) enum DispatchAction {
    Respond(BridgeCall, Result<serde_json::Value, ToolFailure>, usize),
    Live(
        TerminalEntry,
        ExpectedSession,
        nocterm_ai::RunCommand,
        BridgeCall,
    ),
    Execute(TerminalEntry, ExpectedSession, BridgeCall),
    Read(BridgeCall),
    Cancel(BridgeCall),
    Ignore,
    ExecuteAdmitted(BridgeCall, TerminalEntry, ExpectedSession),
    List(BridgeCall),
    Open(String, BridgeCall),
    Reconnect(
        super::super::bound_call::BoundToolCall,
        TerminalEntry,
        ExpectedSession,
    ),
}
impl TerminalDispatch {
    pub(in crate::thread) fn new(
        call: BridgeCall,
        entry: TerminalEntry,
        expected: ExpectedSession,
    ) -> Self {
        Self {
            call,
            entry,
            expected,
        }
    }
    pub(in crate::thread) fn prepare(self, redact: bool, cx: &mut App) -> DispatchAction {
        match &self.call.call {
            TerminalCall::ReadTerminal(request) => {
                let result = self
                    .entry
                    .capabilities
                    .text()
                    .ok_or(TerminalError::Unsupported(TerminalCapability::TextRead))
                    .and_then(|port| {
                        port.read(
                            self.expected,
                            TextRequest {
                                max_lines: request.lines.unwrap_or(200),
                                max_bytes: 64 * 1024,
                                since_line: request.since,
                            },
                            cx,
                        )
                    })
                    .map(|tail| super::presentation::text_payload(tail, redact));
                let (result, bytes) = match result {
                    Ok((payload, bytes)) => (Ok(payload), bytes),
                    Err(error) => (Err(error.to_string().into()), 0),
                };
                DispatchAction::Respond(self.call, result, bytes)
            }
            TerminalCall::SendInput(request) => {
                let text = format!(
                    "{}{}",
                    request.text,
                    if request.press_enter { "\r" } else { "" }
                );
                let result = self
                    .entry
                    .capabilities
                    .input()
                    .ok_or(TerminalError::Unsupported(TerminalCapability::Input))
                    .and_then(|port| port.send_text(self.expected, &text, cx))
                    .map(|()| serde_json::json!({"accepted":true}))
                    .map_err(|error| error.to_string().into());
                DispatchAction::Respond(self.call, result, 0)
            }
            TerminalCall::RunCommand(request) => {
                DispatchAction::Live(self.entry, self.expected, request.clone(), self.call)
            }
            TerminalCall::ExecCommand(_) => {
                DispatchAction::Execute(self.entry, self.expected, self.call)
            }
            TerminalCall::ReadCommand(_) => DispatchAction::Read(self.call),
            TerminalCall::CancelCommand(_) => DispatchAction::Cancel(self.call),
            TerminalCall::ListTerminals | TerminalCall::OpenTerminal(_) => DispatchAction::Ignore,
        }
    }
}

/// A supplied admission observation cannot renew a captured terminal authority.
pub(in crate::thread) struct ExecutionAdmission {
    available: bool,
    registration: Option<u64>,
}
pub(in crate::thread) enum AdmissionRoute {
    Action(Box<DispatchAction>),
    Resolve(Box<super::super::bound_call::BoundToolCall>),
}
impl ExecutionAdmission {
    pub(in crate::thread) fn new(available: bool, registration: Option<u64>) -> Self {
        Self {
            available,
            registration,
        }
    }
    pub(in crate::thread) fn route(
        &self,
        bound: super::super::bound_call::BoundToolCall,
    ) -> AdmissionRoute {
        let action = if !self.available || self.registration != Some(bound.call().registration_id) {
            DispatchAction::Respond(bound.into_call(), Err("Chat is unavailable.".into()), 0)
        } else {
            match &bound.call().call {
                TerminalCall::ListTerminals => DispatchAction::List(bound.into_call()),
                TerminalCall::OpenTerminal(request) => {
                    DispatchAction::Open(request.server_id.clone(), bound.into_call())
                }
                _ => return AdmissionRoute::Resolve(Box::new(bound)),
            }
        };
        AdmissionRoute::Action(Box::new(action))
    }
    pub(in crate::thread) fn resolve(
        &self,
        bound: super::super::bound_call::BoundToolCall,
        entry: Option<TerminalEntry>,
        reconnect: bool,
        reconnecting: bool,
        cx: &App,
    ) -> DispatchAction {
        let Some(entry) = entry else {
            return DispatchAction::Respond(
                bound.into_call(),
                Err(super::super::tools::unreachable(
                    "Terminal is not attached or was closed. Call list_terminals for the current ones.",
                )),
                0,
            );
        };
        let Some(expected) = bound.expected() else {
            return DispatchAction::Respond(
                bound.into_call(),
                Err(TerminalError::StaleSession.to_string().into()),
                0,
            );
        };
        let info = entry.capabilities.state().info(cx);
        if info.as_ref().is_none_or(|info| info.expected != expected) {
            return DispatchAction::Respond(
                bound.into_call(),
                Err(TerminalError::StaleSession.to_string().into()),
                0,
            );
        }
        let uses_session = matches!(
            &bound.call().call,
            TerminalCall::ReadTerminal(_)
                | TerminalCall::SendInput(_)
                | TerminalCall::RunCommand(_)
                | TerminalCall::ExecCommand(_)
        );
        if reconnect
            && uses_session
            && (reconnecting
                || info.is_some_and(|info| {
                    !info.local && info.status == nocterm_workspace::TerminalStatus::Closed
                }))
        {
            return DispatchAction::Reconnect(bound, entry, expected);
        }
        // Native read/input execution is a separate admitted step interpreted by the coordinator.
        DispatchAction::ExecuteAdmitted(bound.into_call(), entry, expected)
    }
}
