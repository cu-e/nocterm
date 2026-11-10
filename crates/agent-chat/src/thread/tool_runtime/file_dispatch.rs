//! The file tools of a chat: which attached terminals take them, and one call's work on the
//! filesystem captured under the call's `ExpectedSession`.
//!
//! A terminal takes the file tools when it serves a filesystem of its own but runs no programs:
//! the agent cannot reach its files with `exec_command`, so the chat offers these tools while
//! such a terminal is attached, and they act on nothing else.
use std::sync::Arc;

use futures::future::BoxFuture;
use gpui::{App, Context};
use nocterm_ai::{BridgeCall, ToolFailure};
use nocterm_session::RemoteFs;
use nocterm_workspace_ports::{
    ExpectedSession, TerminalCapability, TerminalEntry, TerminalError, TerminalFilesystem,
};

use super::dispatch::DispatchAction;
use crate::thread::{AgentThread, session_driver::OperationGuard};

/// One file call's work, answered when it ends.
pub(in crate::thread) type FileWork = BoxFuture<'static, Result<serde_json::Value, ToolFailure>>;

/// Whether the agent reaches this terminal's files only through the file tools.
pub(in crate::thread) fn takes_file_tools(
    entry: &TerminalEntry,
    expected: ExpectedSession,
    cx: &App,
) -> bool {
    remote(entry, expected, cx).is_ok()
        && entry.capabilities.execution().is_none_or(|execution| {
            matches!(
                execution.executor(expected, cx),
                Err(TerminalError::Unsupported(_))
            )
        })
}

fn remote(
    entry: &TerminalEntry,
    expected: ExpectedSession,
    cx: &App,
) -> Result<Arc<dyn RemoteFs>, TerminalError> {
    let files = entry
        .capabilities
        .files()
        .ok_or(TerminalError::Unsupported(TerminalCapability::Files))?;
    match files.filesystem(expected, cx)? {
        TerminalFilesystem::Remote { fs, .. } => Ok(fs),
        TerminalFilesystem::Local => Err(TerminalError::Unsupported(TerminalCapability::Files)),
    }
}

/// The work of the file call `call` on its terminal, or why the terminal does not take it.
pub(in crate::thread) fn prepare(
    call: BridgeCall,
    entry: &TerminalEntry,
    expected: ExpectedSession,
    redact: bool,
    cx: &App,
) -> DispatchAction {
    if !takes_file_tools(entry, expected, cx) {
        return DispatchAction::Respond(
            call,
            Err(
                "The file tools act only on terminals with file_tools: true. Use exec_command or transfer_files on this terminal."
                    .into(),
            ),
            0,
        );
    }
    match remote(entry, expected, cx) {
        Ok(fs) => {
            let work = Box::pin(nocterm_ai::files::run(call.call.clone(), fs, redact));
            DispatchAction::Files(call, work)
        }
        Err(error) => DispatchAction::Respond(call, Err(error.to_string().into()), 0),
    }
}

/// Answers the file call `call` once its work ends, holding the chat's operation until then.
pub(in crate::thread) fn complete(
    call: BridgeCall,
    work: FileWork,
    guard: OperationGuard,
    cx: &mut Context<AgentThread>,
) {
    cx.spawn(async move |this, cx| {
        let _guard = guard;
        let result = work.await;
        let _ = this.update(cx, |this, cx| this.on_tool_completion(call, result, cx));
    })
    .detach();
}
