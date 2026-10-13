//! Native agent workspace preparation, invoked by the host background executor.
use futures::future::BoxFuture;
use nocterm_ai::{
    sandbox::SandboxPolicy,
    workspace::{PreparedWorkspace, WorkdirError, WorkdirRequest, WorkdirResolver},
};
use std::{path::PathBuf, sync::Arc};
#[derive(Clone, Default)]
enum Home {
    #[default]
    Current,
    Supplied(Option<PathBuf>),
}
#[derive(Clone, Default)]
pub struct NativeWorkdirResolver {
    home: Home,
}
impl NativeWorkdirResolver {
    pub fn with_home(home: Option<PathBuf>) -> Self {
        Self {
            home: Home::Supplied(home),
        }
    }
}
impl WorkdirResolver for NativeWorkdirResolver {
    fn prepare(
        &self,
        request: WorkdirRequest,
    ) -> BoxFuture<'static, Result<PreparedWorkspace, WorkdirError>> {
        let home = self.home.clone();
        Box::pin(async move {
            if !request.candidate.is_absolute() {
                return Err(WorkdirError::InvalidDirectory);
            }
            if request.create_private {
                nocterm_core::paths::ensure_private_dir(&request.candidate).map_err(|source| {
                    WorkdirError::CreatePrivate {
                        path: request.candidate.clone(),
                        source: Arc::new(source),
                    }
                })?;
            }
            let root =
                super::sandbox::resolution::PathResolver::required_directory(&request.candidate)?
                    .resolved;
            let sandbox = request
                .isolation
                .map(|paths| {
                    let home = match home {
                        Home::Current => std::env::home_dir(),
                        Home::Supplied(home) => home,
                    };
                    let policy = SandboxPolicy::new(
                        &root,
                        home.as_deref(),
                        &paths.private_dirs,
                        &paths.shared_dirs,
                    );
                    let facts = super::sandbox::resolution::PathResolver::resolve(&policy, &root)?;
                    policy.validate_workdir(&facts)?;
                    Ok::<_, WorkdirError>(policy)
                })
                .transpose()?;
            Ok(PreparedWorkspace { root, sandbox })
        })
    }
}
#[cfg(test)]
mod tests;
