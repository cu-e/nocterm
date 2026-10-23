use crate::ProjectOpenError;
use gpui_kit::{App, Context, Subscription, Window};
use nocterm_project_model::{Catalog, ProjectId};
use nocterm_project_service::ProjectsHandle;
use nocterm_workspace::{ProjectTerminalOpen, Workspace};
use std::rc::Rc;

#[derive(Clone)]
pub struct Installed {
    model: ProjectsHandle,
    settings: nocterm_store::SettingsHandle,
    terminals: Rc<dyn ProjectTerminalOpen>,
    resolver: Option<Rc<dyn nocterm_workspace::ProjectTerminalResolve>>,
    setup: Option<std::sync::Arc<dyn nocterm_project_providers::ProjectPreparation>>,
    environment: Option<std::sync::Arc<dyn nocterm_project_providers::WorkspaceEnvironment>>,
    container_actions:
        Option<std::sync::Arc<dyn nocterm_project_providers::WorkspaceContainerActions>>,
    image_search: Option<std::sync::Arc<dyn nocterm_project_providers::ImageSearch>>,
    local: Option<std::sync::Arc<dyn nocterm_session::HostExec>>,
    directories: Option<std::sync::Arc<dyn nocterm_explorer::directory::DirectoryReaders>>,
}
impl Installed {
    pub(crate) fn new(
        model: ProjectsHandle,
        terminals: Rc<dyn ProjectTerminalOpen>,
        settings: nocterm_store::SettingsHandle,
        cx: &mut App,
    ) -> Self {
        settings.register::<nocterm_project_providers::WorkspacesSettings>(cx);
        Self {
            model,
            settings,
            terminals,
            resolver: None,
            setup: None,
            directories: None,
            local: None,
            image_search: None,
            environment: None,
            container_actions: None,
        }
    }
    /// Supply the machine's executor without opening a terminal session.
    pub fn with_local_host(
        mut self,
        local: Option<std::sync::Arc<dyn nocterm_session::HostExec>>,
    ) -> Self {
        self.local = local;
        self
    }
    pub(crate) fn local_source(
        &self,
    ) -> Result<
        nocterm_workspace_ports::ProjectCreationSource,
        nocterm_workspace_ports::ProjectParentError,
    > {
        nocterm_workspace_ports::ProjectCreationSource::local(
            self.local
                .clone()
                .ok_or(nocterm_workspace_ports::ProjectParentError::UnavailableParent)?,
            None,
        )
    }
    pub fn with_preparation(
        mut self,
        setup: std::sync::Arc<dyn nocterm_project_providers::ProjectPreparation>,
    ) -> Self {
        self.setup = Some(setup);
        self
    }
    pub fn with_environment(
        mut self,
        environment: std::sync::Arc<dyn nocterm_project_providers::WorkspaceEnvironment>,
    ) -> Self {
        self.environment = Some(environment);
        self
    }
    pub(crate) fn environment(
        &self,
    ) -> Option<&std::sync::Arc<dyn nocterm_project_providers::WorkspaceEnvironment>> {
        self.environment.as_ref()
    }
    pub fn with_container_actions(
        mut self,
        actions: std::sync::Arc<dyn nocterm_project_providers::WorkspaceContainerActions>,
    ) -> Self {
        self.container_actions = Some(actions);
        self
    }
    pub(crate) fn container_actions(
        &self,
    ) -> Option<&std::sync::Arc<dyn nocterm_project_providers::WorkspaceContainerActions>> {
        self.container_actions.as_ref()
    }
    pub fn with_image_search(
        mut self,
        search: std::sync::Arc<dyn nocterm_project_providers::ImageSearch>,
    ) -> Self {
        self.image_search = Some(search);
        self
    }
    pub(crate) fn image_search(
        &self,
    ) -> Option<&std::sync::Arc<dyn nocterm_project_providers::ImageSearch>> {
        self.image_search.as_ref()
    }
    pub fn with_directory_readers(
        mut self,
        readers: std::sync::Arc<dyn nocterm_explorer::directory::DirectoryReaders>,
    ) -> Self {
        self.directories = Some(readers);
        self
    }
    pub(crate) fn preparation(
        &self,
    ) -> Option<&std::sync::Arc<dyn nocterm_project_providers::ProjectPreparation>> {
        self.setup.as_ref()
    }
    pub(crate) fn directories(
        &self,
    ) -> Option<&std::sync::Arc<dyn nocterm_explorer::directory::DirectoryReaders>> {
        self.directories.as_ref()
    }
    pub fn with_terminal_resolver(
        mut self,
        resolver: Rc<dyn nocterm_workspace::ProjectTerminalResolve>,
    ) -> Self {
        self.resolver = Some(resolver);
        self
    }
    pub(crate) fn resolver(&self) -> Option<&Rc<dyn nocterm_workspace::ProjectTerminalResolve>> {
        self.resolver.as_ref()
    }
    pub fn catalog<'a>(&self, cx: &'a App) -> &'a Catalog {
        self.model.catalog(cx)
    }
    pub fn open(
        &self,
        project: ProjectId,
        workspace: &mut Workspace,
        window: &mut Window,
        cx: &mut Context<Workspace>,
    ) -> Result<(), ProjectOpenError> {
        crate::opening::open(self, project, workspace, window, cx)
    }
    pub(crate) fn settings(&self) -> &nocterm_store::SettingsHandle {
        &self.settings
    }
    pub(crate) fn model(&self) -> &ProjectsHandle {
        &self.model
    }
    pub(crate) fn terminals(&self) -> &Rc<dyn ProjectTerminalOpen> {
        &self.terminals
    }
    pub(crate) fn observe(
        &self,
        cx: &mut App,
        changed: impl FnMut(&mut App) + 'static,
    ) -> Subscription {
        self.model.observe(cx, changed)
    }
}
