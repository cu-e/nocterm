//! Explicitly supplied workspace results, without native filesystem validation.
use crate::{
    connection::ManagedStartupHandle,
    history::source::{ManagedCommandError, ManagedSourceFuture, SourceContext, SourceLifetime},
    model::{
        json::deferred::{JsonDecodeError, NativeAllocationError},
        native::{NativeList, NativeOwned},
    },
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
    requests: Vec<RequestObservation>,
    replies: HashMap<PathBuf, Result<PreparedWorkspace, WorkdirError>>,
    echo: bool,
    pending: HashMap<PathBuf, BoxFuture<'static, Result<PreparedWorkspace, WorkdirError>>>,
}
enum RequestObservation {
    Legacy(WorkdirRequest),
    Managed(ManagedStartupHandle),
}
impl RequestObservation {
    fn request(&self) -> WorkdirRequest {
        match self {
            Self::Legacy(request) => request.clone(),
            Self::Managed(startup) => {
                let facts = startup.facts();
                WorkdirRequest {
                    candidate: facts.working_directory().to_owned(),
                    create_private: facts.create_private(),
                    isolation: facts
                        .isolation()
                        .map(|paths| crate::workspace::IsolationPaths {
                            private_dirs: paths.private_dirs().to_vec(),
                            shared_dirs: paths.shared_dirs().to_vec(),
                        }),
                }
            }
        }
    }
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
        self.0
            .lock()
            .expect("workspace script")
            .requests
            .iter()
            .map(RequestObservation::request)
            .collect()
    }
}
impl WorkdirResolver for ScriptedWorkdirResolver {
    fn prepare_managed(
        &self,
        startup: ManagedStartupHandle,
        lifetime: SourceLifetime,
    ) -> Result<
        ManagedSourceFuture<Result<NativeOwned<PreparedWorkspace>, ManagedCommandError>>,
        NativeAllocationError,
    > {
        let context = lifetime.context_ref().clone();
        if !startup.belongs_to_source(&context) {
            return Err(crate::model::retention::RetentionError::ForeignAuthority.into());
        }
        let script = self.0.clone();
        ManagedSourceFuture::new(lifetime, async move {
            let pending = {
                let mut script = script.lock().expect("workspace script");
                script
                    .requests
                    .push(RequestObservation::Managed(startup.clone()));
                script.pending.remove(startup.facts().working_directory())
            };
            if let Some(pending) = pending {
                let result = pending.await;
                return managed_reply(&context, &result);
            }
            let script = script.lock().expect("workspace script");
            if let Some(result) = script.replies.get(startup.facts().working_directory()) {
                return managed_reply(&context, result);
            }
            let facts = startup.facts();
            if !script.echo || !facts.working_directory().is_absolute() {
                return managed_failure(&context, &WorkdirError::InvalidDirectory);
            }
            let root = context
                .copy_path(facts.working_directory())
                .map_err(|error| guard_error(&context, error))?;
            let sandbox = if let Some(paths) = facts.isolation() {
                NativeOwned::supplied(
                    crate::model::native::workspace::sandbox_policy(
                        context.ledger(),
                        facts.working_directory(),
                        None,
                        paths.private_dirs(),
                        paths.shared_dirs(),
                    )
                    .map_err(|error| guard_error(&context, error))?,
                )
            } else {
                NativeOwned::absent(context.ledger())
                    .map_err(|error| guard_error(&context, error))?
            };
            crate::model::native::workspace::prepared_workspace(root, sandbox)
                .map_err(|error| guard_error(&context, error))
        })
    }
    fn prepare(
        &self,
        request: WorkdirRequest,
    ) -> BoxFuture<'static, Result<PreparedWorkspace, WorkdirError>> {
        let script = self.0.clone();
        Box::pin(async move {
            let pending = {
                let mut script = script.lock().expect("workspace script");
                script
                    .requests
                    .push(RequestObservation::Legacy(request.clone()));
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

fn guard_error<E: std::error::Error + Send + Sync + 'static>(
    context: &SourceContext,
    error: E,
) -> ManagedCommandError {
    match context.prepare_error::<E>(0) {
        Ok(slot) => ManagedCommandError::Native(slot.capture(error)),
        Err(refusal) => ManagedCommandError::Admission(refusal),
    }
}
fn paths(
    context: &SourceContext,
    values: &[PathBuf],
) -> Result<NativeOwned<Vec<PathBuf>>, JsonDecodeError> {
    let mut result = NativeList::new(context.ledger(), values.len())?;
    for path in values {
        result.push(context.copy_path(path)?)?;
    }
    Ok(result.finish())
}
fn managed_reply(
    context: &SourceContext,
    reply: &Result<PreparedWorkspace, WorkdirError>,
) -> Result<NativeOwned<PreparedWorkspace>, ManagedCommandError> {
    let workspace = match reply {
        Ok(workspace) => workspace,
        Err(error) => return managed_failure(context, error),
    };
    let root = context
        .copy_path(&workspace.root)
        .map_err(|error| guard_error(context, error))?;
    let sandbox = if let Some(policy) = &workspace.sandbox {
        let writable =
            paths(context, &policy.writable).map_err(|error| guard_error(context, error))?;
        let hidden = paths(context, &policy.hidden).map_err(|error| guard_error(context, error))?;
        let readable =
            paths(context, &policy.readable).map_err(|error| guard_error(context, error))?;
        let credentials =
            paths(context, &policy.credentials).map_err(|error| guard_error(context, error))?;
        NativeOwned::supplied(
            crate::model::native::workspace::from_paths(writable, hidden, readable, credentials)
                .map_err(|error| guard_error(context, error))?,
        )
    } else {
        NativeOwned::absent(context.ledger()).map_err(|error| guard_error(context, error))?
    };
    crate::model::native::workspace::prepared_workspace(root, sandbox)
        .map_err(|error| guard_error(context, error))
}
fn managed_failure<T>(
    context: &SourceContext,
    error: &WorkdirError,
) -> Result<T, ManagedCommandError> {
    let nested = match error {
        WorkdirError::CreatePrivate { path, .. } | WorkdirError::Resolve { path, .. } => {
            path.as_os_str().len()
        }
        _ => 0,
    };
    let slot = context
        .prepare_error::<WorkdirError>(nested)
        .map_err(ManagedCommandError::Admission)?;
    Err(ManagedCommandError::Native(slot.capture(error.clone())))
}

#[cfg(test)]
mod tests;
