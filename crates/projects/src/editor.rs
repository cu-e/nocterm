//! Native controls retain the original host and expected saved definition.
mod controls;
mod navigation;
mod options;
mod publication;
use crate::{Installed, ProjectOpenError};
use controls::ProjectControls;
use gpui_kit::{
    App, AppContext as _, Context, FocusHandle, Focusable, Subscription, Window,
    base::TestSupportExt as _,
    component::{
        Disableable as _, Selectable as _, Sizable as _, StyledExt as _, WindowExt as _,
        button::{Button, ButtonVariants as _},
        h_flex, v_flex,
    },
    div,
    prelude::*,
    px,
};
use navigation::{WizardNavigation, WizardStep};
use nocterm_project_model::ProjectDefinition;
use nocterm_workspace_ports::ProjectCreationSource;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

pub(crate) fn open(
    installed: Installed,
    source: ProjectCreationSource,
    window: &mut Window,
    cx: &mut App,
) {
    let identity = match installed.model().new_identity(cx) {
        Ok(identity) => identity,
        Err(error) => {
            nocterm_ui::notice::error(
                window,
                cx,
                "project-create",
                "Could not create workspace",
                error.to_string(),
            );
            return;
        }
    };
    show(
        installed.clone(),
        ProjectControls::create(identity, &source, &installed, window, cx),
        None,
        true,
        window,
        cx,
    );
}
pub(crate) fn edit(
    installed: Installed,
    definition: ProjectDefinition,
    workspace: gpui_kit::WeakEntity<nocterm_workspace::Workspace>,
    window: &mut Window,
    cx: &mut App,
) {
    let environment = cx.new(|cx| {
        crate::environment::EnvironmentControls::new(
            installed.clone(),
            definition.clone(),
            workspace,
            window,
            cx,
        )
    });
    show(
        installed,
        ProjectControls::edit(definition, window, cx),
        Some(environment),
        false,
        window,
        cx,
    );
}
fn show(
    installed: Installed,
    controls: ProjectControls,
    environment: Option<gpui_kit::Entity<crate::environment::EnvironmentControls>>,
    creating: bool,
    window: &mut Window,
    cx: &mut App,
) {
    let editor = cx.new(|cx| ProjectEditor::new(installed, controls, environment, cx));
    let content = editor.clone();
    let closing = editor.downgrade();
    let confirming = editor.downgrade();
    let dismissed = editor.read(cx).dismissed.clone();
    window.open_dialog(cx, move |dialog, _, _| {
        let dismissed = dismissed.clone();
        dialog
            .title(if creating {
                "New workspace"
            } else {
                "Workspace settings"
            })
            .w(px(540.))
            .child(content.clone())
            .on_close({
                let closing = closing.clone();
                move |_, _, cx| {
                    dismissed.store(true, Ordering::Release);
                    let _ = closing.update(cx, |this, cx| this.dismiss(cx));
                }
            })
            .on_ok({
                let confirming = confirming.clone();
                move |_, window, cx| {
                    if creating {
                        let _ = confirming.update(cx, |this, cx| this.advance(window, cx));
                    }
                    false
                }
            })
    });
    let focus = window.focused(cx);
    editor.update(cx, |editor, _| editor.dialog_focus = focus);
    window.focus(&editor.read(cx).focus_handle(cx), cx);
}
pub(crate) struct ProjectEditor {
    installed: Installed,
    controls: ProjectControls,
    environment: Option<gpui_kit::Entity<crate::environment::EnvironmentControls>>,
    navigation: WizardNavigation,
    pending: bool,
    error: Option<ProjectOpenError>,
    dismissed: Arc<AtomicBool>,
    dialog_focus: Option<FocusHandle>,
    setup: Option<nocterm_project_service::WorkspaceSetup>,
    phase: Option<nocterm_project_providers::SetupUpdate>,
    _subscriptions: Vec<Subscription>,
}
impl ProjectEditor {
    fn new(
        installed: Installed,
        controls: ProjectControls,
        environment: Option<gpui_kit::Entity<crate::environment::EnvironmentControls>>,
        cx: &mut Context<Self>,
    ) -> Self {
        let editor = cx.weak_entity();
        let mut subscriptions = controls.observe(cx, move |cx| {
            let _ = editor.update(cx, |this, cx| this.clear_error(cx));
        });
        if let Some(environment) = &environment {
            subscriptions.push(cx.observe(environment, |_, _, cx| cx.notify()));
        }
        subscriptions.push(cx.on_release(|this, cx| this.dismiss(cx)));
        Self {
            installed,
            controls,
            environment,
            navigation: WizardNavigation::new(cx),
            pending: false,
            error: None,
            dismissed: Arc::new(AtomicBool::new(false)),
            dialog_focus: None,
            setup: None,
            phase: None,
            _subscriptions: subscriptions,
        }
    }
    fn clear_error(&mut self, cx: &mut Context<Self>) {
        self.error = None;
        self.navigation.branch(self.controls.new_container(cx));
        self.controls.show_step(self.navigation.current(), cx);
        cx.notify();
    }
    fn navigate(&mut self, step: Option<WizardStep>, window: &mut Window, cx: &mut Context<Self>) {
        if self.pending || self.setup.is_some() {
            return;
        }
        match step {
            Some(step) => self.navigation.visit(step),
            None => self.navigation.back(),
        }
        self.present_step(window, cx);
    }
    fn present_step(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.controls.show_step(self.navigation.current(), cx);
        let focus = if self.navigation.current() == WizardStep::Connection {
            self.navigation.focus()
        } else {
            self.controls.step_focus(self.navigation.current(), cx)
        };
        window.focus(&focus, cx);
        self.error = None;
        cx.notify();
    }
    fn validate_step(
        &mut self,
        step: WizardStep,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        if let Err((error, focus)) = self.controls.validate(step, cx) {
            self.navigation.visit(step);
            self.controls.show_step(step, cx);
            window.focus(
                &if step == WizardStep::Connection && !self.controls.new_container(cx) {
                    self.navigation.focus()
                } else {
                    focus
                },
                cx,
            );
            self.error = Some(error);
            cx.notify();
            false
        } else {
            true
        }
    }
    fn advance(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.navigation.branch(self.controls.new_container(cx));
        if self.pending {
            return;
        }
        if self.controls.original().is_some() || self.navigation.final_step() {
            self.save(window, cx);
        } else if self.validate_step(self.navigation.current(), window, cx) {
            self.navigation.advance();
            self.present_step(window, cx);
        }
    }
    fn save(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.pending || self.dismissed.load(Ordering::Acquire) {
            return;
        }
        if self.controls.original().is_none() {
            if !self.navigation.final_step() {
                return;
            }
            for step in self.navigation.steps().to_vec() {
                if !self.validate_step(step, window, cx) {
                    return;
                }
            }
        }
        let request = match self.publication(cx) {
            Ok(request) => request,
            Err(error) => {
                self.error = Some(error);
                cx.notify();
                return;
            }
        };
        self.controls.lock(true, cx);
        self.pending = true;
        self.error = None;
        cx.notify();
        cx.spawn_in(window, async move |this, cx| {
            let result = request.run(&this, cx).await;
            let _ = this.update_in(cx, |this, window, cx| this.complete(result, window, cx));
        })
        .detach();
    }
    fn publication(
        &mut self,
        cx: &mut App,
    ) -> Result<publication::ProjectPublication, ProjectOpenError> {
        let request = self.controls.setup_request(cx)?;
        let target = request.as_ref().and_then(|request| {
            Some(nocterm_project_model::ProjectTarget::HostFolder {
                host: request.host.clone(),
                root: match &request.mode {
                    nocterm_project_providers::SetupMode::Direct { root } => Some(root.clone()),
                    _ => nocterm_project_model::RootPath::parse("/").ok(),
                }?,
            })
        });
        let candidate =
            self.controls
                .candidate(self.installed.model().timestamp(cx)?, target, cx)?;
        if self.setup.is_none() {
            self.setup = self.admit_setup(request, cx)?;
        }
        Ok(publication::ProjectPublication::new(
            candidate,
            self.controls.original().cloned(),
            self.setup.clone(),
            self.controls.directory_check(cx),
            self.dismissed.clone(),
        ))
    }
    fn admit_setup(
        &self,
        request: Option<nocterm_project_providers::ProjectSetupRequest>,
        cx: &mut App,
    ) -> Result<Option<nocterm_project_service::WorkspaceSetup>, ProjectOpenError> {
        let Some(request) = request else {
            return Ok(None);
        };
        if let (Some(preparation), Some(exec)) = (
            self.installed.preparation(),
            self.controls.source().and_then(|source| source.executor()),
        ) {
            return Ok(Some(self.installed.model().prepare_workspace(
                preparation.clone(),
                exec,
                request,
                cx,
            )?));
        }
        if !matches!(
            request.mode,
            nocterm_project_providers::SetupMode::Direct { .. }
        ) || request.repository.is_some()
        {
            return Err(nocterm_project_providers::ProjectSetupError::InvalidInput(
                "the original server has no command execution capability",
            )
            .into());
        }
        Ok(None)
    }
    fn complete(
        &mut self,
        result: Result<(), ProjectOpenError>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.pending = false;
        self.phase = None;
        if self.dismissed.load(Ordering::Acquire) {
            return;
        }
        match result {
            Ok(()) => {
                if self
                    .dialog_focus
                    .as_ref()
                    .is_some_and(|focus| focus.contains_focused(window, cx))
                {
                    self.dismissed.store(true, Ordering::Release);
                    window.close_dialog(cx);
                }
            }
            Err(error) => {
                if !self.setup.as_ref().is_some_and(|setup| setup.is_prepared()) {
                    self.setup = None;
                    self.controls.lock(false, cx);
                }
                self.error = Some(error);
            }
        }
        cx.notify();
    }
    fn dismiss(&mut self, cx: &mut App) {
        self.dismissed.store(true, Ordering::Release);
        if let Some(environment) = &self.environment {
            environment.update(cx, |environment, _| environment.cancel());
        }
        if let Some(operation) = self.setup.clone() {
            cx.background_executor()
                .spawn(async move {
                    if let Err(error) = operation.discard().await {
                        tracing::warn!(?error, "workspace cleanup failed");
                    }
                })
                .detach();
        }
    }
    fn cancel(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.dismiss(cx);
        window.close_dialog(cx);
    }
}
impl Focusable for ProjectEditor {
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        if self.controls.original().is_none() && self.navigation.current() == WizardStep::Connection
        {
            self.navigation.focus()
        } else {
            self.controls.focus(cx)
        }
    }
}
impl ProjectEditor {
    fn steps(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        h_flex()
            .id("project-wizard-steps")
            .test_support()
            .gap_2()
            .children(
                self.navigation
                    .steps()
                    .iter()
                    .enumerate()
                    .map(|(index, step)| {
                        let step = *step;
                        Button::new(step.id())
                            .ghost()
                            .small()
                            .label(format!("{}. {}", index + 1, step.label()))
                            .selected(self.navigation.current() == step)
                            .accessibility_label(format!(
                                "Step {} of {}: {}{}",
                                index + 1,
                                self.navigation.steps().len(),
                                step.label(),
                                if self.navigation.current() == step {
                                    ", current step"
                                } else {
                                    ""
                                }
                            ))
                            .disabled(
                                self.pending
                                    || self.setup.is_some()
                                    || !self.navigation.can_visit(step),
                            )
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.navigate(Some(step), window, cx)
                            }))
                    }),
            )
    }
    fn footer(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let creating = self.controls.original().is_none();
        let final_step = !creating || self.navigation.final_step();
        h_flex()
            .id("project-form-footer")
            .test_support()
            .items_center()
            .gap_2()
            .child(crate::progress::render(
                "project-progress",
                self.phase.as_ref(),
                self.pending,
                cx,
            ))
            .child(
                Button::new("project-cancel")
                    .ghost()
                    .label("Cancel")
                    .on_click(cx.listener(|this, _, window, cx| this.cancel(window, cx))),
            )
            .when(
                creating && self.navigation.current() != WizardStep::Connection,
                |view| {
                    view.child(
                        Button::new("project-back")
                            .ghost()
                            .label("Back")
                            .disabled(self.pending || self.setup.is_some())
                            .on_click(
                                cx.listener(|this, _, window, cx| this.navigate(None, window, cx)),
                            ),
                    )
                },
            )
            .child(
                Button::new(if final_step {
                    "project-save"
                } else {
                    "project-next"
                })
                .primary()
                .label(if !final_step {
                    "Next"
                } else if creating {
                    "Create workspace"
                } else {
                    "Save workspace"
                })
                .disabled(self.pending)
                .on_click(cx.listener(|this, _, window, cx| this.advance(window, cx))),
            )
    }
}
impl Render for ProjectEditor {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.navigation.branch(self.controls.new_container(cx));
        let creating = self.controls.original().is_none();
        let body_height = if creating {
            (window.viewport_size().height - px(300.))
                .max(px(96.))
                .min(px(420.))
        } else {
            px(440.)
        };
        v_flex()
            .gap_3()
            .child(self.controls.server_header(cx))
            .when(creating, |view| {
                view.child(self.steps(cx)).child(
                    div()
                        .id("project-step-heading")
                        .test_support()
                        .track_focus(&self.navigation.focus())
                        .role(gpui_kit::Role::Heading)
                        .aria_label(self.navigation.current().label())
                        .text_sm()
                        .font_medium()
                        .child(self.navigation.current().label()),
                )
            })
            .child(
                div()
                    .id("project-form-body")
                    .test_support()
                    .w_full()
                    .min_w_0()
                    .max_h(body_height)
                    .overflow_y_scroll()
                    .child(
                        v_flex()
                            .gap_4()
                            .child(self.controls.render(
                                self.pending,
                                self.navigation.current(),
                                cx,
                            ))
                            .when_some(self.environment.clone(), |view, environment| {
                                view.child(environment)
                            }),
                    ),
            )
            .when_some(self.error.as_ref(), |form, error| {
                form.child(
                    div()
                        .id("project-form-error")
                        .test_support()
                        .role(gpui_kit::Role::Alert)
                        .aria_label(error.to_string())
                        .child(nocterm_ui::form::error_text(error.to_string(), cx)),
                )
            })
            .when_some(self.environment.clone(), |view, environment| {
                view.child(environment.read(cx).footer(environment.downgrade(), cx))
            })
            .child(self.footer(cx))
    }
}
