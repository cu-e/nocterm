//! Presentation of the retained transfer service. Available outside Explorer.
use crate::{ShowTransfers, local::bytes};
use gpui_kit::{
    App, Context, EventEmitter, FocusHandle, Focusable, Global, SharedString, Task, WeakEntity,
    Window,
    component::{
        ActiveTheme as _, Disableable as _, Icon, Sizable as _,
        button::{Button, ButtonVariants as _},
        h_flex, v_flex,
    },
    div,
    prelude::*,
};
use nocterm_transfers::{Progress, TransferDirection, Transfers};
use nocterm_ui::IconName;
use nocterm_workspace::{Item, ItemEvent, Workspace};
use std::{sync::Arc, time::Duration};

struct Service(Result<Arc<Transfers>, String>);
impl Global for Service {}
pub(super) fn init(cx: &mut App) {
    if !cx.has_global::<Service>() {
        cx.set_global(Service(
            Transfers::new().map(Arc::new).map_err(|e| e.to_string()),
        ));
    }
}
pub(super) fn service(cx: &App) -> Result<Arc<Transfers>, String> {
    cx.try_global::<Service>()
        .ok_or("Upload service is unavailable")?
        .0
        .clone()
}
pub(super) struct TransferStatus {
    snapshot: Vec<Progress>,
    _poll: Task<()>,
}
impl TransferStatus {
    pub(super) fn new(cx: &mut Context<Self>) -> Self {
        Self {
            snapshot: Vec::new(),
            _poll: cx.spawn(async move |this, cx| {
                loop {
                    cx.background_executor()
                        .timer(Duration::from_millis(150))
                        .await;
                    if this
                        .update(cx, |this, cx| {
                            let snapshot = service(cx).map(|s| s.snapshot()).unwrap_or_default();
                            if snapshot != this.snapshot {
                                this.snapshot = snapshot;
                                cx.notify();
                            }
                        })
                        .is_err()
                    {
                        break;
                    }
                }
            }),
        }
    }
}
impl Render for TransferStatus {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let active = self.snapshot.iter().filter(|p| !p.state.finished()).count();
        let failed = self.snapshot.iter().filter(|p| p.failed_files > 0).count();
        Button::new("show-transfers")
            .ghost()
            .small()
            .icon(IconName::Upload)
            .label(if active > 0 {
                format!("Transfers · {active}")
            } else if failed > 0 {
                format!("Transfers · {failed} failed")
            } else {
                "Transfers".into()
            })
            .tooltip("Open transfer queue")
            .on_click(|_, window, cx| window.dispatch_action(Box::new(ShowTransfers), cx))
    }
}
pub(super) struct TransfersView {
    focus: FocusHandle,
    workspace: WeakEntity<Workspace>,
    snapshot: Vec<Progress>,
    error: Option<String>,
    _poll: Task<()>,
}
impl TransfersView {
    pub(super) fn new(workspace: WeakEntity<Workspace>, cx: &mut Context<Self>) -> Self {
        let snapshot = service(cx).map(|s| s.snapshot()).unwrap_or_default();
        Self {
            focus: cx.focus_handle(),
            workspace,
            snapshot,
            error: None,
            _poll: cx.spawn(async move |this, cx| {
                loop {
                    cx.background_executor()
                        .timer(Duration::from_millis(150))
                        .await;
                    if this
                        .update(cx, |this, cx| {
                            let snapshot = service(cx).map(|s| s.snapshot()).unwrap_or_default();
                            if this.snapshot != snapshot {
                                this.snapshot = snapshot;
                                cx.notify();
                            }
                        })
                        .is_err()
                    {
                        break;
                    }
                }
            }),
        }
    }
}
impl EventEmitter<ItemEvent> for TransfersView {}
impl Focusable for TransfersView {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}
impl Item for TransfersView {
    fn tab_title(&self, _: &App) -> SharedString {
        "Transfers".into()
    }
    fn tab_icon(&self, _: &App) -> IconName {
        IconName::Upload
    }
}
impl Render for TransfersView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let workspace = self.workspace.upgrade();
        v_flex()
            .id("upload-queue")
            .track_focus(&self.focus)
            .size_full()
            .overflow_y_scroll()
            .p_4()
            .gap_3()
            .child(div().text_lg().child("Transfers"))
            .when(self.snapshot.is_empty(), |p| {
                p.child(div().text_color(cx.theme().muted_foreground).child(
                    "Drag between Local and Remote in Explorer to upload or download files.",
                ))
            })
            .when_some(self.error.as_ref(), |p, e| {
                p.child(div().text_color(cx.theme().danger).child(e.clone()))
            })
            .children(self.snapshot.iter().map(|job| {
                let id = job.id;
                let retry_fs = workspace
                    .as_ref()
                    .and_then(|workspace| {
                        workspace.read(cx).connected_session_for_target(&job.target)
                    })
                    .and_then(|session| session.fs);
                let direction = match job.direction {
                    TransferDirection::Upload => "Upload",
                    TransferDirection::Download => "Download",
                };
                let description = format!(
                    "{direction} · {} · {} complete / {} files · {} / {}{} · {:?}",
                    job.target,
                    job.completed_files,
                    job.discovered_files,
                    bytes(job.sent_bytes),
                    bytes(job.total_bytes),
                    if job.discovery_complete {
                        ""
                    } else {
                        " · discovering…"
                    },
                    job.state
                );
                v_flex()
                    .gap_1()
                    .pb_3()
                    .border_b_1()
                    .border_color(cx.theme().border)
                    .child(
                        h_flex()
                            .gap_2()
                            .child(Icon::new(IconName::Upload).small())
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .text_ellipsis()
                                    .child(job.destination.clone()),
                            )
                            .child(
                                Button::new(SharedString::from(format!("cancel-upload-{id}")))
                                    .small()
                                    .ghost()
                                    .label("Cancel")
                                    .disabled(job.state.finished())
                                    .on_click(cx.listener(move |_, _, _, cx| {
                                        if let Ok(service) = service(cx) {
                                            service.cancel(id);
                                        }
                                    })),
                            )
                            .child(
                                Button::new(SharedString::from(format!("retry-upload-{id}")))
                                    .small()
                                    .ghost()
                                    .label("Retry")
                                    .disabled(!job.state.finished() || retry_fs.is_none())
                                    .tooltip("Reconnect to this host, then retry explicitly")
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        if let Some(fs) = retry_fs.clone() {
                                            this.error = service(cx)
                                                .and_then(|s| {
                                                    s.retry(id, fs)
                                                        .map(|_| ())
                                                        .map_err(|e| e.to_string())
                                                })
                                                .err();
                                            cx.notify();
                                        }
                                    })),
                            ),
                    )
                    .child(
                        div()
                            .text_sm()
                            .text_color(cx.theme().muted_foreground)
                            .child(description),
                    )
                    .child(
                        gpui_kit::component::progress::Progress::new(SharedString::from(format!(
                            "upload-progress-{id}"
                        )))
                        .small()
                        .w_full()
                        .accessibility_label("Transfer completion")
                        .loading(!job.discovery_complete && !job.state.finished())
                        .value(if job.discovered_files == 0 {
                            0.
                        } else {
                            (job.completed_files + job.skipped_files) as f32
                                / job.discovered_files as f32
                                * 100.
                        }),
                    )
                    .when(job.skipped_files > 0, |p| {
                        p.child(
                            div()
                                .text_sm()
                                .child(format!("{} skipped", job.skipped_files)),
                        )
                    })
                    .children(job.errors.iter().map(|error| {
                        div()
                            .text_xs()
                            .text_color(cx.theme().danger)
                            .child(error.clone())
                    }))
            }))
    }
}
