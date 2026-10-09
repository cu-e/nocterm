//! Pointer interactions route snippets to the user-selected terminal only.
use super::*;
use gpui_kit::{
    EventEmitter, InputEvent as _, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent,
    point, px,
};
use nocterm_workspace::{
    Item, ItemEvent, LocalTerminal, SignInPrompt, TerminalAccess, TerminalInfo, TerminalStatus,
    TerminalText, TextRequest,
};
use std::{
    cell::{Cell, RefCell},
    path::PathBuf,
    rc::Rc,
};

struct Access {
    info: RefCell<Option<TerminalInfo>>,
    sent: RefCell<Vec<(String, bool)>>,
    reject: Cell<bool>,
}

#[gpui_kit::test]
fn hidden_bottom_refuses_run_and_show_restores_the_same_terminal(cx: &mut TestAppContext) {
    let (handle, panel) = fixture(cx);
    let original = snippet();
    seed(&original, cx);
    let row: SharedString = format!("snippet-{}", original.id).into();
    let access = Access::new(true);
    let workspace = cx
        .update_window(handle, |_, window, cx| {
            let workspace = panel.read(cx).workspace.upgrade().unwrap();
            let item = terminal(access.clone(), cx);
            workspace.update(cx, |workspace, cx| {
                workspace.set_local_terminal(item.clone(), window, cx)
            });
            window.render_frame(cx);
            window.focus(&panel.read(cx).focus_handle(cx), cx);
            window.render_frame(cx);
            window.focus(&item.read(cx).focus_handle(cx), cx);
            window.render_frame(cx);
            workspace
        })
        .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("run", cx);
        assert_eq!(access.sent.borrow().len(), 1);
        workspace.update(cx, |workspace, cx| {
            workspace.toggle_local_terminal(window, cx)
        });
        window.render_frame(cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(!workspace.read(cx).local_terminal_is_visible(cx));
        window.click("run", cx);
        click_row(row, 2, window, cx);
        assert_eq!(
            access.sent.borrow().len(),
            1,
            "Hidden bottom receives no command"
        );
        workspace.update(cx, |workspace, cx| {
            workspace.toggle_local_terminal(window, cx)
        });
        window.render_frame(cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(workspace.read(cx).local_terminal_is_visible(cx));
        window.click("run", cx);
        assert_eq!(
            access.sent.borrow().len(),
            2,
            "Show restores the existing process"
        );
    })
    .unwrap();
}
impl Access {
    fn new(local: bool) -> Rc<Self> {
        Rc::new(Self {
            info: RefCell::new(Some(TerminalInfo {
                title: if local { "Local" } else { "Remote" }.into(),
                local,
                target: None,
                profile: (!local).then(|| "remote-profile".into()),
                status: TerminalStatus::Connected,
                cwd: None,
                at_prompt: None,
                dirty_input: false,
                alt_screen: false,
                generation: 1,
                sign_in: None,
            })),
            sent: RefCell::new(Vec::new()),
            reject: Cell::new(false),
        })
    }
}
impl TerminalAccess for Access {
    fn info(&self, _: &App) -> Option<TerminalInfo> {
        self.info.borrow().clone()
    }
    fn read(&self, _: TextRequest, _: &App) -> Result<TerminalText, String> {
        Err("Unused by snippets".into())
    }
    fn send_text(&self, _: &str, _: &mut App) -> Result<(), String> {
        panic!("Snippets must use the native paste contract")
    }
    fn run_command(&self, _: &str, _: &mut App) -> Result<(), String> {
        panic!("Snippets must preserve agent command guards")
    }
    fn paste_snippet(&self, text: &str, execute: bool, _: &mut App) -> Result<(), String> {
        if self.reject.get() {
            return Err("Input queue rejected the snippet".into());
        }
        self.sent.borrow_mut().push((text.to_owned(), execute));
        Ok(())
    }
    fn answer_sign_in(&self, _: String, _: &mut App) -> Result<(), String> {
        panic!("Snippets must never answer authentication")
    }
}
struct Terminal {
    access: Rc<Access>,
    focus: FocusHandle,
}
impl EventEmitter<ItemEvent> for Terminal {}
impl Focusable for Terminal {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}
impl Render for Terminal {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div().size_full().track_focus(&self.focus)
    }
}
impl Item for Terminal {
    fn tab_title(&self, _: &App) -> SharedString {
        "Terminal".into()
    }
    fn terminal_access(&self) -> Option<Rc<dyn TerminalAccess>> {
        Some(self.access.clone())
    }
}
impl LocalTerminal for Terminal {
    fn cwd(&self, _: &App) -> Option<PathBuf> {
        None
    }
    fn change_directory(
        &mut self,
        _: PathBuf,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Result<(), String> {
        Err("Unused by snippets".into())
    }
}

fn terminal(access: Rc<Access>, cx: &mut App) -> Entity<Terminal> {
    cx.new(|cx| Terminal {
        access,
        focus: cx.focus_handle(),
    })
}

// Target the row's left padding so a toolbar button cannot intercept this gesture.
fn click_row(id: SharedString, count: usize, window: &mut Window, cx: &mut App) {
    window.render_frame(cx);
    let position = window.find(id).bounds().origin + point(px(8.), px(8.));
    window.dispatch_event(
        MouseMoveEvent {
            position,
            pressed_button: None,
            modifiers: Default::default(),
        }
        .to_platform_input(),
        cx,
    );
    for click_count in 1..=count {
        window.dispatch_event(
            MouseDownEvent {
                position,
                button: MouseButton::Left,
                modifiers: Default::default(),
                click_count,
                first_mouse: false,
            }
            .to_platform_input(),
            cx,
        );
        window.render_frame(cx);
        window.dispatch_event(
            MouseUpEvent {
                position,
                button: MouseButton::Left,
                modifiers: Default::default(),
                click_count,
            }
            .to_platform_input(),
            cx,
        );
        window.render_frame(cx);
    }
}

#[gpui_kit::test]
#[expect(clippy::too_many_lines, reason = "predates the limit")]
fn run_and_double_click_follow_bottom_then_remote_focus_and_ignore_nested_actions(
    cx: &mut TestAppContext,
) {
    let (handle, panel) = fixture(cx);
    let original = snippet();
    seed(&original, cx);
    let row: SharedString = format!("snippet-{}", original.id).into();
    let remote = Access::new(false);
    let local = Access::new(true);
    let (workspace, remote_item, local_item) = cx
        .update_window(handle, |_, window, cx| {
            let workspace = panel.read(cx).workspace.upgrade().unwrap();
            let remote_item = terminal(remote.clone(), cx);
            let local_item = terminal(local.clone(), cx);
            workspace.update(cx, |workspace, cx| {
                workspace.add_item(remote_item.clone(), window, cx);
                workspace.set_local_terminal(local_item.clone(), window, cx);
            });
            window.render_frame(cx);
            window.focus(&panel.read(cx).focus_handle(cx), cx);
            window.render_frame(cx);
            window.focus(&local_item.read(cx).focus_handle(cx), cx);
            window.render_frame(cx);
            (workspace, remote_item, local_item)
        })
        .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        workspace.update(cx, |workspace, cx| {
            workspace.activate_panel_of::<SnippetsPanel>(window, cx)
        });
        window.render_frame(cx);
        assert!(panel.read(cx).profile.is_none());
        window.click("run", cx);
        assert!(
            local_item.read(cx).focus_handle(cx).is_focused(window),
            "Run focuses the receiving bottom terminal"
        );
        assert_eq!(
            local.sent.borrow().as_slice(),
            [(original.content.clone(), true)]
        );
        assert!(remote.sent.borrow().is_empty());
        click_row(row.clone(), 1, window, cx);
        assert_eq!(
            local.sent.borrow().len(),
            1,
            "Single click only selects the row"
        );
        click_row(row.clone(), 2, window, cx);
        assert!(
            local_item.read(cx).focus_handle(cx).is_focused(window),
            "Double click focuses the receiving bottom terminal"
        );
        assert_eq!(
            local.sent.borrow().len(),
            2,
            "Double click executes exactly once"
        );
        window.double_click("copy", cx);
        assert_eq!(
            cx.read_from_clipboard().unwrap().text().unwrap(),
            original.content
        );
        assert_eq!(local.sent.borrow().len(), 2, "Copy must not run the row");
        window.click("edit", cx);
        window.render_frame(cx);
        window.click("snippet-cancel", cx);
        window.render_frame(cx);
        window.click("delete", cx);
        window.render_frame(cx);
        window.click("cancel", cx);
        assert_eq!(
            local.sent.borrow().len(),
            2,
            "Nested actions must not run the row"
        );
        window.focus(&remote_item.read(cx).focus_handle(cx), cx);
        window.render_frame(cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        workspace.update(cx, |workspace, cx| {
            workspace.activate_panel_of::<SnippetsPanel>(window, cx)
        });
        window.render_frame(cx);
        assert_eq!(panel.read(cx).profile.as_deref(), Some("remote-profile"));
        window.click("run", cx);
        assert!(
            remote_item.read(cx).focus_handle(cx).is_focused(window),
            "Run focuses the receiving center terminal"
        );
        assert_eq!(
            remote.sent.borrow().as_slice(),
            [(original.content.clone(), true)]
        );
        assert_eq!(local.sent.borrow().len(), 2);
        window.focus(&panel.read(cx).focus_handle(cx), cx);
        window.render_frame(cx);
        click_row(row.clone(), 2, window, cx);
        assert_eq!(
            remote.sent.borrow().len(),
            2,
            "Center double click executes exactly once"
        );
        assert!(
            remote_item.read(cx).focus_handle(cx).is_focused(window),
            "Double click focuses the receiving center terminal"
        );
        remote.reject.set(true);
        window.focus(&panel.read(cx).focus_handle(cx), cx);
        window.render_frame(cx);
        window.click("run", cx);
        assert!(
            panel.read(cx).focus_handle(cx).is_focused(window),
            "Input queue rejection keeps panel focus"
        );
        assert_eq!(remote.sent.borrow().len(), 2);
        remote.reject.set(false);
        window.focus(&local_item.read(cx).focus_handle(cx), cx);
        window.render_frame(cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        assert!(panel.read(cx).profile.is_none());
        workspace.update(cx, |workspace, cx| {
            workspace.close_local_terminal(window, cx)
        });
        window.render_frame(cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(panel.read(cx).profile.as_deref(), Some("remote-profile"));
        window.click("run", cx);
        assert_eq!(
            remote.sent.borrow().len(),
            3,
            "Closed bottom falls back to the current tab"
        );
        assert_eq!(
            local.sent.borrow().len(),
            2,
            "Closed bottom must not receive input"
        );
    })
    .unwrap();
}

#[gpui_kit::test]
#[expect(clippy::too_many_lines, reason = "predates the limit")]
fn run_refuses_authentication_alternate_closed_and_background_terminals(cx: &mut TestAppContext) {
    let (handle, panel) = fixture(cx);
    let original = snippet();
    seed(&original, cx);
    let row: SharedString = format!("snippet-{}", original.id).into();
    let access = Access::new(false);
    let (workspace, item) = cx
        .update_window(handle, |_, window, cx| {
            let workspace = panel.read(cx).workspace.upgrade().unwrap();
            let item = terminal(access.clone(), cx);
            workspace.update(cx, |workspace, cx| {
                workspace.add_item(item.clone(), window, cx)
            });
            window.render_frame(cx);
            (workspace, item)
        })
        .unwrap();
    cx.run_until_parked();
    for status in [
        TerminalStatus::Connecting,
        TerminalStatus::AwaitingUser,
        TerminalStatus::AwaitingVault,
        TerminalStatus::Closed,
    ] {
        access.info.borrow_mut().as_mut().unwrap().status = status;
        cx.update(|cx| item.update(cx, |_, cx| cx.emit(ItemEvent::Changed)));
        cx.run_until_parked();
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            window.focus(&panel.read(cx).focus_handle(cx), cx);
            window.render_frame(cx);
            window.click("run", cx);
            click_row(row.clone(), 2, window, cx);
            assert!(
                panel.read(cx).focus_handle(cx).is_focused(window),
                "Rejected Run keeps panel focus"
            );
        })
        .unwrap();
        assert!(access.sent.borrow().is_empty(), "Refuse {status:?}");
    }
    for authentication in [false, true] {
        {
            let mut info = access.info.borrow_mut();
            let info = info.as_mut().unwrap();
            info.status = TerminalStatus::Connected;
            info.alt_screen = !authentication;
            info.sign_in = authentication.then(|| SignInPrompt {
                label: "Password".into(),
                retry: false,
                masked: true,
            });
        }
        cx.update(|cx| item.update(cx, |_, cx| cx.emit(ItemEvent::Changed)));
        cx.run_until_parked();
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            window.focus(&panel.read(cx).focus_handle(cx), cx);
            window.render_frame(cx);
            window.click("run", cx);
            click_row(row.clone(), 2, window, cx);
            assert!(
                panel.read(cx).focus_handle(cx).is_focused(window),
                "Rejected Run keeps panel focus"
            );
        })
        .unwrap();
        assert!(access.sent.borrow().is_empty());
    }
    {
        let mut info = access.info.borrow_mut();
        let info = info.as_mut().unwrap();
        info.alt_screen = false;
        info.sign_in = None;
    }
    cx.update(|cx| item.update(cx, |_, cx| cx.emit(ItemEvent::Changed)));
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("run", cx);
        assert_eq!(
            access.sent.borrow().len(),
            1,
            "Returning to the shell re-enables Run"
        );
    })
    .unwrap();
    *access.info.borrow_mut() = None;
    cx.update(|cx| item.update(cx, |_, cx| cx.emit(ItemEvent::Changed)));
    cx.run_until_parked();
    let background = Access::new(false);
    cx.update_window(handle, |_, window, cx| {
        workspace.update(cx, |workspace, cx| workspace.close_item(0, window, cx));
        let background_item = terminal(background.clone(), cx);
        workspace.update(cx, |workspace, cx| {
            workspace.add_background_item(background_item, window, cx);
        });
        window.render_frame(cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("run", cx);
        click_row(row, 2, window, cx);
    })
    .unwrap();
    assert_eq!(
        access.sent.borrow().len(),
        1,
        "Closed terminal receives no more input"
    );
    assert!(background.sent.borrow().is_empty());
}
