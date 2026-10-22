//! Native controls retain the original host and expected saved definition.
mod controls;
use crate::{Installed, ProjectOpenError};
use controls::ProjectControls;
use gpui_kit::{
    App, AppContext as _, Context, FocusHandle, Focusable, Subscription, Window,
    base::TestSupportExt as _,
    component::{
        Disableable as _, WindowExt as _,
        button::{Button, ButtonVariants as _},
        h_flex, v_flex,
    },
    div,
    prelude::*,
    px,
};
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
        installed,
        ProjectControls::create(identity, &source, window, cx),
        true,
        window,
        cx,
    );
}
pub(crate) fn edit(
    installed: Installed,
    definition: ProjectDefinition,
    window: &mut Window,
    cx: &mut App,
) {
    show(
        installed,
        ProjectControls::edit(definition, window, cx),
        false,
        window,
        cx,
    );
}
fn show(
    installed: Installed,
    controls: ProjectControls,
    creating: bool,
    window: &mut Window,
    cx: &mut App,
) {
    let editor = cx.new(|cx| ProjectEditor::new(installed, controls, cx));
    let content = editor.clone();
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
            .on_close(move |_, _, _| dismissed.store(true, Ordering::Release))
            .on_ok(|_, _, _| false)
    });
    let focus = window.focused(cx);
    editor.update(cx, |editor, _| editor.dialog_focus = focus);
    window.focus(&editor.read(cx).focus_handle(cx), cx);
}
pub(crate) struct ProjectEditor {
    installed: Installed,
    controls: ProjectControls,
    pending: bool,
    error: Option<ProjectOpenError>,
    dismissed: Arc<AtomicBool>,
    dialog_focus: Option<FocusHandle>,
    _subscriptions: Vec<Subscription>,
}
impl ProjectEditor {
    fn new(installed: Installed, controls: ProjectControls, cx: &mut Context<Self>) -> Self {
        let editor = cx.weak_entity();
        let subscriptions = controls.observe(cx, move |cx| {
            let _ = editor.update(cx, |this, cx| this.clear_error(cx));
        });
        Self {
            installed,
            controls,
            pending: false,
            error: None,
            dismissed: Arc::new(AtomicBool::new(false)),
            dialog_focus: None,
            _subscriptions: subscriptions,
        }
    }
    fn clear_error(&mut self, cx: &mut Context<Self>) {
        self.error = None;
        cx.notify();
    }
    fn candidate(&self, cx: &App) -> Result<ProjectDefinition, ProjectOpenError> {
        Ok(self
            .controls
            .candidate(self.installed.model().timestamp(cx)?, cx)?)
    }
    fn save(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.pending || self.dismissed.load(Ordering::Acquire) {
            return;
        }
        let candidate = match self.candidate(cx) {
            Ok(candidate) => candidate,
            Err(error) => {
                self.error = Some(error);
                cx.notify();
                return;
            }
        };
        let saved = self
            .installed
            .model()
            .save(candidate, self.controls.original().cloned(), cx);
        self.pending = true;
        self.error = None;
        cx.notify();
        cx.spawn_in(window, async move |this, cx| {
            let result = saved.await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.pending = false;
                if this.dismissed.load(Ordering::Acquire) {
                    return;
                }
                match result {
                    Ok(_) => {
                        if this
                            .dialog_focus
                            .as_ref()
                            .is_some_and(|focus| focus.contains_focused(window, cx))
                        {
                            this.dismissed.store(true, Ordering::Release);
                            window.close_dialog(cx);
                        }
                    }
                    Err(error) => this.error = Some(error.into()),
                }
                cx.notify();
            });
        })
        .detach();
    }
}
impl Focusable for ProjectEditor {
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        self.controls.focus(cx)
    }
}
impl Render for ProjectEditor {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        v_flex()
            .gap_3()
            .child(self.controls.render(self.pending, cx))
            .when_some(self.error.as_ref(), |form, error| {
                form.child(
                    div()
                        .id("project-form-error")
                        .test_support()
                        .child(nocterm_ui::form::error_text(error.to_string(), cx)),
                )
            })
            .child(
                h_flex()
                    .justify_end()
                    .gap_2()
                    .child(
                        Button::new("project-cancel")
                            .ghost()
                            .label("Cancel")
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.dismissed.store(true, Ordering::Release);
                                window.close_dialog(cx);
                            })),
                    )
                    .child(
                        Button::new("project-save")
                            .primary()
                            .label(if self.pending {
                                "Saving…"
                            } else {
                                "Save workspace"
                            })
                            .disabled(self.pending)
                            .on_click(cx.listener(|this, _, window, cx| this.save(window, cx))),
                    ),
            )
    }
}
