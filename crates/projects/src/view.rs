//! One outer Item owns presentation of its original inner dock.
mod lifetime;
pub(crate) use lifetime::ProjectLifetime;

use crate::{Installed, ProjectOpenError};
use gpui_kit::{
    AnyWindowHandle, App, Context, EntityId, EventEmitter, FocusHandle, Focusable, SharedString,
    Subscription, WeakEntity, Window,
    base::TestSupportExt as _,
    component::{
        ActiveTheme as _, Disableable as _, Icon, Sizable as _,
        button::{Button, ButtonVariants as _},
        h_flex, v_flex,
    },
    div, img,
    prelude::*,
};
use nocterm_project_model::{HostLocator, ProjectDefinition, ProjectId};
use nocterm_project_service::{ProjectBinding, ProjectClaim};
use nocterm_ui::IconName;
use nocterm_workspace::{
    ContextSelectionReceipt, Item, ItemEvent, LocalTerminalTarget, ProjectSurface,
    ProjectSurfaceEvent, TabState, Workspace,
};
use nocterm_workspace_ports::ProjectParentDescriptor;

pub(crate) struct ProjectPresentation {
    definition: ProjectDefinition,
    parent: ProjectParentDescriptor,
    parent_item: Option<EntityId>,
}
impl ProjectPresentation {
    pub(crate) fn new(
        definition: ProjectDefinition,
        parent: ProjectParentDescriptor,
        parent_item: Option<EntityId>,
    ) -> Self {
        Self {
            definition,
            parent,
            parent_item,
        }
    }
    fn caption(
        &self,
        icon: Option<std::sync::Arc<gpui_kit::Image>>,
        cx: &App,
    ) -> impl IntoElement + use<> {
        let theme = cx.theme();
        h_flex()
            .gap_2()
            .flex_1()
            .min_w_0()
            .child(
                div()
                    .id("project-server-icon")
                    .test_support()
                    .flex_shrink_0()
                    .child(match icon {
                        Some(icon) => img(icon).size_4().flex_shrink_0().into_any_element(),
                        None => Icon::new(IconName::Server)
                            .small()
                            .text_color(theme.muted_foreground)
                            .into_any_element(),
                    }),
            )
            .child(
                div()
                    .id("project-server")
                    .test_support()
                    .aria_label(self.parent.title().clone())
                    .text_xs()
                    .max_w_48()
                    .truncate()
                    .child(self.parent.title().clone()),
            )
            .child(
                div()
                    .id("project-path")
                    .test_support()
                    .aria_label(self.definition.root().as_str().to_owned())
                    .text_xs()
                    .text_color(theme.muted_foreground)
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .child(self.definition.root().as_str().to_owned()),
            )
    }
}

pub struct ProjectView {
    installed: Installed,
    presentation: ProjectPresentation,
    lifetime: ProjectLifetime,
    surface: ProjectSurface,
    workspace: WeakEntity<Workspace>,
    _subscriptions: Vec<Subscription>,
}
impl ProjectView {
    pub(crate) fn new(
        installed: Installed,
        presentation: ProjectPresentation,
        lifetime: ProjectLifetime,
        surface: ProjectSurface,
        workspace: WeakEntity<Workspace>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let subscriptions = Self::subscribe(&installed, &surface, &workspace, window, cx);
        Self {
            installed,
            presentation,
            lifetime,
            surface,
            workspace,
            _subscriptions: subscriptions,
        }
    }
    fn subscribe(
        installed: &Installed,
        surface: &ProjectSurface,
        workspace: &WeakEntity<Workspace>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Vec<Subscription> {
        let owner = cx.weak_entity();
        let native_window = window.window_handle();
        let refresh = owner.clone();
        let mut subscriptions = vec![installed.observe(cx, move |cx| {
            Self::schedule_refresh(refresh.clone(), native_window, cx);
        })];
        if let Some(workspace) = workspace.upgrade() {
            let refresh = owner.clone();
            subscriptions.push(cx.subscribe(&workspace, move |_, _, event, cx| {
                if matches!(event, nocterm_workspace::WorkspaceEvent::ConnectionsChanged) {
                    Self::schedule_refresh(refresh.clone(), native_window, cx);
                }
            }));
        }
        let terminal = owner.clone();
        subscriptions.push(surface.observe(cx, move |event, cx| {
            if let ProjectSurfaceEvent::OpenTerminal {
                selection,
                destination,
            } = event
            {
                Self::schedule_terminal(
                    terminal.clone(),
                    native_window,
                    selection,
                    destination,
                    cx,
                );
            }
        }));
        subscriptions.push(installed.model().observe_focus(cx, move |claim, cx| {
            Self::schedule_focus(owner.clone(), native_window, claim, cx);
        }));
        subscriptions.push(cx.on_release(|this, cx| this.lifetime.close(&this.installed, cx)));
        subscriptions
    }
    fn schedule_refresh(owner: WeakEntity<Self>, window: AnyWindowHandle, cx: &mut App) {
        cx.defer(move |cx| {
            let _ = window.update(cx, |_, window, cx| {
                let _ = owner.update(cx, |view, cx| view.refresh_parent(window, cx));
            });
        });
    }
    fn refresh_parent(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let parent = self
            .installed
            .catalog(cx)
            .get(self.project())
            .and_then(|definition| {
                let workspace = self.workspace.upgrade()?;
                crate::context::server(
                    workspace.read(cx),
                    definition.host(),
                    Some(&self.presentation.parent),
                    cx,
                )
                .ok()
            });
        if let Some(parent) = parent {
            self.presentation.parent = parent;
        }
        self.refresh(window, cx);
    }
    fn schedule_terminal(
        owner: WeakEntity<Self>,
        window: AnyWindowHandle,
        selection: ContextSelectionReceipt,
        destination: Option<LocalTerminalTarget>,
        cx: &mut App,
    ) {
        let project = owner
            .upgrade()
            .ok_or(ProjectOpenError::Surface(
                nocterm_workspace::ContextSelectionError::StaleSurface,
            ))
            .and_then(|view| {
                let view = view.read(cx);
                Ok((
                    view.installed.model().binding(view.project(), cx)?,
                    view.presentation.definition.host().clone(),
                    view.lifetime.session()?,
                ))
            });
        cx.defer(move |cx| {
            let _ = window.update(cx, |_, window, cx| {
                let _ = owner.update(cx, |view, cx| {
                    let result = project.and_then(|(binding, host, project)| {
                        view.installed.model().check_binding(binding, cx)?;
                        if view.presentation.definition.host() != &host {
                            return Err(
                                nocterm_workspace_ports::ProjectParentError::WrongHost.into()
                            );
                        }
                        view.surface
                            .terminal_target(selection, destination, window, cx)
                            .map_err(ProjectOpenError::from)
                            .and_then(|target| view.open_with(project, target, window, cx))
                    });
                    view.report_terminal_result(result, cx);
                });
            });
        });
    }
    fn report_terminal_result(
        &mut self,
        result: Result<(), ProjectOpenError>,
        cx: &mut Context<Self>,
    ) {
        if let Err(error) = result {
            self.lifetime.failed(error);
            cx.notify();
        }
    }
    fn retry(&self, window: AnyWindowHandle, cx: &mut Context<Self>) {
        let request = RetryRequest {
            claim: self.lifetime.claim(),
            binding: self.installed.model().binding(self.project(), cx),
            host: self.presentation.definition.host().clone(),
        };
        let owner = cx.weak_entity();
        cx.defer(move |cx| {
            let _ = window.update(cx, |_, window, cx| {
                let _ = owner.update(cx, |view, cx| view.resume(request, window, cx));
            });
        });
    }
    fn resume(&mut self, request: RetryRequest, window: &mut Window, cx: &mut Context<Self>) {
        if !self.lifetime.can_retry() || request.claim != self.lifetime.claim() {
            return;
        }
        let session = request
            .binding
            .map_err(ProjectOpenError::from)
            .and_then(|binding| {
                let workspace = self
                    .workspace
                    .upgrade()
                    .ok_or(nocterm_workspace_ports::ProjectParentError::Disconnected)?;
                crate::opening::capture_session(
                    &self.installed,
                    binding,
                    request.claim,
                    &request.host,
                    workspace.read(cx),
                    window,
                    cx,
                )
            });
        self.lifetime.retry(session);
        self.refresh(window, cx);
    }
    fn schedule_focus(
        owner: WeakEntity<Self>,
        window: AnyWindowHandle,
        claim: ProjectClaim,
        cx: &mut App,
    ) {
        cx.defer(move |cx| {
            let _ = window.update(cx, |_, window, cx| {
                let workspace = owner
                    .upgrade()
                    .and_then(|view| view.read(cx).focus_destination(claim, cx));
                if let Some(workspace) = workspace {
                    let focused = workspace.update(cx, |workspace, cx| {
                        workspace.activate_item_by_id(owner.entity_id(), window, cx)
                    });
                    if focused {
                        window.activate_window();
                    }
                }
            });
        });
    }
    fn focus_destination(
        &self,
        claim: ProjectClaim,
        cx: &App,
    ) -> Option<gpui_kit::Entity<Workspace>> {
        (claim == self.lifetime.claim() && self.installed.model().check_claim(claim, cx).is_ok())
            .then(|| self.workspace.upgrade())
            .flatten()
    }
    pub fn project(&self) -> ProjectId {
        self.presentation.definition.id()
    }
    pub fn parent(&self) -> &ProjectParentDescriptor {
        &self.presentation.parent
    }
    pub(crate) fn refresh(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(definition) = self.installed.catalog(cx).get(self.project()) {
            if definition.host() != self.presentation.definition.host() {
                self.presentation.parent_item = None;
                self.lifetime.retry(Err(
                    nocterm_workspace_ports::ProjectParentError::WrongHost.into()
                ));
            }
            self.presentation.definition = definition.clone();
        }
        self.surface
            .set_root(self.presentation.definition.root().clone(), cx);
        if self.lifetime.start_first() {
            let result = self
                .surface
                .initial_terminal_target(window, cx)
                .map_err(ProjectOpenError::from)
                .and_then(|target| self.open_terminal(target, window, cx));
            if let Err(error) = result {
                self.lifetime.failed(error);
            }
        }
        cx.emit(ItemEvent::Changed);
        cx.notify();
    }
    fn open_terminal(
        &self,
        target: nocterm_workspace::ProjectTerminalTarget,
        window: &mut Window,
        cx: &mut App,
    ) -> Result<(), ProjectOpenError> {
        let project = self.lifetime.session()?;
        self.open_with(project, target, window, cx)
    }
    fn open_with(
        &self,
        session: std::rc::Rc<dyn nocterm_workspace::ProjectTerminalSession>,
        target: nocterm_workspace::ProjectTerminalTarget,
        window: &mut Window,
        cx: &mut App,
    ) -> Result<(), ProjectOpenError> {
        let definition = self
            .installed
            .catalog(cx)
            .get(self.project())
            .ok_or(ProjectOpenError::Removed)?;
        if definition.host() != self.presentation.definition.host() {
            return Err(nocterm_workspace_ports::ProjectParentError::WrongHost.into());
        }
        let root = definition.root().clone();
        session.open(target, &root, window, cx)?;
        Ok(())
    }
    fn new_terminal(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let project = self.lifetime.session();
        let selection = self.surface.selection_receipt(cx);
        let result = project.and_then(|project| {
            self.surface
                .terminal_target(selection, None, window, cx)
                .map_err(ProjectOpenError::from)
                .and_then(|target| self.open_with(project, target, window, cx))
        });
        if let Err(error) = result {
            self.lifetime.failed(error);
            cx.notify();
        }
    }
    fn header(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let theme = cx.theme().clone();
        let parent_open = self.presentation.parent_item.is_some_and(|id| {
            self.workspace.upgrade().is_some_and(|workspace| {
                let workspace = workspace.read(cx);
                workspace.tab_location(id, cx).is_some() || workspace.is_background(id, cx)
            })
        });
        let commands = self.lifetime.session().is_ok();
        let icon = self.workspace.upgrade().and_then(|workspace| {
            crate::context::summary(workspace.read(cx), self.presentation.definition.host(), cx)
                .and_then(|connection| connection.icon)
        });
        h_flex()
            .px_3()
            .py_1()
            .gap_2()
            .border_b_1()
            .border_color(theme.border)
            .child(self.presentation.caption(icon, cx))
            .child(
                Button::new("project-terminal-new")
                    .ghost()
                    .xsmall()
                    .icon(IconName::SquareTerminal)
                    .label("New terminal")
                    .disabled(!commands)
                    .tooltip(if commands {
                        "Open a terminal inside this workspace"
                    } else {
                        "Terminal commands are unavailable in this workspace"
                    })
                    .on_click(cx.listener(|this, _, window, cx| this.new_terminal(window, cx))),
            )
            .when(parent_open, |header| {
                header.child(
                    Button::new("project-leave")
                        .ghost()
                        .xsmall()
                        .label("Leave workspace")
                        .tooltip("Return to the server; keep this workspace open")
                        .on_click(cx.listener(|this, _, window, cx| this.leave(window, cx))),
                )
            })
    }
    fn leave(&self, window: &mut Window, cx: &mut Context<Self>) {
        if let (Some(workspace), Some(parent)) =
            (self.workspace.upgrade(), self.presentation.parent_item)
        {
            workspace.update(cx, |workspace, cx| {
                workspace.activate_item_by_id(parent, window, cx);
            });
        }
    }
}
impl EventEmitter<ItemEvent> for ProjectView {}
impl Focusable for ProjectView {
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        self.surface.focus_handle(cx)
    }
}
impl Item for ProjectView {
    fn tab_title(&self, _: &App) -> SharedString {
        self.presentation.definition.name().to_owned().into()
    }
    fn tab_icon(&self, _: &App) -> IconName {
        IconName::FolderTree
    }
    fn tab_state(&self, _: &App) -> TabState {
        if self.lifetime.message().is_some() {
            TabState::Attention
        } else {
            TabState::Idle
        }
    }
    fn on_close(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.surface.close(window, cx);
        self.lifetime.close(&self.installed, cx);
    }
}
impl Render for ProjectView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme().clone();
        let message = self.lifetime.message();
        v_flex()
            .size_full()
            .min_h_0()
            .child(self.header(cx))
            .when_some(message, |view, message| {
                view.child(
                    div()
                        .id("project-unavailable")
                        .test_support()
                        .aria_label(message.clone())
                        .px_3()
                        .py_2()
                        .text_sm()
                        .text_color(theme.muted_foreground)
                        .child(message),
                )
            })
            .when(self.lifetime.can_retry(), |view| {
                view.child(
                    h_flex().px_3().py_1().gap_2().child(
                        Button::new("project-retry")
                            .ghost()
                            .small()
                            .label("Retry")
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.retry(window.window_handle(), cx)
                            })),
                    ),
                )
            })
            .child(div().flex_1().min_h_0().child(self.surface.view()))
    }
}

struct RetryRequest {
    claim: ProjectClaim,
    binding: Result<ProjectBinding, nocterm_project_service::ProjectsError>,
    host: HostLocator,
}
