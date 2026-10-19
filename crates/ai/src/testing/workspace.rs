//! Explicitly supplied workspace results, without native filesystem validation.
use crate::{
    sandbox::SandboxPolicy,
    workspace::{PreparedWorkspace, WorkdirError, WorkdirRequest, WorkdirResolver},
};
use futures::{channel::oneshot, future::BoxFuture};
use std::{
    collections::HashMap,
    path::PathBuf,
    sync::{Arc, Mutex},
};
#[derive(Default)]
struct Script {
    requests: Vec<WorkdirRequest>,
    replies: HashMap<PathBuf, Result<PreparedWorkspace, WorkdirError>>,
    echo: bool,
    pending: HashMap<PathBuf, BoxFuture<'static, Result<PreparedWorkspace, WorkdirError>>>,
}
#[derive(Clone, Default)]
pub struct ScriptedWorkdirResolver(Arc<Mutex<Script>>);
impl ScriptedWorkdirResolver {
    /// Fake policy facts are based on the request, never on filesystem existence.
    pub fn accept_requests() -> Self {
        Self(Arc::new(Mutex::new(Script {
            echo: true,
            ..Default::default()
        })))
    }
    pub fn supply(&self, candidate: PathBuf, result: Result<PreparedWorkspace, WorkdirError>) {
        self.0
            .lock()
            .expect("workspace script")
            .replies
            .insert(candidate, result);
    }
    pub fn pending(
        &self,
        candidate: PathBuf,
    ) -> oneshot::Sender<Result<PreparedWorkspace, WorkdirError>> {
        let (sender, receiver) = oneshot::channel();
        self.0.lock().expect("workspace script").pending.insert(
            candidate,
            Box::pin(async move {
                receiver
                    .await
                    .unwrap_or(Err(WorkdirError::InvalidDirectory))
            }),
        );
        sender
    }
    pub fn requests(&self) -> Vec<WorkdirRequest> {
        self.0.lock().expect("workspace script").requests.clone()
    }
}
impl WorkdirResolver for ScriptedWorkdirResolver {
    fn prepare(
        &self,
        request: WorkdirRequest,
    ) -> BoxFuture<'static, Result<PreparedWorkspace, WorkdirError>> {
        let script = self.0.clone();
        Box::pin(async move {
            let pending = {
                let mut script = script.lock().expect("workspace script");
                script.requests.push(request.clone());
                script.pending.remove(&request.candidate)
            };
            if let Some(pending) = pending {
                return pending.await;
            }
            let script = script.lock().expect("workspace script");
            if let Some(result) = script.replies.get(&request.candidate) {
                return result.clone();
            }
            if !script.echo || !request.candidate.is_absolute() {
                return Err(WorkdirError::InvalidDirectory);
            }
            let sandbox = request.isolation.map(|paths| {
                SandboxPolicy::new(
                    &request.candidate,
                    None,
                    &paths.private_dirs,
                    &paths.shared_dirs,
                )
            });
            Ok(PreparedWorkspace {
                root: request.candidate,
                sandbox,
            })
        })
    }
}
