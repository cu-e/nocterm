//! The layouts to choose from and one line of tools under them: what can be done to the layout
//! shown, what just happened to it, and undo. Renaming and confirming a delete take that line
//! for themselves, so the bar below never moves.

use std::rc::Rc;

use gpui_kit::{
    AnyElement, App, Entity, IntoElement, RenderOnce, SharedString, TestSupportExt as _, Window,
    component::{
        ActiveTheme as _, Disableable as _, Selectable as _, Sizable as _,
        button::{Button, ButtonVariants as _},
        h_flex,
        input::{Input, InputState},
        v_flex,
    },
    div,
    prelude::*,
    rems,
};
use nocterm_ui::IconName;
use nocterm_window_layout::EditOp;

use crate::{
    commands::Commands,
    parts::action,
    rename::Renamed,
    section::{LayoutEditorSection, Mode},
    snapshot::Snapshot,
};

/// The height of the line of tools, whatever it holds.
const TOOLS_HEIGHT_REMS: f32 = 2.;

/// What the line of tools shows.
enum Line {
    Tools,
    Renaming {
        what: &'static str,
        input: Entity<InputState>,
        error: Option<SharedString>,
    },
    ConfirmingDelete,
}

/// A line about what just happened.
struct NoticeLine {
    text: SharedString,
    failed: bool,
    undoable: bool,
}

#[derive(IntoElement)]
pub(crate) struct Presets {
    snapshot: Rc<Snapshot>,
    line: Line,
    notice: Option<NoticeLine>,
    can_undo: bool,
    can_redo: bool,
    commands: Commands,
}

impl Presets {
    pub(crate) fn new(
        section: &LayoutEditorSection,
        snapshot: Rc<Snapshot>,
        commands: Commands,
    ) -> Self {
        let line = match section.mode() {
            Mode::Idle => Line::Tools,
            Mode::Renaming(draft) => Line::Renaming {
                what: match draft.renamed() {
                    Renamed::Layout(_) => "Layout name",
                    Renamed::Group(_) => "Group name",
                },
                input: draft.input().clone(),
                error: draft.error().cloned(),
            },
            Mode::ConfirmingDelete => Line::ConfirmingDelete,
        };
        Self {
            snapshot,
            line,
            notice: section.notice().map(|notice| NoticeLine {
                text: notice.text().clone(),
                failed: notice.is_failure(),
                undoable: notice.offers_undo(),
            }),
            can_undo: section.can_undo(),
            can_redo: section.can_redo(),
            commands,
        }
    }

    fn choices(&self) -> AnyElement {
        let buttons = self.snapshot.choices().iter().map(|choice| {
            let (commands, id) = (self.commands.clone(), choice.id().clone());
            let label = if choice.in_use() {
                format!("{} \u{b7} in use", choice.name())
            } else {
                choice.name().to_owned()
            };
            Button::new(SharedString::from(format!("layout-choice:{}", choice.id())))
                .small()
                .ghost()
                .label(label)
                .selected(choice.id() == self.snapshot.shown())
                .on_click(move |_, window, cx| {
                    let id = id.clone();
                    commands.run(window, cx, move |section, window, cx| {
                        section.show(id, window, cx);
                    });
                })
                .into_any_element()
        });
        h_flex()
            .flex_wrap()
            .gap_1()
            .children(buttons)
            .into_any_element()
    }

    /// Duplicate, and for a saved layout Rename, Reset and Delete; then what just happened, and
    /// the history. A built-in that is not in use cannot be changed, so its copy is offered.
    fn tools(&self, cx: &App) -> AnyElement {
        let commands = &self.commands;
        let duplicate = {
            let commands = commands.clone();
            let run = move |window: &mut Window, cx: &mut App| {
                commands.run(window, cx, |section, _, cx| {
                    section.edit(EditOp::Duplicate, cx)
                });
            };
            if self.snapshot.editable() {
                action("layout-duplicate", "Duplicate", true, run)
            } else {
                Button::new("layout-duplicate-to-edit")
                    .small()
                    .primary()
                    .label("Duplicate to edit")
                    .on_click(move |_, window, cx| run(window, cx))
            }
        };
        let saved = !self.snapshot.built_in();
        let rename = {
            let commands = commands.clone();
            action("layout-rename", "Rename", true, move |window, cx| {
                commands.run(window, cx, |section, window, cx| {
                    section.begin_rename(window, cx);
                });
            })
        };
        let reset = {
            let commands = commands.clone();
            action("layout-reset", "Reset", true, move |window, cx| {
                commands.run(window, cx, |section, _, cx| section.edit(EditOp::Reset, cx));
            })
        };
        let delete = {
            let commands = commands.clone();
            action("layout-delete", "Delete", true, move |window, cx| {
                commands.run(window, cx, |section, _, cx| section.ask_delete(cx));
            })
        };
        let undo = {
            let commands = commands.clone();
            history("layout-undo", "Undo", self.can_undo, move |window, cx| {
                commands.run(window, cx, |section, _, cx| section.undo(cx));
            })
        };
        let redo = {
            let commands = commands.clone();
            history("layout-redo", "Redo", self.can_redo, move |window, cx| {
                commands.run(window, cx, |section, _, cx| section.redo(cx));
            })
        };
        h_flex()
            .gap_2()
            .child(duplicate)
            .when(saved, |row| row.child(rename).child(reset).child(delete))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .children(self.notice.as_ref().map(|notice| self.notice(notice, cx))),
            )
            .child(undo)
            .child(redo)
            .into_any_element()
    }

    fn renaming(
        &self,
        what: &'static str,
        input: &Entity<InputState>,
        error: Option<&SharedString>,
        cx: &App,
    ) -> AnyElement {
        let (save, cancel) = (self.commands.clone(), self.commands.clone());
        h_flex()
            .id("layout-rename-row")
            .test_support()
            .gap_2()
            .child(
                div()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(what),
            )
            .child(div().w(rems(16.)).child(Input::new(input).small()))
            .child(action(
                "layout-rename-save",
                "Save",
                true,
                move |window, cx| {
                    save.run(window, cx, |section, window, cx| {
                        section.commit_rename(window, cx);
                    });
                },
            ))
            .child(action(
                "layout-rename-cancel",
                "Cancel",
                true,
                move |window, cx| {
                    cancel.run(window, cx, |section, window, cx| {
                        section.cancel_rename(window, cx);
                    });
                },
            ))
            .children(error.map(|error| {
                div()
                    .id("layout-rename-error")
                    .aria_label(error.clone())
                    .test_support()
                    .min_w_0()
                    .truncate()
                    .text_xs()
                    .text_color(cx.theme().danger)
                    .child(error.clone())
            }))
            .into_any_element()
    }

    fn confirming_delete(&self) -> AnyElement {
        let (delete, cancel) = (self.commands.clone(), self.commands.clone());
        h_flex()
            .id("layout-delete-confirm")
            .aria_label("Delete this layout?")
            .test_support()
            .gap_2()
            .child(
                div()
                    .text_sm()
                    .child(format!("Delete \u{201c}{}\u{201d}?", self.snapshot.name())),
            )
            .child(
                Button::new("layout-delete-confirmed")
                    .small()
                    .danger()
                    .label("Delete layout")
                    .on_click(move |_, window, cx| {
                        delete.run(window, cx, |section, _, cx| section.confirm_delete(cx));
                    }),
            )
            .child(action(
                "layout-delete-cancel",
                "Keep it",
                true,
                move |window, cx| {
                    cancel.run(window, cx, |section, _, cx| section.cancel_delete(cx));
                },
            ))
            .into_any_element()
    }

    /// What just happened, on the line of tools: the text, Undo when the change can be undone,
    /// and a way to dismiss it.
    fn notice(&self, notice: &NoticeLine, cx: &App) -> AnyElement {
        let (undo, dismiss) = (self.commands.clone(), self.commands.clone());
        let theme = cx.theme();
        h_flex()
            .id("layout-notice")
            .aria_label(notice.text.clone())
            .test_support()
            .min_w_0()
            .gap_1()
            .child(
                div()
                    .min_w_0()
                    .truncate()
                    .text_xs()
                    .text_color(if notice.failed {
                        theme.danger
                    } else {
                        theme.muted_foreground
                    })
                    .child(notice.text.clone()),
            )
            .when(notice.undoable && self.can_undo, |row| {
                row.child(
                    Button::new("layout-notice-undo")
                        .xsmall()
                        .link()
                        .label("Undo")
                        .on_click(move |_, window, cx| {
                            undo.run(window, cx, |section, _, cx| section.undo(cx));
                        }),
                )
            })
            .child(
                Button::new("layout-notice-dismiss")
                    .xsmall()
                    .ghost()
                    .icon(IconName::Close)
                    .on_click(move |_, window, cx| {
                        dismiss.run(window, cx, |section, _, cx| section.dismiss_notice(cx));
                    }),
            )
            .into_any_element()
    }
}

/// A quiet button of the history.
fn history(
    id: &'static str,
    label: &'static str,
    enabled: bool,
    on_click: impl Fn(&mut Window, &mut App) + 'static,
) -> Button {
    Button::new(id)
        .small()
        .ghost()
        .label(label)
        .disabled(!enabled)
        .on_click(move |_, window, cx| on_click(window, cx))
}

impl RenderOnce for Presets {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let line = match &self.line {
            Line::Tools => self.tools(cx),
            Line::Renaming { what, input, error } => self.renaming(what, input, error.as_ref(), cx),
            Line::ConfirmingDelete => self.confirming_delete(),
        };
        v_flex()
            .id("layout-presets")
            .test_support()
            .gap_2()
            .child(self.choices())
            .child(
                h_flex()
                    .id("layout-tools")
                    .test_support()
                    .h(rems(TOOLS_HEIGHT_REMS))
                    .items_center()
                    .child(div().flex_1().min_w_0().child(line)),
            )
    }
}
