//! The active connection's snippets precede the collapsible remaining library.
mod run;
use crate::{editor, model::Snippets};
use gpui_kit::{
    App, ClickEvent, ClipboardItem, Context, Entity, FocusHandle, Focusable, SharedString,
    Subscription, WeakEntity, Window,
    base::TestSupportExt as _,
    component::{
        ActiveTheme as _, Disableable as _, Icon, Sizable as _, StyledExt as _, WindowExt as _,
        button::{Button, ButtonVariants as _},
        h_flex,
        input::{Input, InputState},
        v_flex,
    },
    div,
    prelude::*,
};
use nocterm_snippets::Snippet;
use nocterm_ui::IconName;
use nocterm_workspace::{Panel, Workspace, WorkspaceEvent};

pub struct SnippetsPanel {
    model: Entity<Snippets>,
    workspace: WeakEntity<Workspace>,
    focus: FocusHandle,
    search: Entity<InputState>,
    profile: Option<String>,
    group: Option<String>,
    context: SharedString,
    folded: bool,
    copied: Option<String>,
    run_enabled: bool,
    _subscriptions: Vec<Subscription>,
}
impl SnippetsPanel {
    pub(crate) fn new(
        model: Entity<Snippets>,
        workspace: Entity<Workspace>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let search = cx.new(|cx| InputState::new(window, cx).placeholder("Search snippets…"));
        let subscriptions = vec![
            cx.observe(&model, |_, _, cx| cx.notify()),
            cx.observe(&search, |_, _, cx| cx.notify()),
            cx.subscribe(&workspace, |_, _, event, cx| {
                if matches!(
                    event,
                    WorkspaceEvent::ActiveItemChanged
                        | WorkspaceEvent::ActiveSessionChanged
                        | WorkspaceEvent::ConnectionsChanged
                        | WorkspaceEvent::ItemsChanged
                ) {
                    // Workspace events can originate inside Item updates. Read after those borrows end.
                    let panel = cx.weak_entity();
                    cx.defer(move |cx| {
                        let _ = panel.update(cx, |this, cx| {
                            this.refresh_context(cx);
                            cx.notify();
                        });
                    });
                }
            }),
        ];
        let panel = Self {
            model,
            workspace: workspace.downgrade(),
            focus: cx.focus_handle(),
            search,
            profile: None,
            group: None,
            context: "No active terminal".into(),
            folded: false,
            copied: None,
            run_enabled: false,
            _subscriptions: subscriptions,
        };
        let weak = cx.weak_entity();
        cx.defer(move |cx| {
            let _ = weak.update(cx, |this, cx| {
                this.refresh_context(cx);
                cx.notify();
            });
        });
        panel
    }
    fn refresh_context(&mut self, cx: &App) {
        let Some(workspace) = self.workspace.upgrade() else {
            return;
        };
        let workspace = workspace.read(cx);
        let target = self.target(cx).ok().map(|target| target.info);
        self.run_enabled = target.as_ref().is_some_and(Self::ready);
        self.profile = target
            .as_ref()
            .and_then(|info| info.profile.as_ref().map(ToString::to_string));
        let title = target.map_or_else(|| "No active terminal".into(), |info| info.title);
        let saved = workspace.connection_directory().and_then(|directory| {
            directory
                .connections(cx)
                .into_iter()
                .find(|connection| Some(connection.id.as_ref()) == self.profile.as_deref())
        });
        self.group = saved
            .as_ref()
            .and_then(|connection| connection.group.as_ref().map(ToString::to_string));
        self.context = saved.map_or(title, |connection| connection.name);
    }
    fn confirm_delete(&self, snippet: Snippet, window: &mut Window, cx: &mut Context<Self>) {
        let model = self.model.clone();
        window.open_alert_dialog(cx, move |alert, _, _| {
            let model = model.clone();
            let snippet = snippet.clone();
            alert
                .title(format!("Delete snippet “{}”?", snippet.name))
                .description("This permanently removes the saved snippet.")
                .ok_text("Delete")
                .show_cancel(true)
                .on_ok(move |_, window, cx| {
                    let saved = model.update(cx, |model, cx| model.delete(snippet.clone(), cx));
                    window
                        .spawn(cx, async move |cx| {
                            if let Err(error) = saved.await {
                                let _ = cx.update(|window, cx| {
                                    nocterm_ui::notice::error(
                                        window,
                                        cx,
                                        "snippet-delete",
                                        "Could not delete snippet",
                                        error,
                                    )
                                });
                            }
                        })
                        .detach();
                    true
                })
        });
    }
    #[expect(clippy::too_many_lines, reason = "predates the limit")]
    fn row(&self, snippet: &Snippet, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let edit = snippet.clone();
        let delete = snippet.clone();
        let copy = snippet.clone();
        let run = snippet.clone();
        let double_click = snippet.clone();
        let copied = self.copied.as_deref() == Some(&snippet.id.to_string());
        let theme = cx.theme().clone();
        v_flex()
            .id(SharedString::from(format!("snippet-{}", snippet.id)))
            .test_support()
            .on_click(cx.listener(move |this, event: &ClickEvent, window, cx| {
                if event.click_count() == 2 {
                    cx.stop_propagation();
                    this.run(&double_click.content, window, cx);
                }
            }))
            .px_3()
            .py_2()
            .gap_1()
            .border_b_1()
            .border_color(theme.border.opacity(0.5))
            .cursor_pointer()
            .hover(|row| row.bg(theme.sidebar_accent))
            .child(
                h_flex()
                    .gap_1()
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .text_sm()
                            .font_medium()
                            .truncate()
                            .child(snippet.name.clone()),
                    )
                    .child(
                        Button::new("run").ghost().xsmall().icon(IconName::Play).label("Run")
                            .disabled(!self.run_enabled)
                            .tooltip(if self.run_enabled { format!("Run in {}", self.context) } else { "Focus a connected terminal outside an alternate screen to run snippets".into() })
                            .on_click(cx.listener(move |this, event: &ClickEvent, window, cx| {
                                cx.stop_propagation();
                                if event.click_count() == 1 {
                                    this.run(&run.content, window, cx);
                                }
                            })),
                    )
                    .child(
                        Button::new("copy")
                            .ghost()
                            .xsmall()
                            .icon(if copied {
                                IconName::Check
                            } else {
                                IconName::Copy
                            })
                            .tooltip(if copied { "Copied" } else { "Copy snippet" })
                            .on_click(cx.listener(move |this, _, _, cx| {
                                cx.stop_propagation();
                                cx.write_to_clipboard(ClipboardItem::new_string(
                                    copy.content.clone(),
                                ));
                                this.copied = Some(copy.id.to_string());
                                cx.notify();
                            })),
                    )
                    .child(
                        Button::new("edit")
                            .ghost()
                            .xsmall()
                            .icon(IconName::Pencil)
                            .tooltip("Edit snippet")
                            .on_click(cx.listener(move |this, _, window, cx| {
                                cx.stop_propagation();
                                editor::open(
                                    Some(edit.clone()),
                                    this.workspace.clone(),
                                    window,
                                    cx,
                                );
                            })),
                    )
                    .child(
                        Button::new("delete")
                            .ghost()
                            .xsmall()
                            .icon(IconName::Trash)
                            .tooltip("Delete snippet")
                            .on_click(cx.listener(move |this, _, window, cx| {
                                cx.stop_propagation();
                                this.confirm_delete(delete.clone(), window, cx)
                            })),
                    ),
            )
            .child(
                h_flex()
                    .gap_2()
                    .text_xs()
                    .text_color(theme.muted_foreground)
                    .child(snippet.language.label()),
            )
    }
}
impl Panel for SnippetsPanel {
    fn title(&self, _: &App) -> SharedString {
        "Snippets".into()
    }
    fn icon(&self, _: &App) -> IconName {
        IconName::ScrollText
    }
}
impl Focusable for SnippetsPanel {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}
impl Render for SnippetsPanel {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let model = self.model.read(cx);
        let error = model.error().map(ToOwned::to_owned);
        let writable = model.writable();
        let query = self.search.read(cx).value();
        let (current, other) =
            model
                .library
                .partition(self.profile.as_deref(), self.group.as_deref(), &query);
        let current: Vec<_> = current.into_iter().cloned().collect();
        let other: Vec<_> = other.into_iter().cloned().collect();
        let empty = current.is_empty() && other.is_empty();
        let searching = !query.trim().is_empty();
        let theme = cx.theme().clone();
        v_flex().track_focus(&self.focus).size_full().gap_1()
            .child(h_flex().px_3().gap_1().text_xs().text_color(theme.muted_foreground).child(Icon::new(IconName::Server).xsmall()).child(div().flex_1().min_w_0().truncate().child(self.context.clone()))
                .child(Button::new("snippet-new").ghost().xsmall().icon(IconName::Plus).tooltip("New snippet").disabled(!writable).on_click(cx.listener(|this, _, window, cx| { editor::open(None, this.workspace.clone(), window, cx); }))))
            .child(div().px_3().py_1().child(Input::new(&self.search).small()))
            .when_some(error, |panel, error| panel.child(div().px_3().py_2().text_xs().text_color(theme.danger).child(format!("Could not save or load snippets: {error}"))))
            .child(v_flex().id("snippets-list").flex_1().min_h_0().overflow_y_scroll()
                .when(!current.is_empty(), |list| list.child(div().px_3().pt_2().pb_1().text_xs().font_medium().text_color(theme.muted_foreground).child("For this server")).children(current.iter().map(|snippet| self.row(snippet, cx))))
                .when(empty, |list| list.child(div().px_3().py_4().text_sm().text_color(theme.muted_foreground).child(if searching { "No matching snippets." } else { "Save reusable commands and code with +. Assign them to servers or groups to see them here." })))
                .when(!other.is_empty(), |list| list.child(Button::new("snippets-other").ghost().small().w_full().justify_start().icon(if self.folded { IconName::ChevronRight } else { IconName::ChevronDown }).label(format!("All other snippets ({})", other.len())).on_click(cx.listener(|this, _, _, cx| { this.folded = !this.folded; cx.notify(); }))).when(!self.folded, |list| list.children(other.iter().map(|snippet| self.row(snippet, cx))))))
    }
}
#[cfg(test)]
mod tests;
