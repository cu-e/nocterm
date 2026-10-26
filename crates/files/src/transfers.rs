//! Presentation of the retained transfer service. Available outside Explorer.
use crate::{ShowTransfers, local::bytes};
use gpui_kit::{
    App, Context, EventEmitter, FocusHandle, Focusable, SharedString, Task, WeakEntity, Window,
    component::{
        ActiveTheme as _, Disableable as _, Icon, Sizable as _,
        button::{Button, ButtonVariants as _},
        h_flex, v_flex,
    },
    div,
    prelude::*,
};
use nocterm_transfers::{Progress, TransferDirection, TransferService};
use nocterm_ui::IconName;
use nocterm_workspace::{Item, ItemEvent, Workspace};
use std::{sync::Arc, time::Duration};

pub(super) struct TransferStatus {
    service: Arc<dyn TransferService>,
    snapshot: Vec<Progress>,
    _poll: Task<()>,
}
impl TransferStatus {
    pub(super) fn new(
        service: Arc<dyn TransferService>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        Self {
            snapshot: service.snapshot(),
            service,
            _poll: cx.spawn_in(window, async move |this, cx| {
                loop {
                    cx.background_executor()
                        .timer(Duration::from_millis(150))
                        .await;
                    if this
                        .update_in(cx, |this, window, cx| {
                            let snapshot = this.service.snapshot();
                            if snapshot != this.snapshot {
                                for job in &snapshot {
                                    if job.state.finished()
                                        && job.failed_files > 0
                                        && !this
                                            .snapshot
                                            .iter()
                                            .any(|old| old.id == job.id && old.state.finished())
                                    {
                                        let error = job
                                            .errors
                                            .first()
                                            .map(ToString::to_string)
                                            .unwrap_or_else(|| {
                                                format!(
                                                    "{} files failed; review the transfer queue.",
                                                    job.failed_files
                                                )
                                            });
                                        nocterm_ui::notice::error_action(
                                            window,
                                            cx,
                                            "files-transfer-failed",
                                            "Transfer failed",
                                            error,
                                            "Open Transfers",
                                            |window, cx| {
                                                window.defer(cx, |window, cx| {
                                                    window.dispatch_action(
                                                        Box::new(ShowTransfers),
                                                        cx,
                                                    )
                                                });
                                            },
                                        );
                                    }
                                }
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
    service: Arc<dyn TransferService>,
    focus: FocusHandle,
    workspace: WeakEntity<Workspace>,
    snapshot: Vec<Progress>,
    _poll: Task<()>,
}
impl TransfersView {
    pub(super) fn new(
        service: Arc<dyn TransferService>,
        workspace: WeakEntity<Workspace>,
        cx: &mut Context<Self>,
    ) -> Self {
        let snapshot = service.snapshot();
        Self {
            service,
            focus: cx.focus_handle(),
            workspace,
            snapshot,
            _poll: cx.spawn(async move |this, cx| {
                loop {
                    cx.background_executor()
                        .timer(Duration::from_millis(150))
                        .await;
                    if this
                        .update(cx, |this, cx| {
                            let snapshot = this.service.snapshot();
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
    #[expect(clippy::too_many_lines, reason = "predates the limit")]
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
            .children(self.snapshot.iter().map(|job| {
                let id = job.id;
                let retry_fs = workspace
                    .as_ref()
                    .and_then(|workspace| {
                        workspace.read(cx).connected_session_for_host(&job.host, cx)
                    })
                    .and_then(|session| session.fs);
                let direction = match job.direction {
                    TransferDirection::Upload => "Upload",
                    TransferDirection::Download => "Download",
                };
                let description = format!(
                    "{direction} · {} · {} complete / {} files · {} / {}{} · {:?}",
                    job.host,
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
                                    .on_click(cx.listener(move |this, _, window, cx| {
                                        if let Err(error) = this.service.cancel(id) {
                                            nocterm_ui::notice::error(
                                                window,
                                                cx,
                                                "files-transfer-cancel",
                                                "Could not cancel transfer",
                                                error.to_string(),
                                            );
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
                                    .on_click(cx.listener(move |this, _, window, cx| {
                                        if let Some(fs) = retry_fs.clone() {
                                            let error = this.service.retry(id, fs).err();
                                            if let Some(error) = error {
                                                nocterm_ui::notice::error(
                                                    window,
                                                    cx,
                                                    "files-transfer-retry",
                                                    "Could not retry transfer",
                                                    error.to_string(),
                                                );
                                            }
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
                            .child(error.to_string())
                    }))
            }))
    }
}
