//! Literal metadata and one current source retain their checked creation identity.
use super::navigation::WizardStep;
use super::options::{WorkspaceDraftChanged, WorkspaceOptions};
use crate::{Installed, ProjectOpenError};
use gpui_kit::{
    App, AppContext as _, Entity, FocusHandle, Focusable as _, SharedString, Subscription, Window,
    base::TestSupportExt as _,
    component::{
        ActiveTheme as _, StyledExt as _,
        input::{Input, InputEvent, InputState},
        v_flex,
    },
    div,
    prelude::*,
};
use nocterm_project_model::{
    ProjectDefinition, ProjectDraft, ProjectId, ProjectIdentity, ProjectSource, ProjectTarget,
    RootPath,
};
use nocterm_project_providers::{ContainerLocation, ProjectSetupRequest};
use nocterm_workspace_ports::ProjectCreationSource;
use std::rc::Rc;

pub(super) struct ProjectControls {
    original: Option<ProjectDefinition>,
    metadata: ProjectMetadata,
    name: Entity<InputState>,
    path: Entity<InputState>,
    options: Option<Entity<WorkspaceOptions>>,
    source: Option<ProjectCreationSource>,
    server: SharedString,
}
struct ProjectMetadata {
    id: ProjectId,
    target: Option<ProjectTarget>,
    source: ProjectSource,
}
impl ProjectControls {
    pub(super) fn create(
        identity: ProjectIdentity,
        source: &ProjectCreationSource,
        installed: &Installed,
        window: &mut Window,
        cx: &mut App,
    ) -> Self {
        let name = cx.new(|cx| InputState::new(window, cx).placeholder("Project name"));
        let path = cx.new(|cx| InputState::new(window, cx));
        let options =
            cx.new(|cx| WorkspaceOptions::new(source.clone(), installed.clone(), window, cx));
        Self {
            original: None,
            metadata: ProjectMetadata {
                id: identity.id,
                target: None,
                source: ProjectSource::Manual,
            },
            name,
            path,
            options: Some(options),
            source: Some(source.clone()),
            server: source.parent().title().clone(),
        }
    }
    pub(super) fn edit(original: ProjectDefinition, window: &mut Window, cx: &mut App) -> Self {
        let server = crate::location::host(original.target())
            .map(|host| match host {
                nocterm_project_model::HostLocator::Local => "This computer".into(),
                nocterm_project_model::HostLocator::Ssh { target, .. }
                | nocterm_project_model::HostLocator::SshDirect { target, .. } => {
                    target.to_string().into()
                }
            })
            .unwrap_or_else(|| crate::location::label(original.target()).into());
        let name = cx.new(|cx| {
            let mut input = InputState::new(window, cx).placeholder("Project name");
            input.set_value(original.name(), window, cx);
            input
        });
        let path = cx.new(|cx| {
            let mut input = InputState::new(window, cx).placeholder("/srv/my-project");
            input.set_value(crate::location::label(original.target()), window, cx);
            input
        });
        Self {
            metadata: ProjectMetadata {
                id: original.id(),
                target: Some(original.target().clone()),
                source: original.source().clone(),
            },
            original: Some(original),
            name,
            path,
            options: None,
            source: None,
            server,
        }
    }
    pub(super) fn new_container(&self, cx: &App) -> bool {
        self.options
            .as_ref()
            .is_some_and(|options| options.read(cx).new_container())
    }
    pub(super) fn show_step(&self, step: WizardStep, cx: &mut App) {
        if let Some(options) = &self.options {
            options.update(cx, |options, cx| options.show_step(step, cx));
        }
    }
    pub(super) fn step_focus(&self, step: WizardStep, cx: &App) -> FocusHandle {
        if step == WizardStep::Project {
            self.name.focus_handle(cx)
        } else {
            self.options
                .as_ref()
                .map(|options| options.read(cx).focus(step, cx))
                .unwrap_or_else(|| self.focus(cx))
        }
    }
    pub(super) fn validate(
        &self,
        step: WizardStep,
        cx: &App,
    ) -> Result<(), (ProjectOpenError, FocusHandle)> {
        if step == WizardStep::Project {
            let target = ProjectTarget::HostFolder {
                host: self
                    .source
                    .as_ref()
                    .map(|source| source.parent().host().clone())
                    .unwrap_or(nocterm_project_model::HostLocator::Local),
                root: RootPath::parse("/").map_err(|error| (error.into(), self.focus(cx)))?,
            };
            self.candidate(0, Some(target), cx)
                .map_err(|error| (error, self.focus(cx)))?;
        }
        if let Some(options) = &self.options {
            options
                .read(cx)
                .validate(step, cx)
                .map_err(|(error, focus)| (error.into(), focus))?;
        }
        Ok(())
    }
    pub(super) fn server_header(&self, cx: &App) -> impl IntoElement + use<> {
        div()
            .id("project-form-server")
            .test_support()
            .w_full()
            .min_w_0()
            .truncate()
            .aria_label(self.server.clone())
            .text_sm()
            .text_color(cx.theme().muted_foreground)
            .child(self.server.clone())
    }
    pub(super) fn original(&self) -> Option<&ProjectDefinition> {
        self.original.as_ref()
    }
    pub(super) fn lock(&self, locked: bool, cx: &mut App) {
        if let Some(options) = &self.options {
            options.update(cx, |options, cx| options.lock(locked, cx));
        }
    }
    pub(super) fn directory_check(
        &self,
        cx: &App,
    ) -> Option<(
        std::sync::Arc<dyn nocterm_explorer::directory::DirectoryRead>,
        nocterm_explorer::directory::Directory,
    )> {
        self.options.as_ref()?.read(cx).directory_check(cx)
    }
    pub(super) fn source(&self) -> Option<&ProjectCreationSource> {
        self.source.as_ref()
    }
    pub(super) fn focus(&self, cx: &App) -> FocusHandle {
        self.name.focus_handle(cx)
    }
    pub(super) fn observe(
        &self,
        cx: &mut App,
        changed: impl Fn(&mut App) + 'static,
    ) -> Vec<Subscription> {
        let changed = Rc::new(changed);
        let mut subscriptions: Vec<_> = [&self.name, &self.path]
            .into_iter()
            .map(|input| {
                let changed = changed.clone();
                cx.subscribe(input, move |_, event: &InputEvent, cx| {
                    if matches!(event, InputEvent::Change) {
                        changed(cx);
                    }
                })
            })
            .collect();
        if let Some(options) = &self.options {
            subscriptions
                .push(cx.subscribe(options, move |_, _: &WorkspaceDraftChanged, cx| changed(cx)));
        }
        subscriptions
    }
    pub(super) fn setup_request(
        &self,
        cx: &App,
    ) -> Result<Option<ProjectSetupRequest>, ProjectOpenError> {
        self.options
            .as_ref()
            .map(|options| {
                options
                    .read(cx)
                    .request(self.metadata.id, cx)
                    .map_err(Into::into)
            })
            .transpose()
    }
    pub(super) fn candidate(
        &self,
        timestamp: u64,
        target: Option<ProjectTarget>,
        cx: &App,
    ) -> Result<ProjectDefinition, ProjectOpenError> {
        let target = match target {
            Some(target) => target,
            None => match self.metadata.target.as_ref() {
                Some(ProjectTarget::HostFolder { host, .. }) => ProjectTarget::HostFolder {
                    host: host.clone(),
                    root: RootPath::parse(self.path.read(cx).value().to_string())?,
                },
                Some(target @ ProjectTarget::Provider { .. }) => {
                    match ContainerLocation::from_target(target) {
                        Ok(location) => location
                            .with_root(RootPath::parse(self.path.read(cx).value().to_string())?)
                            .to_target()?,
                        Err(_) => target.clone(),
                    }
                }
                None => {
                    return Err(nocterm_project_providers::ProjectSetupError::InvalidInput(
                        "choose a workspace location",
                    )
                    .into());
                }
            },
        };
        Ok(ProjectDraft {
            id: self.metadata.id,
            name: self.name.read(cx).value().trim().to_owned(),
            target,
            source: self.metadata.source.clone(),
            updated: timestamp,
        }
        .try_into()?)
    }
    pub(super) fn render(
        &self,
        pending: bool,
        step: WizardStep,
        _cx: &App,
    ) -> impl IntoElement + use<> {
        v_flex()
            .gap_3()
            .when(
                self.original.is_some() || step == WizardStep::Project,
                |form| {
                    form.child(
                        v_flex()
                            .gap_1()
                            .child(div().text_sm().font_medium().child("Name"))
                            .child(
                                div().id("project-name").test_support().child(
                                    Input::new(&self.name)
                                        .id("input")
                                        .aria_label("Workspace name")
                                        .disabled(pending),
                                ),
                            ),
                    )
                },
            )
            .when_some(self.options.clone(), |form, options| form.child(options))
            .when(self.options.is_none(), |form| {
                form.child(
                    v_flex()
                        .gap_1()
                        .child(div().text_sm().font_medium().child("Project directory"))
                        .child(div().id("project-path").test_support().child(
                            Input::new(&self.path).disabled(
                                pending
                                    || self.metadata.target.as_ref().is_none_or(|target| {
                                        crate::location::root(target).is_none()
                                    }),
                            ),
                        )),
                )
            })
    }
}
