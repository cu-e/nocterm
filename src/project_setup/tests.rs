use futures::executor::block_on;
use nocterm_containers::Engine;
use nocterm_project_model::{HostLocator, RootPath};
use nocterm_project_providers::{ContainerLocation, ContainerOrigin, ContainerSessionFactory};
use nocterm_session::testing::{ExecResponse, FakeExec};
use std::sync::Arc;

fn container_sessions() -> nocterm_project_container_fs::NativeContainerSessions {
    nocterm_project_container_fs::NativeContainerSessions::new(Arc::new(
        nocterm_project_runtime::NativeWorkspaceSessions,
    ))
}
fn id() -> String {
    "a".repeat(64)
}
fn answer(exec: &FakeExec, prefix: &str, stdout: String) {
    exec.respond(prefix, Ok(ExecResponse::text(stdout, 0, "")));
}

mod session;
