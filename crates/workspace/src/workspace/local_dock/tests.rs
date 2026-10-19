//! Native bottom header interactions and multi-session lifetime regressions.
mod lifecycle;
mod targets;

use super::*;
use gpui_kit::component::WindowExt as _;
use gpui_kit::{TestAppContext, px, test::TestWindowExt as _};
use std::cell::{Cell, RefCell};

struct Probe {
    focus: FocusHandle,
    cwd: PathBuf,
    closes: Rc<Cell<usize>>,
    commands: Rc<RefCell<Vec<PathBuf>>>,
    refusal: Option<crate::LocalDirectoryError>,
}
impl EventEmitter<ItemEvent> for Probe {}
impl Focusable for Probe {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}
impl Render for Probe {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .size_full()
            .track_focus(&self.focus)
            .on_action(
                cx.listener(|this, _: &gpui_kit::component::input::Copy, _, _| {
                    this.commands.borrow_mut().push(this.cwd.clone())
                }),
            )
    }
}
impl Item for Probe {
    fn tab_title(&self, _: &App) -> SharedString {
        self.cwd.display().to_string().into()
    }
    fn on_close(&mut self, _: &mut Window, _: &mut Context<Self>) {
        self.closes.set(self.closes.get() + 1);
    }
}
impl crate::LocalTerminal for Probe {
    fn cwd(&self, _: &App) -> Option<PathBuf> {
        Some(self.cwd.clone())
    }
    fn change_directory(
        &mut self,
        path: PathBuf,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<(), crate::LocalDirectoryError> {
        if let Some(error) = &self.refusal {
            return Err(error.clone());
        }
        self.cwd = path;
        cx.emit(ItemEvent::Changed);
        Ok(())
    }
}
fn probe(cx: &mut App, path: &str, closes: Rc<Cell<usize>>) -> Entity<Probe> {
    probe_with_refusal(cx, path, closes, None)
}
fn probe_with_refusal(
    cx: &mut App,
    path: &str,
    closes: Rc<Cell<usize>>,
    refusal: Option<crate::LocalDirectoryError>,
) -> Entity<Probe> {
    cx.new(|cx| Probe {
        focus: cx.focus_handle(),
        cwd: path.into(),
        closes,
        commands: Rc::default(),
        refusal,
    })
}
fn fixture(
    cx: &mut TestAppContext,
) -> (
    gpui_kit::AnyWindowHandle,
    Entity<Workspace>,
    Rc<Cell<usize>>,
) {
    let (handle, workspace) = super::super::tests::fixture(cx);
    let closes = Rc::new(Cell::new(0));
    let counts = closes.clone();
    workspace.update(cx, |workspace, _| {
        workspace.set_local_terminal_opener(move |workspace, target, window, cx| {
            let item = probe(cx, "/new", counts.clone());
            workspace.add_local_terminal_at(item, target, window, cx);
        })
    });
    (handle, workspace, closes)
}
fn count(workspace: &Entity<Workspace>, cx: &App) -> usize {
    workspace
        .read(cx)
        .tabs(cx)
        .iter()
        .filter(|tab| tab.is_local())
        .count()
}

#[gpui_kit::test]
fn native_header_plus_precedes_zoom_and_blank_double_click_adds_exactly_one(
    cx: &mut TestAppContext,
) {
    let (handle, workspace, closes) = fixture(cx);
    cx.update_window(handle, |_, window, cx| {
        workspace.update(cx, |workspace, cx| workspace.new_local_terminal(window, cx));
        window.render_frame(cx);
        let plus = window.find("local-terminal-new").bounds();
        let zoom = window.find("zoom-in").bounds();
        assert!(plus.right() <= zoom.left());
        assert!(window.try_find("menu").is_none());
        let blank = window.find("local-terminal-free-header");
        assert!(blank.visible() && blank.bounds().size.width > px(20.));
        window.click("zoom-in", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(workspace.read(cx).zoomed_pane(cx).is_some());
        window.click("local-terminal-new", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(count(&workspace, cx), 2);
        assert!(workspace.read(cx).zoomed_pane(cx).is_some());
        window.double_click("local-terminal-free-header", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(count(&workspace, cx), 3);
        window.click("zoom-out", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(workspace.read(cx).zoomed_pane(cx).is_none());
    })
    .unwrap();
    assert_eq!(closes.get(), 0);
}

#[gpui_kit::test]
fn local_tabs_keep_selected_cwd_commands_order_height_and_individual_close(
    cx: &mut TestAppContext,
) {
    let (handle, workspace, _) = fixture(cx);
    let a_closes = Rc::new(Cell::new(0));
    let b_closes = Rc::new(Cell::new(0));
    let (a, b) = cx
        .update_window(handle, |_, window, cx| {
            let a = probe(cx, "/a", a_closes.clone());
            let b = probe(cx, "/b", b_closes.clone());
            workspace.update(cx, |workspace, cx| {
                workspace.add_local_terminal(a.clone(), window, cx);
                workspace.add_local_terminal(b.clone(), window, cx);
            });
            window.render_frame(cx);
            (a, b)
        })
        .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        workspace.update(cx, |workspace, cx| {
            workspace.activate_item_by_id(b.entity_id(), window, cx);
            workspace.move_active_tab(-1, window, cx);
            workspace.activate_item_by_id(a.entity_id(), window, cx);
        });
        crate::workspace::tests::resize_bottom(window, &workspace, px(260.), cx);
        window.render_frame(cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        workspace.update(cx, |workspace, cx| {
            assert_eq!(workspace.local_terminal_cwd(cx), Some("/a".into()));
            workspace.dispatch_command(Box::new(gpui_kit::component::input::Copy), window, cx);
        });
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        workspace.update(cx, |workspace, cx| {
            workspace
                .change_local_directory("/changed".into(), window, cx)
                .unwrap();
            workspace.toggle_local_terminal(window, cx);
            assert_eq!(workspace.selected_local_terminal(cx), Some(a.entity_id()));
            let (_, node) = workspace
                .tabs(cx)
                .iter()
                .find(|tab| tab.id() == a.entity_id())
                .unwrap()
                .location()
                .unwrap();
            let panes = workspace.tab_panes(DockPlacement::Bottom, cx);
            let pane = panes.iter().find(|pane| pane.node() == node).unwrap();
            assert_eq!(pane.items(), &[b.entity_id(), a.entity_id()]);
            workspace.toggle_local_terminal(window, cx);
            assert_eq!(
                workspace.dock_region_size(DockPlacement::Bottom, cx),
                Some(px(260.))
            );
            assert_eq!(workspace.local_terminal_cwd(cx), Some("/changed".into()));
            workspace.close_item_by_id(b.entity_id(), window, cx);
            assert_eq!(
                workspace.selected_local_terminal(cx).unwrap(),
                a.entity_id()
            );
            assert_eq!(workspace.local_terminal_cwd(cx), Some("/changed".into()));
            workspace.close_local_terminal(window, cx);
            assert!(workspace.selected_local_terminal(cx).is_none());
        });
        window.render_frame(cx);
    })
    .unwrap();
    cx.run_until_parked();
    assert_eq!(a_closes.get(), 1);
    assert_eq!(b_closes.get(), 1);
    assert_eq!(
        *a.read_with(cx, |a, _| a.commands.clone()).borrow(),
        [PathBuf::from("/a")]
    );
}

#[gpui_kit::test]
fn native_alias_zoom_and_close_double_clicks_do_not_open_extra_local_tabs(cx: &mut TestAppContext) {
    let (handle, workspace, closes) = fixture(cx);
    let id = cx
        .update_window(handle, |_, window, cx| {
            workspace.update(cx, |workspace, cx| {
                workspace.new_local_terminal(window, cx);
                workspace.new_local_terminal(window, cx);
                workspace.selected_local_terminal(cx).unwrap()
            })
        })
        .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.double_click(("tab-title", id.as_u64()), cx);
        window.render_frame(cx);
        assert_eq!(count(&workspace, cx), 2);
        assert!(
            window.focused_input(cx).is_some(),
            "Tab label retains native alias editing"
        );
        window.press("escape", cx);
        window.double_click("zoom-in", cx);
    })
    .unwrap();
    cx.run_until_parked();
    assert_eq!(
        workspace.read_with(cx, |workspace, cx| workspace
            .tabs(cx)
            .iter()
            .filter(|tab| tab.is_local())
            .count()),
        2
    );
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.double_click(("close-tab", id.as_u64()), cx);
    })
    .unwrap();
    cx.run_until_parked();
    assert_eq!(
        workspace.read_with(cx, |workspace, cx| workspace
            .tabs(cx)
            .iter()
            .filter(|tab| tab.is_local())
            .count()),
        1
    );
    assert_eq!(closes.get(), 1);
}

#[gpui_kit::test]
fn native_drag_to_free_header_reorders_without_opening_or_closing_sessions(
    cx: &mut TestAppContext,
) {
    let (handle, workspace, closes) = fixture(cx);
    let first = cx
        .update_window(handle, |_, window, cx| {
            workspace.update(cx, |workspace, cx| {
                workspace.new_local_terminal(window, cx);
                let first = workspace.selected_local_terminal(cx).unwrap();
                workspace.new_local_terminal(window, cx);
                first
            })
        })
        .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.drag_to(
            ("tab-title", first.as_u64()),
            "local-terminal-free-header",
            cx,
        );
    })
    .unwrap();
    cx.run_until_parked();
    workspace.read_with(cx, |workspace, cx| {
        assert_eq!(
            workspace
                .tabs(cx)
                .iter()
                .filter(|tab| tab.is_local())
                .count(),
            2
        );
        let (_, node) = workspace
            .tabs(cx)
            .iter()
            .find(|tab| tab.id() == first)
            .unwrap()
            .location()
            .unwrap();
        let panes = workspace.tab_panes(DockPlacement::Bottom, cx);
        let pane = panes.iter().find(|pane| pane.node() == node).unwrap();
        assert_eq!(pane.items().last(), Some(&first));
    });
    assert_eq!(closes.get(), 0);
}

#[gpui_kit::test]
fn selected_split_group_controls_cwd_addition_and_toolkit_removal_closes_once(
    cx: &mut TestAppContext,
) {
    let (handle, workspace, closes) = fixture(cx);
    let (a, b) = cx
        .update_window(handle, |_, window, cx| {
            let a = probe(cx, "/a", closes.clone());
            let b = probe(cx, "/b", closes.clone());
            workspace.update(cx, |workspace, cx| {
                workspace.add_local_terminal(a.clone(), window, cx);
                workspace.add_local_terminal(b.clone(), window, cx);
                workspace.split_item(
                    b.entity_id(),
                    gpui_kit::component::Placement::Right,
                    window,
                    cx,
                );
                window.focus(&b.read(cx).focus_handle(cx), cx);
            });
            window.render_frame(cx);
            (a, b)
        })
        .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        workspace.update(cx, |workspace, cx| {
            assert_eq!(workspace.local_terminal_cwd(cx), Some("/b".into()));
            let node = workspace
                .tabs(cx)
                .iter()
                .find(|tab| tab.id() == b.entity_id())
                .unwrap()
                .location()
                .unwrap()
                .1;
            workspace.new_local_terminal(window, cx);
            let new_item = workspace
                .tabs(cx)
                .into_iter()
                .rfind(|tab| tab.is_local())
                .unwrap();
            assert_eq!(new_item.location().map(|(_, node)| node), Some(node));
            workspace.activate_item_by_id(b.entity_id(), window, cx);
        });
        window.render_frame(cx);
        window.dispatch_action(Box::new(gpui_kit::component::dock::ClosePanel), cx);
        window.render_frame(cx);
    })
    .unwrap();
    cx.run_until_parked();
    assert_eq!(closes.get(), 1);
    workspace.read_with(cx, |workspace, cx| {
        assert!(
            workspace
                .tabs(cx)
                .iter()
                .any(|tab| tab.id() == a.entity_id() && tab.is_local())
        );
        assert!(
            !workspace
                .tabs(cx)
                .iter()
                .any(|tab| tab.id() == b.entity_id() && tab.is_local())
        );
        assert_eq!(
            workspace
                .tabs(cx)
                .iter()
                .filter(|tab| tab.is_local())
                .count(),
            2
        );
    });
}

#[gpui_kit::test]
fn directory_request_opens_local_shell_and_preserves_selected_target_on_refusal(
    cx: &mut TestAppContext,
) {
    let (handle, workspace, _) = fixture(cx);
    cx.update_window(handle, |_, window, cx| {
        workspace.update(cx, |workspace, cx| {
            workspace
                .change_local_directory("/opened".into(), window, cx)
                .unwrap();
            assert_eq!(workspace.local_terminal_cwd(cx), Some("/opened".into()));
        });
        let refusal = crate::LocalDirectoryError::Busy {
            command: "cd -- '/refused'".into(),
        };
        let shell = probe_with_refusal(cx, "/busy", Rc::default(), Some(refusal.clone()));
        workspace.update(cx, |workspace, cx| {
            workspace.add_local_terminal(shell.clone(), window, cx);
            workspace.toggle_local_terminal(window, cx);
            assert_eq!(
                workspace.change_local_directory("/refused".into(), window, cx),
                Err(refusal)
            );
            assert_eq!(
                workspace.selected_local_terminal(cx),
                Some(shell.entity_id())
            );
            assert_eq!(workspace.local_terminal_cwd(cx), Some("/busy".into()));
            assert!(workspace.local_terminal_is_visible(cx));
            assert_eq!(
                workspace
                    .tabs(cx)
                    .iter()
                    .filter(|tab| tab.is_local())
                    .count(),
                2
            );
        });
    })
    .unwrap();
}

#[gpui_kit::test]
fn directory_request_reports_unavailable_when_opener_registers_no_shell(cx: &mut TestAppContext) {
    let (handle, workspace, _) = fixture(cx);
    workspace.update(cx, |workspace, _| {
        workspace.set_local_terminal_opener(|_, _, _, _| {})
    });
    cx.update_window(handle, |_, window, cx| {
        workspace.update(cx, |workspace, cx| {
            assert_eq!(
                workspace.change_local_directory("/tmp".into(), window, cx),
                Err(crate::LocalDirectoryError::Unavailable)
            );
            assert!(workspace.selected_local_terminal(cx).is_none());
        });
    })
    .unwrap();
}
