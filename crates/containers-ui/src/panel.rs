//! The sidebar panel: the active host's containers, grouped by Compose
//! project, and its images.

mod rows;

use std::collections::HashSet;

use gpui_kit::{
    App, ClickEvent, Context, Entity, FocusHandle, Focusable, SharedString, Subscription,
    WeakEntity, Window,
    component::{
        ActiveTheme as _, Icon, Sizable as _, WindowExt as _,
        button::{Button, ButtonVariants as _},
        h_flex, v_flex,
    },
    div,
    prelude::*,
};
use nocterm_containers::{Action, ContainersError};
use nocterm_ui::IconName;
use nocterm_workspace::{Panel, Workspace};

use crate::{
    model::{ContainersModel, Status},
    ops::Op,
};

/// What a [`Subject`] is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Kind {
    Container,
    Project,
    Image,
}

impl Kind {
    fn noun(self) -> &'static str {
        match self {
            Self::Container => "container",
            Self::Project => "project",
            Self::Image => "image",
        }
    }
}

/// What an [`Op`] is done to.
#[derive(Clone, Debug)]
pub(crate) struct Subject {
    kind: Kind,
    name: String,
    /// The containers or image it stands for; a project stands for all its
    /// containers.
    ids: Vec<String>,
}

/// A part of the tree the user folded.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
enum Fold {
    Containers,
    Images,
    Project(String),
}

pub struct ContainersPanel {
    model: Entity<ContainersModel>,
    workspace: WeakEntity<Workspace>,
    focus_handle: FocusHandle,
    folded: HashSet<Fold>,
    _observe: Subscription,
}

impl ContainersPanel {
    pub(crate) fn new(
        model: Entity<ContainersModel>,
        workspace: WeakEntity<Workspace>,
        cx: &mut Context<Self>,
    ) -> Self {
        Self {
            _observe: cx.observe(&model, |_, _, cx| cx.notify()),
            model,
            workspace,
            focus_handle: cx.focus_handle(),
            // Images are many and rarely the point.
            folded: HashSet::from([Fold::Images]),
        }
    }

    fn toggle(&mut self, fold: Fold, cx: &mut Context<Self>) {
        if !self.folded.remove(&fold) {
            self.folded.insert(fold);
        }
        cx.notify();
    }

    /// Does `op` to `subject`, asking first when it destroys something.
    pub(crate) fn run(
        &mut self,
        op: Op,
        subject: Subject,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match op {
            Op::Logs => self.open(
                format!("Logs: {}", subject.name),
                subject,
                window,
                cx,
                |engine, id| engine.logs(id),
            ),
            Op::Shell => self.open(
                format!("Shell: {}", subject.name),
                subject,
                window,
                cx,
                |engine, id| engine.shell(id),
            ),
            Op::Act(action) if op.destructive() => self.confirm(action, subject, window, cx),
            Op::Act(action) => self.perform(action, subject.ids, window, cx),
        }
    }

    /// Opens a tab running a program about the subject's container.
    fn open(
        &mut self,
        title: String,
        subject: Subject,
        window: &mut Window,
        cx: &mut Context<Self>,
        program: impl FnOnce(nocterm_containers::Engine, &str) -> nocterm_session::ExecRequest,
    ) {
        let Some(id) = subject.ids.first() else {
            return;
        };
        let Some(spec) = self
            .model
            .read(cx)
            .program(title, |engine| program(engine, id))
        else {
            return;
        };
        let _ = self
            .workspace
            .update(cx, |workspace, cx| workspace.open_program(spec, window, cx));
    }

    fn confirm(
        &mut self,
        action: Action,
        subject: Subject,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let panel = cx.entity().downgrade();
        let Subject { kind, name, ids } = subject;
        let description = match kind {
            Kind::Container => "It is stopped and deleted. Its volumes are kept.",
            Kind::Project => "Its containers are stopped and deleted. Their volumes are kept.",
            Kind::Image => "Containers made from it must be removed first.",
        };
        let noun = kind.noun();
        window.open_alert_dialog(cx, move |alert, _, _| {
            let panel = panel.clone();
            let ids = ids.clone();
            alert
                .title(format!("Remove {noun} “{name}”?"))
                .description(description)
                .ok_text("Remove")
                .show_cancel(true)
                .on_ok(move |_, window, cx| {
                    let ids = ids.clone();
                    let _ = panel.update(cx, |this, cx| this.perform(action, ids, window, cx));
                    true
                })
        });
    }

    fn perform(
        &mut self,
        action: Action,
        ids: Vec<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let done = self
            .model
            .update(cx, |model, cx| model.perform(action, ids, cx));
        window
            .spawn(cx, async move |cx| {
                if let Err(error) = done.await {
                    let _ = cx.update(|window, cx| {
                        nocterm_ui::notice::error(
                            window,
                            cx,
                            "containers-action",
                            "Container action failed",
                            format!("{}: {error}", action.label()),
                        )
                    });
                }
            })
            .detach();
    }

    fn render_toolbar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let model = self.model.read(cx);
        let host = model
            .host()
            .map_or_else(|| "No host".to_owned(), |host| host.key.label());
        let engine = model.engine().map(|engine| engine.name());
        let theme = cx.theme();
        h_flex()
            .gap_1()
            .px_3()
            .pb_1()
            .text_xs()
            .text_color(theme.muted_foreground)
            .child(Icon::new(IconName::Server).xsmall())
            .child(div().flex_1().min_w_0().truncate().child(host))
            .when_some(engine, |bar, engine| bar.child(engine))
            .child(
                Button::new("containers-refresh")
                    .ghost()
                    .xsmall()
                    .icon(IconName::RefreshCw)
                    .tooltip("Refresh")
                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                        this.model.update(cx, |model, cx| model.refresh(cx));
                    })),
            )
    }

    /// A line on why there is nothing, or nothing current, to show.
    fn render_status(&self, cx: &mut Context<Self>) -> Option<impl IntoElement> {
        let model = self.model.read(cx);
        let found = model.engine().is_some();
        let (text, danger) = match model.status() {
            Status::Ready => return None,
            Status::Loading if found => return None,
            Status::Loading => ("Looking for Docker or Podman…".to_owned(), false),
            Status::Off => ("No host to look at.".to_owned(), false),
            Status::Offline => ("The session is not connected.".to_owned(), false),
            Status::Failed(
                error @ (ContainersError::NotInstalled | ContainersError::Unsupported),
            ) => (error.to_string(), false),
            Status::Failed(error) => (error.to_string(), true),
        };
        let theme = cx.theme();
        Some(
            div()
                .id("containers-status")
                .mx_2()
                .mb_1()
                .px_2()
                .py_1()
                .text_xs()
                .text_color(if danger {
                    theme.danger
                } else {
                    theme.muted_foreground
                })
                .child(text),
        )
    }
}

impl Panel for ContainersPanel {
    fn title(&self, _: &App) -> SharedString {
        "Containers".into()
    }

    fn icon(&self, _: &App) -> IconName {
        IconName::Container
    }

    /// How many containers run; none shows no count.
    fn badge(&self, cx: &App) -> Option<SharedString> {
        let running = self.model.read(cx).snapshot().running();
        (running > 0).then(|| running.to_string().into())
    }
}

impl Focusable for ContainersPanel {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for ContainersPanel {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let shown = self.model.read(cx).engine().is_some();
        v_flex()
            .track_focus(&self.focus_handle)
            .size_full()
            .child(self.render_toolbar(cx))
            .children(self.render_status(cx))
            .when(shown, |panel| {
                panel.child(
                    div()
                        .id("containers-list")
                        .flex_1()
                        .min_h_0()
                        .overflow_y_scroll()
                        .child(self.render_tree(cx)),
                )
            })
    }
}
