//! Off-thread acquisition and explicit composition of workspace installations.
mod connect;

use gpui_kit::App;
use nocterm_core::Paths;
use nocterm_project_model::{Catalog, ProjectRepository};
use nocterm_project_service::{PreparedProjects, ProjectsError, ProjectsHandle};
use nocterm_store::{ShutdownCoordinator, SnapshotWriter, StoreError};
use std::sync::Arc;

pub(crate) fn terminal_resolver(
    terminal: &nocterm_terminal::Installed,
    connections: Option<&nocterm_connections::Installed>,
) -> std::rc::Rc<dyn nocterm_workspace::ProjectTerminalResolve> {
    std::rc::Rc::new(connect::TerminalResolver::new(
        terminal.clone(),
        connections.cloned(),
    ))
}

pub(crate) fn chat_connection_opener(
    terminal: &nocterm_terminal::Installed,
    connections: &nocterm_connections::Installed,
) -> std::rc::Rc<dyn nocterm_workspace::ChatConnectionOpen> {
    std::rc::Rc::new(connect::TerminalResolver::new(
        terminal.clone(),
        Some(connections.clone()),
    ))
}

pub(crate) struct PreparedProjectSetup {
    catalog: PreparedProjects,
    writer: SnapshotWriter<Catalog>,
}
impl PreparedProjectSetup {
    /// Called with connection/history acquisition on the startup executor.
    pub(crate) fn native(paths: &Paths) -> Self {
        let repository = Arc::new(nocterm_project_storage::FileProjectRepository::new(
            paths.projects_file(),
        ));
        let catalog = PreparedProjects::loaded(repository.load(), true);
        let writer: SnapshotWriter<Catalog> = Arc::new(move |catalog| {
            let repository = repository.clone();
            Box::pin(async move { repository.save(&catalog).map_err(StoreError::backend) })
        });
        Self { catalog, writer }
    }
    #[cfg(test)]
    pub(crate) fn memory() -> Self {
        Self {
            catalog: PreparedProjects::loaded(Ok(None), false),
            writer: Arc::new(|_| Box::pin(async { Ok(()) })),
        }
    }
    pub(crate) fn install(
        self,
        terminal: &nocterm_terminal::Installed,
        connections: Option<&nocterm_connections::Installed>,
        shutdown: &ShutdownCoordinator,
        cx: &mut App,
    ) -> Result<nocterm_projects::Installed, ProjectsError> {
        let model = ProjectsHandle::install(
            self.catalog,
            self.writer,
            Arc::new(nocterm_project_storage::NativeProjectMetadata),
            shutdown,
            cx,
        )?;
        Ok(
            nocterm_projects::init(model, nocterm_terminal::project_terminal_opener(terminal))
                .with_terminal_resolver(terminal_resolver(terminal, connections)),
        )
    }
}
