//! A searchable dialog that asks the user to pick one value from a short list.
use std::rc::Rc;

use gpui_kit::{
    App, AppContext as _, Window,
    component::{
        WindowExt as _,
        command::{Command, CommandItem, CommandState},
    },
    prelude::*,
    px,
};

/// One row of a choice picker: what it shows, whether it is the current choice, and what it
/// stands for.
#[derive(Clone)]
pub struct PickerOption<T> {
    label: String,
    checked: bool,
    value: T,
}

impl<T> PickerOption<T> {
    pub fn new(label: impl Into<String>, checked: bool, value: T) -> Self {
        Self {
            label: label.into(),
            checked,
            value,
        }
    }
}

/// Opens a searchable dialog over `options`. Confirming a row closes the dialog, returns focus
/// to where it was and calls `on_choose` with that row's value; dismissing it calls nothing.
pub fn open_choice_picker<T: Clone + 'static>(
    placeholder: &'static str,
    options: Vec<PickerOption<T>>,
    on_choose: impl Fn(T, &mut Window, &mut App) + 'static,
    window: &mut Window,
    cx: &mut App,
) {
    let origin = window.focused(cx);
    let options = Rc::new(options);
    let on_choose = Rc::new(on_choose);
    let state = cx.new(|cx| CommandState::new(window, cx));
    let field = state.clone();
    window.open_dialog(cx, move |dialog, _, _| {
        let confirm = options.clone();
        let on_choose = on_choose.clone();
        let origin = origin.clone();
        dialog.w(px(560.)).p_0().close_button(false).child(
            Command::new(&state)
                .items(
                    options
                        .iter()
                        .map(|option| {
                            CommandItem::new()
                                .label(option.label.clone())
                                .checked(option.checked)
                        })
                        .collect::<Vec<_>>(),
                )
                .placeholder(placeholder)
                .bordered(false)
                .max_h(px(420.))
                .on_confirm(move |index, window, cx| {
                    let Some(option) = confirm.get(index.row) else {
                        return;
                    };
                    let value = option.value.clone();
                    window.close_dialog(cx);
                    if let Some(origin) = &origin {
                        window.focus(origin, cx);
                    }
                    on_choose(value, window, cx);
                }),
        )
    });
    field.update(cx, |state, cx| state.focus(window, cx));
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui_kit::{
        Context, IntoElement, Render, TestAppContext, WindowOptions, div, test::TestWindowExt as _,
    };
    use std::{cell::RefCell, rc::Rc};

    struct Blank;
    impl Render for Blank {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            div()
        }
    }

    fn open(
        picked: &Rc<RefCell<Vec<&'static str>>>,
        cx: &mut TestAppContext,
    ) -> gpui_kit::AnyWindowHandle {
        let picked = picked.clone();
        cx.update(|cx| {
            gpui_kit::init(cx);
            crate::init(
                crate::DesignTokens::builtin(),
                nocterm_store::SettingsConfig::in_memory(
                    nocterm_settings::SettingsDocument::default(),
                )
                .install(cx),
                cx,
            );
            let (handle, _) =
                gpui_kit::open_window(WindowOptions::default(), cx, |_, cx| cx.new(|_| Blank))
                    .unwrap();
            handle
                .update(cx, move |_, window, cx| {
                    open_choice_picker(
                        "Pick one",
                        vec![
                            PickerOption::new("First", true, "first"),
                            PickerOption::new("Second", false, "second"),
                        ],
                        move |value, _, _| picked.borrow_mut().push(value),
                        window,
                        cx,
                    );
                })
                .unwrap();
            handle
        })
    }

    #[gpui_kit::test]
    fn confirming_a_row_closes_the_dialog_and_reports_its_value(cx: &mut TestAppContext) {
        let picked = Rc::new(RefCell::new(Vec::new()));
        let handle = open(&picked, cx);
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            assert!(window.has_active_dialog(cx));
            window.press("down", cx);
            window.press("enter", cx);
        })
        .unwrap();
        cx.run_until_parked();
        assert_eq!(*picked.borrow(), ["second"]);
        cx.update_window(handle, |_, window, cx| {
            assert!(!window.has_active_dialog(cx));
        })
        .unwrap();
    }

    #[gpui_kit::test]
    fn dismissing_the_dialog_reports_nothing(cx: &mut TestAppContext) {
        let picked = Rc::new(RefCell::new(Vec::new()));
        let handle = open(&picked, cx);
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            window.press("escape", cx);
        })
        .unwrap();
        cx.run_until_parked();
        assert!(picked.borrow().is_empty());
        cx.update_window(handle, |_, window, cx| {
            assert!(!window.has_active_dialog(cx));
        })
        .unwrap();
    }
}
