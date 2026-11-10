use futures::executor::block_on;
use nocterm_containers::Engine;
use nocterm_project_model::{HostLocator, RootPath};
use nocterm_project_providers::{ContainerLocation, ContainerOrigin, ContainerSessionFactory};
use nocterm_session::testing::FakeExec;
use std::sync::Arc;

fn container_sessions() -> crate::NativeContainerSessions {
    crate::NativeContainerSessions::new(Arc::new(
        nocterm_project_providers::testing::FakeWorkspaceCommandActivation::default(),
    ))
}
fn id() -> String {
    "a".repeat(64)
}

mod filesystem;
