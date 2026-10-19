use super::{Gate, lock};
use crate::{ExecError, ExecExit, ExecFuture, ExecOutput, ExecRequest, ExecSink, HostExec};
use crate::{ProgramOpenError, ProgramOpenFuture, ProgramOpenRequest, ProgramOpener};
use futures::FutureExt as _;
use std::{
    collections::VecDeque,
    sync::{Arc, Mutex},
};

/// A short, repeatable program answer. For long streaming responses, use
/// `FakeExec::queue_stream` and drive the real sink concurrently with the reader.
#[derive(Clone)]
pub struct ExecResponse {
    chunks: Vec<Vec<u8>>,
    exit: ExecExit,
}

impl ExecResponse {
    pub fn text(stdout: impl Into<Vec<u8>>, status: u32, stderr: impl Into<String>) -> Self {
        Self {
            chunks: vec![stdout.into()],
            exit: ExecExit {
                status: Some(status),
                stderr: stderr.into(),
            },
        }
    }

    /// At most the port's 16-chunk backlog can be preloaded without a worker.
    pub fn chunks(chunks: Vec<Vec<u8>>, exit: ExecExit) -> Result<Self, ExecError> {
        if chunks.len() > 16 {
            return Err(ExecError::Unsupported);
        }
        Ok(Self { chunks, exit })
    }

    fn output(&self) -> Result<ExecOutput, ExecError> {
        let (sink, output) = ExecOutput::channel();
        for chunk in &self.chunks {
            if sink.send(chunk.clone()).now_or_never() != Some(true) {
                return Err(ExecError::Disconnected);
            }
        }
        sink.finish(self.exit.clone());
        Ok(output)
    }
}

#[derive(Default)]
struct ExecScript {
    rules: Vec<(String, Result<ExecResponse, ExecError>)>,
    streams: VecDeque<(String, ExecOutput)>,
    requests: VecDeque<ExecRequest>,
    gate: Option<Gate>,
}

#[derive(Default)]
struct OpenScript {
    replies: VecDeque<Result<(), ProgramOpenError>>,
    requests: VecDeque<ProgramOpenRequest>,
    gate: Option<Gate>,
}

/// Prefix-matched repeatable answers and single-use streams. Records requests
/// before suspension; streaming uses the actual port's bounded output channel.
#[derive(Clone, Default)]
pub struct FakeExec {
    state: Arc<Mutex<ExecScript>>,
    opens: Arc<Mutex<OpenScript>>,
}

impl FakeExec {
    pub fn new(answers: Vec<(&str, Result<(&str, u32, &str), ExecError>)>) -> Self {
        let exec = Self::default();
        for (prefix, answer) in answers {
            exec.respond(
                prefix,
                answer.map(|(out, status, err)| ExecResponse::text(out, status, err)),
            );
        }
        exec
    }

    pub fn respond(&self, prefix: impl Into<String>, answer: Result<ExecResponse, ExecError>) {
        lock(&self.state).rules.push((prefix.into(), answer));
    }

    /// Queue an externally driven program. A stream is consumed by the first
    /// matching request; repeatable rules are checked only after queued streams.
    pub fn queue_stream(&self, prefix: impl Into<String>) -> ExecSink {
        let (sink, output) = ExecOutput::channel();
        lock(&self.state).streams.push_back((prefix.into(), output));
        sink
    }

    pub fn suspend(&self, gate: Gate) {
        lock(&self.state).gate = Some(gate);
    }

    pub fn requests(&self) -> Vec<ExecRequest> {
        lock(&self.state).requests.iter().cloned().collect()
    }

    pub fn ran(&self) -> Vec<String> {
        self.requests()
            .iter()
            .map(ExecRequest::command_line)
            .collect()
    }

    pub fn queue_open(&self, reply: Result<(), ProgramOpenError>) -> Result<(), ProgramOpenError> {
        let mut script = lock(&self.opens);
        if script.replies.len() == 4096 {
            return Err(ProgramOpenError::Busy { limit: 4096 });
        }
        script.replies.push_back(reply);
        Ok(())
    }
    pub fn opened_requests(&self) -> Vec<ProgramOpenRequest> {
        lock(&self.opens).requests.iter().cloned().collect()
    }
    pub fn suspend_open(&self, gate: Gate) {
        lock(&self.opens).gate = Some(gate);
    }
}

impl ProgramOpener for FakeExec {
    fn open(&self, request: ProgramOpenRequest) -> ProgramOpenFuture {
        let opens = self.opens.clone();
        Box::pin(async move {
            let (gate, reply) = {
                let mut script = lock(&opens);
                if script.requests.len() == 4096 {
                    script.requests.pop_front();
                }
                script.requests.push_back(request);
                (
                    script.gate.clone(),
                    script.replies.pop_front().unwrap_or(Ok(())),
                )
            };
            if let Some(gate) = gate {
                gate.wait().await;
            }
            reply
        })
    }
}

impl HostExec for FakeExec {
    fn exec(&self, request: ExecRequest) -> ExecFuture<ExecOutput> {
        let (gate, response) = {
            let mut script = lock(&self.state);
            let line = request.command_line();
            if script.requests.len() == 4096 {
                script.requests.pop_front();
            }
            script.requests.push_back(request.clone());
            let response = if let Some(index) = script
                .streams
                .iter()
                .position(|(prefix, _)| line.starts_with(prefix))
            {
                script
                    .streams
                    .remove(index)
                    .map(|(_, output)| Ok(output))
                    .unwrap_or(Err(ExecError::Disconnected))
            } else {
                script
                    .rules
                    .iter()
                    .find(|(prefix, _)| line.starts_with(prefix))
                    .map(|(_, answer)| answer.clone().and_then(|answer| answer.output()))
                    .unwrap_or_else(|| Err(ExecError::NotFound(request.program)))
            };
            (script.gate.clone(), response)
        };
        Box::pin(async move {
            if let Some(gate) = gate {
                gate.wait().await;
            }
            response
        })
    }
}
