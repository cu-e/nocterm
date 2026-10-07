//! Native bottom header interactions and multi-session lifetime regressions.
mod lifecycle;

use super::*;
use gpui_kit::component::WindowExt as _;
use gpui_kit::{TestAppContext, px, test::TestWindowExt as _};
use std::cell::{Cell, RefCell};

struct Probe {
    focus: FocusHandle,
    cwd: PathBuf,
    closes: Rc<Cell<usize>>,
    commands: Rc<RefCell<Vec<PathBuf>>>,
}
impl EventEmitter<ItemEvent> for Probe {}
impl Focusable for Probe {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}
impl Render for Probe {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div().size_full().track_focus(&self.focus)
    }
}
impl Item for Probe {
    fn tab_title(&self, _: &App) -> SharedString {
        self.cwd.display().to_string().into()
    }
    fn on_close(&mut self, _: &mut Window, _: &mut Context<Self>) {
        self.closes.set(self.closes.get() + 1);
    }
    fn command_enabled(&self, _: ItemCommand, _: &App) -> bool {
        true
    }
    fn execute(&mut self, _: ItemCommand, _: &mut Window, _: &mut Context<Self>) {
        self.commands.borrow_mut().push(self.cwd.clone());
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
    ) -> Result<(), String> {
        self.cwd = path;
        cx.emit(ItemEvent::Changed);
        Ok(())
    }
}
fn probe(cx: &mut App, path: &str, closes: Rc<Cell<usize>>) -> Entity<Probe> {
    cx.new(|cx| Probe {
        focus: cx.focus_handle(),
        cwd: path.into(),
        closes,
        commands: Rc::default(),
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
        workspace.set_local_terminal_opener(move |workspace, window, cx| {
            let item = probe(cx, "/new", counts.clone());
            workspace.add_local_terminal(item, window, cx);
        })
    });
    (handle, workspace, closes)
}
fn count(workspace: &Entity<Workspace>, cx: &App) -> usize {
    workspace
        .read(cx)
        .items
        .iter()
        .filter(|open| open.local.is_some())
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
        assert!(workspace.read(cx).dock.read(cx).zoomed_group().is_some());
        window.click("local-terminal-new", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(count(&workspace, cx), 2);
        assert!(workspace.read(cx).dock.read(cx).zoomed_group().is_some());
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
        assert!(workspace.read(cx).dock.read(cx).zoomed_group().is_none());
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
            let first = workspace
                .local_entry(a.entity_id())
                .unwrap()
                .dock_item
                .clone();
            let second = workspace
                .local_entry(b.entity_id())
                .unwrap()
                .dock_item
                .clone();
            let first_id = PanelId::from(first.entity_id());
            let second_id = PanelId::from(second.entity_id());
            workspace.dock.update(cx, |dock, cx| {
                let node = dock
                    .layout(DockPlacement::Bottom)
                    .unwrap()
                    .find_panel_node(first_id)
                    .unwrap();
                dock.move_panel(
                    second_id,
                    InsertTarget::Tabs {
                        node,
                        ix: Some(0),
                        activate: false,
                    },
                    window,
                    cx,
                );
                dock.select_panel(first_id, window, cx);
                dock.set_dock_size(DockPlacement::Bottom, px(260.), window, cx);
            });
            window.focus(&a.read(cx).focus_handle(cx), cx);
        });
        window.render_frame(cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        workspace.update(cx, |workspace, cx| {
            assert_eq!(workspace.local_terminal_cwd(cx), Some("/a".into()));
            workspace.execute_item_command(ItemCommand::Copy, window, cx);
            workspace
                .change_local_directory("/changed".into(), window, cx)
                .unwrap();
            workspace.toggle_local_terminal(window, cx);
            assert_eq!(workspace.selected_local_id, Some(a.entity_id()));
            let dock = workspace.dock.read(cx);
            let tree = dock.layout(DockPlacement::Bottom).unwrap();
            let node = tree
                .find_panel_node(PanelId::from(
                    workspace
                        .local_entry(a.entity_id())
                        .unwrap()
                        .dock_item
                        .entity_id(),
                ))
                .unwrap();
            let PaneRef::Tabs { panels, .. } = tree.find_node(node).unwrap().kind() else {
                panic!("expected retained pane");
            };
            assert_eq!(
                panels,
                &[
                    PanelId::from(
                        workspace
                            .local_entry(b.entity_id())
                            .unwrap()
                            .dock_item
                            .entity_id()
                    ),
                    PanelId::from(
                        workspace
                            .local_entry(a.entity_id())
                            .unwrap()
                            .dock_item
                            .entity_id()
                    )
                ]
            );
            workspace.toggle_local_terminal(window, cx);
            assert_eq!(
                workspace.dock.read(cx).dock_size(DockPlacement::Bottom),
                Some(px(260.))
            );
            assert_eq!(workspace.local_terminal_cwd(cx), Some("/changed".into()));
            workspace.close_item_by_id(b.entity_id(), window, cx);
            assert_eq!(
                workspace.selected_local(cx).unwrap().handle.item_id(),
                a.entity_id()
            );
            assert_eq!(workspace.local_terminal_cwd(cx), Some("/changed".into()));
            workspace.close_local_terminal(window, cx);
            assert!(workspace.selected_local_id.is_none());
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
                workspace.selected_local_id.unwrap()
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
        workspace.read_with(cx, |workspace, _| workspace
            .items
            .iter()
            .filter(|open| open.local.is_some())
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
        workspace.read_with(cx, |workspace, _| workspace
            .items
            .iter()
            .filter(|open| open.local.is_some())
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
                let first = workspace.selected_local_id.unwrap();
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
                .items
                .iter()
                .filter(|open| open.local.is_some())
                .count(),
            2
        );
        let panel = PanelId::from(workspace.local_entry(first).unwrap().dock_item.entity_id());
        let tree = workspace
            .dock
            .read(cx)
            .layout(DockPlacement::Bottom)
            .unwrap();
        let node = tree
            .find_node(tree.find_panel_node(panel).unwrap())
            .unwrap();
        let PaneRef::Tabs { panels, .. } = node.kind() else {
            panic!("local tabs must remain grouped");
        };
        assert_eq!(panels.last(), Some(&panel));
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
                let panel = PanelId::from(
                    workspace
                        .local_entry(b.entity_id())
                        .unwrap()
                        .dock_item
                        .entity_id(),
                );
                workspace.dock.update(cx, |dock, cx| {
                    let node = dock
                        .layout(DockPlacement::Bottom)
                        .unwrap()
                        .find_panel_node(panel)
                        .unwrap();
                    dock.move_panel(
                        panel,
                        InsertTarget::Split {
                            node,
                            placement: gpui_kit::component::Placement::Right,
                            size: None,
                        },
                        window,
                        cx,
                    );
                });
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
            let b_panel = PanelId::from(
                workspace
                    .local_entry(b.entity_id())
                    .unwrap()
                    .dock_item
                    .entity_id(),
            );
            let node = workspace
                .dock
                .read(cx)
                .layout(DockPlacement::Bottom)
                .unwrap()
                .find_panel_node(b_panel)
                .unwrap();
            workspace.new_local_terminal(window, cx);
            let new_panel = PanelId::from(
                workspace
                    .items
                    .iter()
                    .rfind(|open| open.local.is_some())
                    .unwrap()
                    .dock_item
                    .entity_id(),
            );
            assert_eq!(
                workspace
                    .dock
                    .read(cx)
                    .layout(DockPlacement::Bottom)
                    .unwrap()
                    .find_panel_node(new_panel),
                Some(node)
            );
            let item = workspace
                .local_entry(b.entity_id())
                .unwrap()
                .dock_item
                .clone();
            workspace
                .dock
                .update(cx, |dock, cx| dock.remove_panel(item, window, cx));
        });
        window.render_frame(cx);
    })
    .unwrap();
    cx.run_until_parked();
    assert_eq!(closes.get(), 1);
    workspace.read_with(cx, |workspace, _| {
        assert!(workspace.local_entry(a.entity_id()).is_some());
        assert!(workspace.local_entry(b.entity_id()).is_none());
        assert_eq!(
            workspace
                .items
                .iter()
                .filter(|open| open.local.is_some())
                .count(),
            2
        );
    });
}
