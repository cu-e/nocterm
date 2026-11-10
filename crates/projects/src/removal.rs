//! Compact removal confirmation, retaining the original definition and host receipt.
mod source;
use crate::Installed;
use gpui_kit::{
    App, AppContext as _, Context, FocusHandle, WeakEntity, Window,
    base::TestSupportExt as _,
    component::{
        ActiveTheme as _, Disableable as _, Sizable as _, WindowExt as _,
        button::{Button, ButtonVariants as _},
        h_flex,
        radio::Radio,
        v_flex,
    },
    div,
    prelude::*,
    px,
};
use nocterm_project_model::ProjectDefinition;
use nocterm_project_providers::{ContainerAction, ContainerActionError, ContainerLocation};
use nocterm_project_service::{
    ProjectBinding, ProjectsError, RemovalContainerAction, RemovalOperation, WorkspaceRemovalError,
};
use nocterm_workspace::Workspace;
use source::CapturedRemoval;

pub(crate) fn open(
    installed: Installed,
    definition: ProjectDefinition,
    workspace: WeakEntity<Workspace>,
    window: &mut Window,
    cx: &mut App,
) {
    let dialog = cx.new(|cx| RemoveWorkspace::new(installed, definition, workspace, window, cx));
    let content = dialog.clone();
    window.open_dialog(cx, move |host, _, cx| {
        let cancel = content.downgrade();
        let confirm = content.downgrade();
        let pending = content.read(cx).pending;
        host.title(format!(
            "Remove workspace “{}”?",
            content.read(cx).definition.name()
        ))
        .w(px(480.))
        .child(content.clone())
        .close_button(false)
        .overlay_closable(false)
        .on_cancel(move |_, _, cx| {
            cancel
                .upgrade()
                .is_some_and(|dialog| !dialog.read(cx).pending)
        })
        .on_ok(move |_, window, cx| {
            let _ = confirm.update(cx, |this, cx| this.submit(window, cx));
            false
        })
        .keyboard(!pending)
    });
    let focus = window.focused(cx);
    dialog.update(cx, |this, _| this.dialog_focus = focus);
}
#[derive(Clone, Copy, Default, PartialEq, Eq)]
enum Choice {
    #[default]
    Keep,
    Stop,
    Delete,
}
impl Choice {
    fn action(self) -> Option<ContainerAction> {
        match self {
            Self::Keep => None,
            Self::Stop => Some(ContainerAction::Stop),
            Self::Delete => Some(ContainerAction::Delete),
        }
    }
    fn button(self) -> &'static str {
        match self {
            Self::Keep => "Remove workspace",
            Self::Stop => "Stop and remove workspace",
            Self::Delete => "Delete container and workspace",
        }
    }
}
struct RemoveWorkspace {
    installed: Installed,
    definition: ProjectDefinition,
    binding: Result<ProjectBinding, ProjectsError>,
    location: Option<ContainerLocation>,
    authority: Result<CapturedRemoval, ContainerActionError>,
    choice: Choice,
    pending: bool,
    operation: Option<RemovalOperation>,
    error: Option<WorkspaceRemovalError>,
    dialog_focus: Option<FocusHandle>,
}
impl RemoveWorkspace {
    fn new(
        installed: Installed,
        definition: ProjectDefinition,
        workspace: WeakEntity<Workspace>,
        window: &Window,
        cx: &App,
    ) -> Self {
        let location = ContainerLocation::from_target(definition.target()).ok();
        let authority = location
            .clone()
            .ok_or(ContainerActionError::Unavailable)
            .and_then(|location| {
                let workspace = workspace
                    .upgrade()
                    .ok_or(ContainerActionError::Unavailable)?;
                CapturedRemoval::capture(location, &installed, workspace.read(cx), window, cx)
            });
        let binding = installed.model().binding(definition.id(), cx);
        Self {
            installed,
            definition,
            binding,
            location,
            authority,
            choice: Choice::Keep,
            pending: false,
            operation: None,
            error: None,
            dialog_focus: None,
        }
    }
    fn choose(&mut self, choice: Choice, cx: &mut Context<Self>) {
        if self.pending
            || self
                .operation
                .as_ref()
                .is_some_and(|op| op.outcome().is_some())
            || choice != Choice::Keep && self.authority.is_err()
        {
            return;
        }
        self.choice = choice;
        self.operation = None;
        self.error = None;
        cx.notify();
    }
    fn request(&mut self, cx: &mut App) -> Result<RemovalOperation, WorkspaceRemovalError> {
        if let Some(operation) = self.operation.as_ref().filter(|op| op.outcome().is_some()) {
            self.installed.model().retry_removal(operation, cx)?;
            return Ok(operation.clone());
        }
        let action = self
            .choice
            .action()
            .map(|action| {
                let source = self.authority.as_ref().map_err(Clone::clone)?;
                source.validate(cx)?;
                Ok::<_, ContainerActionError>(RemovalContainerAction::new(
                    self.installed
                        .container_actions()
                        .cloned()
                        .ok_or(ContainerActionError::Unavailable)?,
                    source.source(),
                    action,
                ))
            })
            .transpose()?;
        self.installed.model().remove_workspace(
            self.definition.clone(),
            self.binding.clone()?,
            action,
            cx,
        )
    }
    fn submit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.pending {
            return;
        }
        let operation = match self.request(cx) {
            Ok(operation) => operation,
            Err(error) => {
                self.error = Some(error);
                cx.notify();
                return;
            }
        };
        self.operation = Some(operation.clone());
        self.pending = true;
        self.error = None;
        cx.notify();
        cx.spawn_in(window, async move |this, cx| {
            let result = operation.wait().await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.pending = false;
                match result {
                    Ok(()) => {
                        if this
                            .dialog_focus
                            .as_ref()
                            .is_some_and(|focus| focus.contains_focused(window, cx))
                        {
                            window.close_dialog(cx);
                        }
                    }
                    Err(error) => this.error = Some(error),
                }
                cx.notify();
            });
        })
        .detach();
    }
    fn choices(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let locked = self.pending
            || self
                .operation
                .as_ref()
                .is_some_and(|op| op.outcome().is_some());
        gpui_kit::base::RadioGroup::new("project-remove-choices")
            .flex()
            .flex_col()
            .w_full()
            .gap_2()
            .children(
                [
                    (
                        Choice::Keep,
                        "project-remove-keep",
                        "Keep container",
                        "Leave it running or stopped as it is.",
                    ),
                    (
                        Choice::Stop,
                        "project-remove-stop",
                        "Stop container",
                        "Stop its processes; keep its files.",
                    ),
                    (
                        Choice::Delete,
                        "project-remove-delete",
                        "Delete container",
                        "Stop and remove it; writable-layer files are lost.",
                    ),
                ]
                .into_iter()
                .map(|(choice, id, label, description)| {
                    Radio::new(id)
                        .small()
                        .label(label)
                        .checked(self.choice == choice)
                        .disabled(
                            locked
                                || choice != Choice::Keep
                                    && (self.authority.is_err()
                                        || self.installed.container_actions().is_none()),
                        )
                        .w_full()
                        .child(
                            div()
                                .text_xs()
                                .text_color(cx.theme().muted_foreground)
                                .child(description),
                        )
                        .on_change(cx.listener(move |this, _, _, cx| this.choose(choice, cx)))
                }),
            )
    }
    fn consequences(&self) -> &'static str {
        match self.choice {
            Choice::Keep => {
                "Removes saved workspace settings. Open terminals, project files and chat history are kept."
            }
            Choice::Stop => {
                "Other workspaces using this container will disconnect too. Images, volumes, host folders and chat history are kept."
            }
            Choice::Delete => {
                "Other workspaces using this container will disconnect too. Images, volumes, bind-mounted host folders and chat history are kept."
            }
        }
    }
    fn error_message(&self, error: &WorkspaceRemovalError) -> String {
        let mut message = match self
            .operation
            .as_ref()
            .and_then(RemovalOperation::outcome)
            .map(|outcome| outcome.action)
        {
            Some(ContainerAction::Stop) => "Container stopped. ".to_owned(),
            Some(ContainerAction::Delete) => "Container deleted. ".to_owned(),
            None => String::new(),
        };
        message.push_str(&error.to_string());
        let mut source = std::error::Error::source(error);
        while let Some(cause) = source {
            message.push_str(&format!(". {cause}"));
            source = cause.source();
        }
        message
    }
}
impl gpui_kit::Render for RemoveWorkspace {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let retry = self
            .operation
            .as_ref()
            .is_some_and(|op| op.outcome().is_some());
        v_flex().id("project-remove-dialog").test_support().w_full().min_w_0().gap_3()
            .when_some(self.location.as_ref(), |view, location| {
                view.child(Button::new("project-remove-container").ghost().small().w_full().justify_start()
                    .label(format!("{} · {}", location.engine().name(), &location.container()[..12]))
                    .tooltip(format!("{} · {}", location.engine().name(), location.container())))
                    .child(self.choices(cx))
                    .when(self.authority.is_err() || self.installed.container_actions().is_none(), |view| view.child(div().id("project-remove-unavailable").test_support().text_xs().text_color(cx.theme().muted_foreground).child("Connect to the original server to stop or delete this container.")))
            })
            .child(div().id("project-remove-consequences").test_support().aria_label(self.consequences()).text_sm().child(self.consequences()))
            .when_some(self.error.as_ref(), |view, error| {
                let message = self.error_message(error);
                view.child(div().id("project-remove-error").test_support().role(gpui_kit::Role::Alert).aria_label(message.clone()).text_sm().text_color(cx.theme().danger).max_h(px(72.)).overflow_y_scroll().child(message))
            })
            .child(h_flex().justify_end().gap_2()
                .child(Button::new("cancel").ghost().label("Cancel").disabled(self.pending).on_click(cx.listener(|this, _, window, cx| { if !this.pending { window.close_dialog(cx); } })))
                .child(Button::new("ok").label(if self.pending { "Removing…" } else if retry { "Retry removal" } else { self.choice.button() })
                    .when(self.choice == Choice::Delete && !retry, |button| button.danger())
                    .when(self.choice != Choice::Delete || retry, |button| button.primary())
                    .disabled(self.pending).on_click(cx.listener(|this, _, window, cx| this.submit(window, cx)))))
    }
}
