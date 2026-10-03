use super::{local, operations::FileTarget};
use gpui_kit::{
    App, ClipboardItem, Context, Entity, FocusHandle, Focusable, SharedString, Subscription,
    Window,
    component::{
        ActiveTheme as _, Disableable as _, Sizable as _, StyledExt as _, WindowExt as _,
        button::{Button, ButtonVariants as _},
        checkbox::Checkbox,
        h_flex,
        input::{Input, InputEvent, InputState},
        menu::{PopupMenu, PopupMenuItem},
        v_flex,
    },
    div,
    prelude::*,
    rems,
};
use nocterm_session::{FileMetadata, FsError};
use nocterm_ui::ActiveDesign as _;
use std::{
    rc::Rc,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

pub(super) type Refresh = Rc<dyn Fn(&mut App)>;
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Kind {
    Rename,
    Delete,
    Properties,
}
pub(super) fn menu(
    menu: PopupMenu,
    target: FileTarget,
    selected: Vec<FileTarget>,
    refresh: Refresh,
    enabled: bool,
) -> PopupMenu {
    let capabilities = target.capabilities();
    let name = target.name();
    let path = target.path();
    let rename = target.clone();
    let properties = target;
    let rename_refresh = refresh.clone();
    let properties_refresh = refresh.clone();
    menu.item(
        PopupMenuItem::new("Rename…")
            .disabled(!enabled || !capabilities.rename)
            .on_click(move |_, window, cx| {
                open(
                    Kind::Rename,
                    vec![rename.clone()],
                    rename_refresh.clone(),
                    window,
                    cx,
                )
            }),
    )
    .separator()
    .item(
        PopupMenuItem::new("Copy name").on_click(move |_, _, cx| {
            cx.write_to_clipboard(ClipboardItem::new_string(name.clone()))
        }),
    )
    .item(
        PopupMenuItem::new("Copy path").on_click(move |_, _, cx| {
            cx.write_to_clipboard(ClipboardItem::new_string(path.clone()))
        }),
    )
    .separator()
    .item(
        PopupMenuItem::new("Delete…")
            .disabled(!enabled || !capabilities.remove)
            .on_click(move |_, window, cx| {
                open(Kind::Delete, selected.clone(), refresh.clone(), window, cx)
            }),
    )
    .separator()
    .item(
        PopupMenuItem::new("Properties…")
            .disabled(!enabled || !capabilities.metadata)
            .on_click(move |_, window, cx| {
                open(
                    Kind::Properties,
                    vec![properties.clone()],
                    properties_refresh.clone(),
                    window,
                    cx,
                )
            }),
    )
}
enum Operation {
    Metadata,
    Rename(String),
    Delete,
    Permissions(u32),
}
enum ResultValue {
    Metadata(FileMetadata),
    Done,
}
pub(super) struct FileDialog {
    focus: FocusHandle,
    kind: Kind,
    targets: Vec<FileTarget>,
    name: Entity<InputState>,
    metadata: Option<FileMetadata>,
    mode: u32,
    pending: bool,
    error: Option<String>,
    refresh: Refresh,
    cancel: Arc<AtomicBool>,
    dismissed: Arc<AtomicBool>,
    _name_subscription: Subscription,
}
impl Drop for FileDialog {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Release);
        self.dismissed.store(true, Ordering::Release);
    }
}
pub(super) fn open(
    kind: Kind,
    targets: Vec<FileTarget>,
    refresh: Refresh,
    window: &mut Window,
    cx: &mut App,
) {
    // Close the menu before mounting a modal: its last render still owns focus.
    // This also lets the modal restore focus to the browser rather than the menu.
    window.defer(cx, move |window, cx| {
        open_modal(kind, targets, refresh, window, cx);
    });
}
fn open_modal(
    kind: Kind,
    targets: Vec<FileTarget>,
    refresh: Refresh,
    window: &mut Window,
    cx: &mut App,
) {
    if targets.is_empty() {
        return;
    }
    let title = match kind {
        Kind::Rename => "Rename",
        Kind::Delete => "Delete permanently",
        Kind::Properties => "Properties",
    };
    let view = cx.new(|cx| FileDialog::new(kind, targets, refresh, window, cx));
    let focus = view.read(cx).focus_handle(cx);
    let width = rems(cx.design().layout.dialog_width).to_pixels(window.rem_size());
    let cancel = view.read(cx).cancel.clone();
    let dismissed = view.read(cx).dismissed.clone();
    window.open_dialog(cx, move |dialog, _, cx| {
        let submit = view.clone();
        let submit_button = view.clone();
        let dismiss = view.clone();
        let dismiss_button = view.clone();
        let cancel = cancel.clone();
        let dismissed = dismissed.clone();
        let stopping = view.read(cx).pending && kind == Kind::Delete;
        let can_submit = view.read(cx).can_submit(cx);
        dialog
            .title(title)
            .w(width)
            .child(view.clone())
            .footer(
                h_flex()
                    .justify_end()
                    .gap_2()
                    .child(
                        Button::new("file-dialog-close")
                            .label(if stopping { "Stop" } else { "Close" })
                            .on_click(move |_, window, cx| {
                                let close = dismiss_button.update(cx, |this, _| this.dismiss());
                                if close {
                                    window.close_dialog(cx);
                                }
                            }),
                    )
                    .child(
                        Button::new("file-dialog-submit")
                            .label(match kind {
                                Kind::Rename => "Rename",
                                Kind::Delete => "Delete",
                                Kind::Properties => "Apply permissions",
                            })
                            .disabled(!can_submit)
                            .when(kind == Kind::Delete, |b| b.danger())
                            .when(kind != Kind::Delete, |b| b.primary())
                            .on_click(move |_, window, cx| {
                                submit_button.update(cx, |this, cx| this.submit(window, cx))
                            }),
                    ),
            )
            .on_ok(move |_, window, cx| {
                submit.update(cx, |this, cx| this.submit(window, cx));
                false
            })
            .on_cancel(move |_, _, cx| dismiss.update(cx, |this, _| this.dismiss()))
            .on_close(move |_, _, _| {
                cancel.store(true, Ordering::Release);
                dismissed.store(true, Ordering::Release);
            })
    });
    window.focus(&focus, cx);
}
impl FileDialog {
    fn dismiss(&self) -> bool {
        if self.pending && self.kind == Kind::Delete {
            self.cancel.store(true, Ordering::Release);
            false
        } else {
            self.dismissed.store(true, Ordering::Release);
            self.cancel.store(true, Ordering::Release);
            true
        }
    }
    fn can_submit(&self, cx: &App) -> bool {
        if self.pending || self.dismissed.load(Ordering::Acquire) {
            return false;
        }
        match self.kind {
            Kind::Rename => {
                super::local_operations::valid_name(self.name.read(cx).value().as_ref())
            }
            Kind::Delete => true,
            Kind::Properties => self.metadata.as_ref().is_some_and(|m| {
                !m.is_symlink
                    && m.permissions.is_some_and(|mode| mode != self.mode)
                    && self.targets[0].capabilities().set_permissions
            }),
        }
    }
    fn new(
        kind: Kind,
        targets: Vec<FileTarget>,
        refresh: Refresh,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let name = cx.new(|cx| InputState::new(window, cx).default_value(targets[0].name()));
        let name_subscription = cx.subscribe(&name, |_, _, event, cx| {
            if matches!(event, InputEvent::Change) {
                cx.notify();
            }
        });
        let mut this = Self {
            focus: cx.focus_handle(),
            kind,
            targets,
            name,
            metadata: None,
            mode: 0,
            pending: false,
            error: None,
            refresh,
            cancel: Arc::new(AtomicBool::new(false)),
            dismissed: Arc::new(AtomicBool::new(false)),
            _name_subscription: name_subscription,
        };
        if kind == Kind::Properties {
            this.run(Operation::Metadata, window, cx);
        }
        this
    }
    fn submit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.can_submit(cx) {
            return;
        }
        let op = match self.kind {
            Kind::Rename => Operation::Rename(self.name.read(cx).value().to_string()),
            Kind::Delete => Operation::Delete,
            Kind::Properties => {
                let Some(m) = &self.metadata else {
                    return;
                };
                if m.is_symlink
                    || m.permissions.is_none()
                    || !self.targets[0].capabilities().set_permissions
                {
                    return;
                }
                Operation::Permissions(self.mode)
            }
        };
        self.run(op, window, cx);
    }
    fn run(&mut self, operation: Operation, window: &mut Window, cx: &mut Context<Self>) {
        self.pending = true;
        self.error = None;
        self.cancel.store(false, Ordering::Release);
        let targets = self.targets.clone();
        let cancel = self.cancel.clone();
        let metadata_request = matches!(&operation, Operation::Metadata);
        let close_success = matches!(&operation, Operation::Rename(_) | Operation::Delete);
        let worker = cx.background_executor().spawn(async move {
            match operation {
                Operation::Metadata => targets[0].metadata().await.map(ResultValue::Metadata),
                Operation::Rename(name) => targets[0].rename(name).await.map(|_| ResultValue::Done),
                Operation::Permissions(mode) => {
                    targets[0].set_permissions(mode).await?;
                    targets[0].metadata().await.map(ResultValue::Metadata)
                }
                Operation::Delete => {
                    for target in targets {
                        if cancel.load(Ordering::Acquire) {
                            return Err(FsError::Other(
                                "Deletion cancelled; some entries may already have been deleted."
                                    .into(),
                            ));
                        }
                        target.remove(cancel.clone()).await?;
                    }
                    Ok(ResultValue::Done)
                }
            }
        });
        let refresh = self.refresh.clone();
        cx.spawn_in(window, async move |this, cx| {
            let result = worker.await;
            let _ = cx.update(|window, app| {
                // A closed modal must not prevent refreshing a partial delete
                // or an already accepted atomic mutation. The callback is weak
                // and checks the browser generation and filesystem identity.
                if !metadata_request {
                    (refresh)(app);
                }
                this.update(app, |this, cx| {
                    // Rendered listeners may keep a dismissed model alive.
                    // Its completion must not update or close a newer modal.
                    if this.dismissed.load(Ordering::Acquire) {
                        return;
                    }
                    this.pending = false;
                    match result {
                        Ok(ResultValue::Metadata(m)) => {
                            this.mode = m.permissions.unwrap_or(0);
                            this.metadata = Some(m);
                        }
                        Ok(ResultValue::Done) => {}
                        Err(error) => this.error = Some(error.to_string()),
                    }
                    if close_success && this.error.is_none() {
                        this.dismissed.store(true, Ordering::Release);
                        window.close_dialog(cx);
                    } else {
                        cx.notify();
                    }
                })
            });
        })
        .detach();
        cx.notify();
    }
    fn property(label: &str, value: impl Into<SharedString>) -> impl IntoElement {
        h_flex()
            .gap_3()
            .items_start()
            .child(
                div()
                    .w_24()
                    .flex_shrink_0()
                    .text_sm()
                    .child(label.to_owned()),
            )
            .child(div().flex_1().min_w_0().text_sm().child(value.into()))
    }
    fn properties(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let mut content = v_flex().gap_2();
        let Some(m) = &self.metadata else {
            return content;
        };
        content = content
            .child(Self::property(
                "Type",
                if m.is_symlink {
                    "Symbolic link"
                } else {
                    match m.kind {
                        nocterm_session::EntryKind::Directory => "Folder",
                        nocterm_session::EntryKind::File => "File",
                        _ => "Special file",
                    }
                },
            ))
            .child(Self::property(
                "Size reported",
                m.size
                    .map(local::bytes)
                    .unwrap_or_else(|| "Not reported".into()),
            ))
            .child(Self::property(
                "Owner / group",
                format!(
                    "{} / {}",
                    m.uid.map(|n| n.to_string()).unwrap_or_else(|| "—".into()),
                    m.gid.map(|n| n.to_string()).unwrap_or_else(|| "—".into())
                ),
            ))
            .child(Self::property(
                "Modified",
                m.modified
                    .map(format_time)
                    .unwrap_or_else(|| "Not reported".into()),
            ));
        if let Some(original) = m.permissions {
            let disabled =
                self.pending || m.is_symlink || !self.targets[0].capabilities().set_permissions;
            content = content.child(
                div()
                    .mt_2()
                    .text_sm()
                    .font_semibold()
                    .child(format!("Permissions · {:04o}", self.mode)),
            );
            for (label, shift) in [("Owner", 6), ("Group", 3), ("Others", 0)] {
                let mut row = h_flex().gap_3().child(div().w_16().text_sm().child(label));
                for (right, bit) in [("Read", 4u32), ("Write", 2), ("Execute", 1)] {
                    let bit = bit << shift;
                    row = row.child(
                        Checkbox::new(SharedString::from(format!("permission-{label}-{right}")))
                            .label(right)
                            .checked(self.mode & bit != 0)
                            .disabled(disabled)
                            .on_click(cx.listener(move |this, checked: &bool, _, cx| {
                                if *checked {
                                    this.mode |= bit;
                                } else {
                                    this.mode &= !bit;
                                }
                                cx.notify();
                            })),
                    );
                }
                content = content.child(row);
            }
            if original & 0o7000 != 0 {
                content = content.child(
                    div()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child("Setuid, setgid and sticky bits are retained."),
                );
            }
            if disabled && !self.pending {
                content = content.child(
                    div()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child("Permissions are read-only for this entry or filesystem."),
                );
            }
        } else {
            content = content.child(
                div()
                    .text_sm()
                    .text_color(cx.theme().muted_foreground)
                    .child("POSIX permissions are not provided by this filesystem."),
            );
        }
        content
    }
}
impl Focusable for FileDialog {
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        if self.kind == Kind::Rename {
            self.name.read(cx).focus_handle(cx)
        } else {
            self.focus.clone()
        }
    }
}
impl Render for FileDialog {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let target = &self.targets[0];
        v_flex().track_focus(&self.focus).gap_3().min_w_0()
            .child(Self::property("Location",target.location()))
            .child(div().text_sm().child(target.path()))
            .when(self.kind==Kind::Rename,|v|v.child(Input::new(&self.name).disabled(self.pending)))
            .when(self.kind==Kind::Delete,|v|v.child(div().text_sm().child(format!("Permanently delete {} selected item(s) and all folder contents? This cannot be undone. Symbolic-link targets are kept.",self.targets.len())))
                .children(self.targets.iter().skip(1).take(7).map(|t|div().text_sm().child(t.path())))
                .when(self.targets.len()>8,|v|v.child(div().text_sm().child(format!("… and {} more selected items",self.targets.len()-8)))))
            .when(self.kind==Kind::Properties,|v|v.child(self.properties(cx)))
            .when(self.pending,|v|v.child(div().text_sm().text_color(cx.theme().muted_foreground).child(if self.kind==Kind::Delete{"Deleting… Close requests cancellation."}else{"Working…"})))
            .when_some(self.error.clone(),|v,e|v.child(div().text_sm().text_color(cx.theme().danger).child(e)))
            .when(self.kind==Kind::Properties&&!self.pending&&self.metadata.is_none(),|v|v.child(Button::new("retry-properties").small().ghost().label("Retry").on_click(cx.listener(|this,_,window,cx|this.run(Operation::Metadata,window,cx)))))
    }
}
fn format_time(seconds: u64) -> String {
    i64::try_from(seconds)
        .ok()
        .and_then(|s| chrono::DateTime::from_timestamp(s, 0))
        .map(|t| t.format("%Y-%m-%d %H:%M:%S UTC").to_string())
        .unwrap_or_else(|| "Date outside the supported range".into())
}
