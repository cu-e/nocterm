//! A transfer call is admitted against two terminals' exact sessions; their filesystems
//! are captured once, under those sessions, into the request the chat submits.
use super::transfers::{ChatTransfers, TransferBinding};
use crate::thread::bound_call::BoundToolCall;
use crate::thread::tool_runtime::dispatch::DispatchAction;
use gpui::App;
use nocterm_ai::{BridgeCall, TerminalCall, ToolFailure};
use nocterm_transfers::{CollisionPolicy, Request, TransferEndpoint, TransferId};
use nocterm_workspace_ports::{
    ExpectedSession, TerminalCapability, TerminalEntry, TerminalError, TerminalFilesystem,
};

const DEFAULT_YIELD_MS: u64 = 1000;

/// An approved `transfer_files` call whose destination is resolved; the source is not yet.
pub(in crate::thread) struct TransferAdmission {
    call: BridgeCall,
    destination: TerminalEntry,
    expected: ExpectedSession,
    source_expected: ExpectedSession,
}
/// The submitted transfer and the call waiting for its report.
pub(in crate::thread) struct StartedTransfer {
    call: BridgeCall,
    id: TransferId,
    yield_ms: u64,
}
impl StartedTransfer {
    pub(in crate::thread) fn into_parts(self) -> (BridgeCall, TransferId, u64) {
        (self.call, self.id, self.yield_ms)
    }
}
/// A transfer call that could not start, with the failure to answer it with. Boxed so the
/// `Err` side of the start path stays small.
pub(in crate::thread) struct RejectedTransfer {
    call: BridgeCall,
    error: ToolFailure,
}
impl RejectedTransfer {
    fn new(call: BridgeCall, error: ToolFailure) -> Box<Self> {
        Box::new(Self { call, error })
    }
    pub(in crate::thread) fn into_parts(self) -> (BridgeCall, ToolFailure) {
        (self.call, self.error)
    }
}
struct PreparedTransfer {
    call: BridgeCall,
    request: Request,
    binding: TransferBinding,
    yield_ms: u64,
}

impl TransferAdmission {
    /// A call without a source session was approved against a terminal that is gone.
    pub(in crate::thread) fn admit(
        bound: BoundToolCall,
        destination: TerminalEntry,
        expected: ExpectedSession,
    ) -> DispatchAction {
        let Some(source_expected) = bound.source() else {
            let stale = TerminalError::StaleSession.to_string().into();
            return DispatchAction::Respond(bound.into_call(), Err(stale), 0);
        };
        DispatchAction::Transfer(Box::new(Self {
            call: bound.into_call(),
            destination,
            expected,
            source_expected,
        }))
    }
    /// Whether the call reads from the terminal with this id.
    pub(in crate::thread) fn reads_from(&self, id: &str) -> bool {
        self.call.call.source_terminal_id() == Some(id)
    }
    /// Rechecks both sessions, captures both filesystems and builds the request.
    fn prepare(
        self,
        source: Option<TerminalEntry>,
        cx: &App,
    ) -> Result<PreparedTransfer, Box<RejectedTransfer>> {
        match self.endpoints(source, cx) {
            Ok((request, binding, yield_ms)) => Ok(PreparedTransfer {
                call: self.call,
                request,
                binding,
                yield_ms,
            }),
            Err(error) => Err(RejectedTransfer::new(self.call, error)),
        }
    }
    fn endpoints(
        &self,
        source: Option<TerminalEntry>,
        cx: &App,
    ) -> Result<(Request, TransferBinding, u64), ToolFailure> {
        let TerminalCall::TransferFiles(call) = &self.call.call else {
            return Err("This is not a file transfer.".into());
        };
        let source = source.ok_or_else(|| {
            crate::thread::tools::unreachable(
                "Terminal is not attached or was closed. Call list_terminals for the current ones.",
            )
        })?;
        let from = endpoint(&source, self.source_expected, cx)?;
        let to = endpoint(&self.destination, self.expected, cx)?;
        let collisions = if call.overwrite {
            CollisionPolicy::Replace
        } else {
            CollisionPolicy::Skip
        };
        let request = Request::between(
            from,
            call.sources.clone(),
            to,
            call.destination.clone(),
            collisions,
        )
        .map_err(|error| ToolFailure::from(error.to_string()))?;
        let binding = TransferBinding::new(
            (call.destination_terminal_id.clone(), self.expected),
            (call.source_terminal_id.clone(), self.source_expected),
        );
        Ok((request, binding, call.yield_ms.unwrap_or(DEFAULT_YIELD_MS)))
    }
}
impl PreparedTransfer {
    fn submit(self, book: &mut ChatTransfers) -> Result<StartedTransfer, Box<RejectedTransfer>> {
        match book.start(self.request, self.binding) {
            Ok(id) => Ok(StartedTransfer {
                call: self.call,
                id,
                yield_ms: self.yield_ms,
            }),
            Err(error) => Err(RejectedTransfer::new(self.call, error)),
        }
    }
}
/// Submits an admitted transfer to the chat's book.
pub(super) fn start(
    book: &mut ChatTransfers,
    admission: TransferAdmission,
    source: Option<TerminalEntry>,
    cx: &App,
) -> Result<StartedTransfer, Box<RejectedTransfer>> {
    admission.prepare(source, cx)?.submit(book)
}

fn endpoint(
    entry: &TerminalEntry,
    expected: ExpectedSession,
    cx: &App,
) -> Result<TransferEndpoint, ToolFailure> {
    let current = entry
        .capabilities
        .state()
        .info(cx)
        .is_some_and(|info| info.expected == expected);
    let files = if current {
        entry
            .capabilities
            .files()
            .ok_or(TerminalError::Unsupported(TerminalCapability::Files))
            .and_then(|port| port.filesystem(expected, cx))
    } else {
        Err(TerminalError::StaleSession)
    };
    match files {
        Ok(TerminalFilesystem::Local) => Ok(TransferEndpoint::Local),
        Ok(TerminalFilesystem::Remote { host, fs }) => Ok(TransferEndpoint::Remote { host, fs }),
        Err(error) => Err(error.to_string().into()),
    }
}
