//! The layouts to choose from, what can be done to the one shown, and undo.

use std::rc::Rc;

use gpui_kit::{
    AnyElement, App, Entity, IntoElement, RenderOnce, SharedString, TestSupportExt as _, Window,
    component::{
        Selectable as _, Sizable as _,
        button::{Button, ButtonVariants as _},
        h_flex,
        input::{Input, InputState},
        v_flex,
    },
    div,
    prelude::*,
};
use nocterm_ui::form;
use nocterm_window_layout::EditOp;

use crate::{
    commands::Commands,
    parts::action,
    section::{LayoutEditorSection, Mode},
    snapshot::Snapshot,
};

/// The row of layouts with Duplicate, Rename, Reset and Delete, the history buttons and the lines that
/// ask or tell something.
#[derive(IntoElement)]
pub(crate) struct Presets {
    snapshot: Rc<Snapshot>,
    rename: Option<(Entity<InputState>, Option<SharedString>)>,
    confirming_delete: bool,
    notice: Option<(SharedString, bool, bool)>,
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
        let (rename, confirming_delete) = match section.mode() {
            Mode::Idle => (None, false),
            Mode::Renaming(draft) => (Some((draft.input().clone(), draft.error().cloned())), false),
            Mode::ConfirmingDelete => (None, true),
        };
        Self {
            snapshot,
            rename,
            confirming_delete,
            notice: section.notice().map(|notice| {
                (
                    notice.text().clone(),
                    notice.is_failure(),
                    notice.offers_undo(),
                )
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

    fn tools(&self) -> AnyElement {
        let saved = !self.snapshot.built_in();
        let commands = &self.commands;
        let duplicate = {
            let commands = commands.clone();
            action("layout-duplicate", "Duplicate", true, move |window, cx| {
                commands.run(window, cx, |section, _, cx| {
                    section.edit(EditOp::Duplicate, cx)
                });
            })
        };
        let rename = {
            let commands = commands.clone();
            action("layout-rename", "Rename", saved, move |window, cx| {
                commands.run(window, cx, |section, window, cx| {
                    section.begin_rename(window, cx);
                });
            })
        };
        let reset = {
            let commands = commands.clone();
            action("layout-reset", "Reset", saved, move |window, cx| {
                commands.run(window, cx, |section, _, cx| section.edit(EditOp::Reset, cx));
            })
        };
        let delete = {
            let commands = commands.clone();
            action("layout-delete", "Delete", saved, move |window, cx| {
                commands.run(window, cx, |section, _, cx| section.ask_delete(cx));
            })
        };
        let undo = {
            let commands = commands.clone();
            action("layout-undo", "Undo", self.can_undo, move |window, cx| {
                commands.run(window, cx, |section, _, cx| section.undo(cx));
            })
        };
        let redo = {
            let commands = commands.clone();
            action("layout-redo", "Redo", self.can_redo, move |window, cx| {
                commands.run(window, cx, |section, _, cx| section.redo(cx));
            })
        };
        h_flex()
            .gap_2()
            .flex_wrap()
            .child(duplicate)
            .child(rename)
            .child(reset)
            .child(delete)
            .child(div().flex_1())
            .child(undo)
            .child(redo)
            .into_any_element()
    }

    fn rename_row(
        &self,
        input: &Entity<InputState>,
        error: Option<&SharedString>,
        cx: &App,
    ) -> AnyElement {
        let (save, cancel) = (self.commands.clone(), self.commands.clone());
        v_flex()
            .id("layout-rename-row")
            .test_support()
            .gap_1()
            .child(
                h_flex()
                    .gap_2()
                    .child(div().flex_1().child(Input::new(input).small()))
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
                    )),
            )
            .children(error.map(|error| {
                div()
                    .id("layout-rename-error")
                    .aria_label(error.clone())
                    .test_support()
                    .child(form::error_text(error.clone(), cx))
            }))
            .into_any_element()
    }

    fn confirm_row(&self) -> AnyElement {
        let (delete, cancel) = (self.commands.clone(), self.commands.clone());
        h_flex()
            .id("layout-delete-confirm")
            .aria_label("Delete this layout?")
            .test_support()
            .gap_2()
            .items_center()
            .child(
                div()
                    .text_sm()
                    .child(format!("Delete \u{201c}{}\u{201d}?", self.snapshot.name())),
            )
            .child(action(
                "layout-delete-confirmed",
                "Delete layout",
                true,
                move |window, cx| {
                    delete.run(window, cx, |section, _, cx| section.confirm_delete(cx));
                },
            ))
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

    fn notice_row(
        &self,
        text: &SharedString,
        failed: bool,
        undoable: bool,
        cx: &App,
    ) -> AnyElement {
        let (undo, dismiss) = (self.commands.clone(), self.commands.clone());
        h_flex()
            .id("layout-notice")
            .aria_label(text.clone())
            .test_support()
            .gap_2()
            .items_center()
            .child(if failed {
                form::error_text(text.clone(), cx)
            } else {
                form::note(text.clone(), cx)
            })
            .when(undoable && self.can_undo, |row| {
                row.child(action(
                    "layout-notice-undo",
                    "Undo",
                    true,
                    move |window, cx| {
                        undo.run(window, cx, |section, _, cx| section.undo(cx));
                    },
                ))
            })
            .child(action(
                "layout-notice-dismiss",
                "Dismiss",
                true,
                move |window, cx| {
                    dismiss.run(window, cx, |section, _, cx| section.dismiss_notice(cx));
                },
            ))
            .into_any_element()
    }

    /// Built-in layouts that are not in use cannot be edited; the copy can.
    fn read_only(&self, cx: &App) -> AnyElement {
        let commands = self.commands.clone();
        h_flex()
            .id("layout-read-only")
            .aria_label("Duplicate to edit")
            .test_support()
            .gap_2()
            .items_center()
            .child(form::note(
                format!(
                    "\u{201c}{}\u{201d} is built in and is not the layout in use, so it cannot be changed.",
                    self.snapshot.name()
                ),
                cx,
            ))
            .child(action(
                "layout-duplicate-to-edit",
                "Duplicate to edit",
                true,
                move |window, cx| {
                    commands.run(window, cx, |section, _, cx| section.edit(EditOp::Duplicate, cx));
                },
            ))
            .into_any_element()
    }
}

impl RenderOnce for Presets {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let mut column = v_flex().id("layout-presets").test_support().gap_2();
        column = column.child(self.choices()).child(self.tools());
        if let Some((input, error)) = &self.rename {
            column = column.child(self.rename_row(input, error.as_ref(), cx));
        }
        if self.confirming_delete {
            column = column.child(self.confirm_row());
        }
        if let Some((text, failed, undoable)) = &self.notice {
            column = column.child(self.notice_row(text, *failed, *undoable, cx));
        }
        if !self.snapshot.editable() {
            column = column.child(self.read_only(cx));
        }
        column
    }
}
