//! Literal controls project a checked domain draft; the host cannot be retargeted.
use gpui_kit::{
    App, AppContext as _, Entity, FocusHandle, Focusable as _, SharedString, Subscription, Window,
    base::TestSupportExt as _,
    component::{
        ActiveTheme as _, StyledExt as _,
        input::{Input, InputState},
        v_flex,
    },
    div,
    prelude::*,
};
use nocterm_project_model::{
    HostLocator, ProjectDefinition, ProjectDraft, ProjectId, ProjectIdentity, ProjectSource,
    ProjectValidationError, RootPath,
};
use nocterm_workspace_ports::ProjectCreationSource;
use std::rc::Rc;

pub(super) struct ProjectControls {
    original: Option<ProjectDefinition>,
    metadata: ProjectMetadata,
    name: Entity<InputState>,
    path: Entity<InputState>,
    server: SharedString,
}
struct ProjectMetadata {
    id: ProjectId,
    host: HostLocator,
    source: ProjectSource,
}
impl ProjectControls {
    pub(super) fn create(
        identity: ProjectIdentity,
        source: &ProjectCreationSource,
        window: &mut Window,
        cx: &mut App,
    ) -> Self {
        let path = source
            .cwd()
            .and_then(|cwd| cwd.to_str())
            .unwrap_or("")
            .to_owned();
        let metadata = ProjectMetadata {
            id: identity.id,
            host: source.parent().host().clone(),
            source: ProjectSource::Manual,
        };
        let name = cx.new(|cx| InputState::new(window, cx).placeholder("Project name"));
        let path = cx.new(|cx| {
            let mut input = InputState::new(window, cx).placeholder("/srv/my-project");
            input.set_value(path, window, cx);
            input
        });
        Self {
            original: None,
            metadata,
            name,
            path,
            server: source.parent().title().clone(),
        }
    }
    pub(super) fn edit(original: ProjectDefinition, window: &mut Window, cx: &mut App) -> Self {
        let server = match original.host() {
            nocterm_project_model::HostLocator::Local => "This computer".into(),
            nocterm_project_model::HostLocator::Ssh { target, .. }
            | nocterm_project_model::HostLocator::SshDirect { target, .. } => {
                target.to_string().into()
            }
        };
        let name = cx.new(|cx| {
            let mut input = InputState::new(window, cx).placeholder("Project name");
            input.set_value(original.name(), window, cx);
            input
        });
        let path = cx.new(|cx| {
            let mut input = InputState::new(window, cx).placeholder("/srv/my-project");
            input.set_value(original.root().as_str(), window, cx);
            input
        });
        let metadata = ProjectMetadata {
            id: original.id(),
            host: original.host().clone(),
            source: original.source().clone(),
        };
        Self {
            original: Some(original),
            metadata,
            name,
            path,
            server,
        }
    }
    pub(super) fn original(&self) -> Option<&ProjectDefinition> {
        self.original.as_ref()
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
        [&self.name, &self.path]
            .into_iter()
            .map(|input| {
                let changed = changed.clone();
                cx.observe(input, move |_, cx| changed(cx))
            })
            .collect()
    }
    pub(super) fn candidate(
        &self,
        timestamp: u64,
        cx: &App,
    ) -> Result<ProjectDefinition, ProjectValidationError> {
        ProjectDraft {
            id: self.metadata.id,
            host: self.metadata.host.clone(),
            source: self.metadata.source.clone(),
            name: self.name.read(cx).value().trim().to_owned(),
            root: RootPath::parse(self.path.read(cx).value().to_string())?,
            updated: timestamp,
        }
        .try_into()
    }
    pub(super) fn render(&self, pending: bool, cx: &App) -> impl IntoElement + use<> {
        let theme = cx.theme().clone();
        v_flex()
            .gap_3()
            .child(
                div()
                    .id("project-form-server")
                    .test_support()
                    .text_sm()
                    .text_color(theme.muted_foreground)
                    .child(self.server.clone()),
            )
            .child(
                v_flex()
                    .gap_1()
                    .child(div().text_sm().font_medium().child("Name"))
                    .child(
                        div()
                            .id("project-name")
                            .test_support()
                            .child(Input::new(&self.name).disabled(pending)),
                    ),
            )
            .child(
                v_flex()
                    .gap_1()
                    .child(div().text_sm().font_medium().child("Folder on this server"))
                    .child(
                        div()
                            .id("project-path")
                            .test_support()
                            .child(Input::new(&self.path).disabled(pending)),
                    ),
            )
    }
}
