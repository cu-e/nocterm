use crate::{Item, ItemEvent, LocalTerminal, Workspace};
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{
    AnyWindowHandle, App, Context, Entity, EventEmitter, FocusHandle, Focusable, TestAppContext,
    TestSupportExt as _, Window, WindowOptions,
    component::{
        Placement, WindowExt as _,
        dock::{DockPlacement, PaneRef},
        floating::FloatingCards,
    },
    div,
    prelude::*,
    px,
};
use std::{
    cell::{Cell, RefCell},
    path::PathBuf,
    rc::Rc,
};

struct Probe {
    focus: FocusHandle,
    alternate_focus: Option<FocusHandle>,
    closes: Rc<Cell<usize>>,
    target: Option<nocterm_session::Target>,
    connected: bool,
    fs: Option<std::sync::Arc<dyn nocterm_session::RemoteFs>>,
    commands: Vec<crate::ItemCommand>,
    executed: Rc<RefCell<Vec<crate::ItemCommand>>>,
}
impl EventEmitter<ItemEvent> for Probe {}
impl Focusable for Probe {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}
impl Render for Probe {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        match &self.alternate_focus {
            Some(alternate) => div()
                .size_full()
                .child(div().h_1_2().track_focus(&self.focus))
                .child(div().h_1_2().track_focus(alternate))
                .into_any_element(),
            None => div()
                .size_full()
                .track_focus(&self.focus)
                .into_any_element(),
        }
    }
}
impl Item for Probe {
    fn tab_title(&self, _: &App) -> gpui_kit::SharedString {
        "Original".into()
    }
    fn session(&self, _: &App) -> Option<crate::SessionContext> {
        self.target
            .clone()
            .map(|target| crate::SessionContext::new(target, self.fs.clone(), self.connected))
    }
    fn on_close(&mut self, _: &mut Window, _: &mut Context<Self>) {
        self.closes.set(self.closes.get() + 1);
    }
    fn command_enabled(&self, command: crate::ItemCommand, _: &App) -> bool {
        self.commands.contains(&command)
    }
    fn execute(&mut self, command: crate::ItemCommand, _: &mut Window, _: &mut Context<Self>) {
        self.executed.borrow_mut().push(command);
    }
}
impl LocalTerminal for Probe {
    fn cwd(&self, _: &App) -> Option<PathBuf> {
        Some(PathBuf::from("/tmp"))
    }
    fn change_directory(
        &mut self,
        _: PathBuf,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Result<(), String> {
        Ok(())
    }
}
pub(super) fn fixture(cx: &mut TestAppContext) -> (AnyWindowHandle, Entity<Workspace>) {
    cx.update(|cx| {
        gpui_kit::init(cx);
        nocterm_ui::init(
            nocterm_ui::DesignTokens::builtin(),
            nocterm_ui::SettingsStore::in_memory(nocterm_settings::SettingsDocument::default()),
            cx,
        );
        let (window, workspace) =
            gpui_kit::open_window(WindowOptions::default(), cx, |window, cx| {
                cx.new(|cx| Workspace::new(window, cx))
            })
            .unwrap();
        window
            .update(cx, |_, window, _| window.activate_window())
            .unwrap();
        (window, workspace)
    })
}
fn probe(cx: &mut App, closes: Rc<Cell<usize>>) -> Entity<Probe> {
    cx.new(|cx| Probe {
        focus: cx.focus_handle(),
        alternate_focus: None,
        closes,
        target: None,
        connected: true,
        fs: None,
        commands: vec![crate::ItemCommand::Copy, crate::ItemCommand::Find],
        executed: Rc::new(RefCell::new(Vec::new())),
    })
}

fn groups(workspace: &Workspace, cx: &App) -> usize {
    let dock = workspace.dock.read(cx);
    let tree = dock.layout(DockPlacement::Center).unwrap();
    tree.node_ids().iter().filter(|id| matches!(tree.find_node(**id).unwrap().kind(), PaneRef::Tabs { panels, .. } if !panels.is_empty())).count()
}

#[gpui_kit::test]
fn moves_splits_and_pane_collapse_preserve_item_identity(cx: &mut TestAppContext) {
    let (window, workspace) = fixture(cx);
    let closes = Rc::new(Cell::new(0));
    cx.update_window(window, |_, window, cx| {
        let a = probe(cx, closes.clone());
        let b = probe(cx, closes.clone());
        workspace.update(cx, |workspace, cx| {
            workspace.add_item(a.clone(), window, cx);
            workspace.add_item(b.clone(), window, cx);
            workspace.move_active_tab(-1, window, cx);
            assert_eq!(workspace.active_item().unwrap().item_id(), b.entity_id());
            workspace.split_active(Placement::Right, window, cx);
            assert_eq!(groups(workspace, cx), 2);
            workspace.focus_pane(-1, window, cx);
            assert_eq!(workspace.active_item().unwrap().item_id(), a.entity_id());
            workspace.close_item(0, window, cx);
            assert_eq!(groups(workspace, cx), 1);
            assert_eq!(workspace.active_item().unwrap().item_id(), b.entity_id());
            assert_eq!(closes.get(), 1);
        });
        window.render_frame(cx);
    })
    .unwrap();
    cx.run_until_parked();
    assert_eq!(
        closes.get(),
        1,
        "deferred toolkit removal must not close the Item twice"
    );
}

#[gpui_kit::test]
fn local_hide_show_preserves_process_and_central_focus_context(cx: &mut TestAppContext) {
    let (window, workspace) = fixture(cx);
    let closes = Rc::new(Cell::new(0));
    cx.update_window(window, |_, window, cx| {
        let central = probe(cx, closes.clone());
        let local = probe(cx, closes.clone());
        workspace.update(cx, |workspace, cx| {
            workspace.add_item(central.clone(), window, cx);
            workspace.set_local_terminal(local.clone(), window, cx);
            assert_eq!(
                workspace.active_item().unwrap().item_id(),
                central.entity_id()
            );
            assert_eq!(
                workspace.local_terminal_cwd(cx),
                Some(PathBuf::from("/tmp"))
            );
            workspace.toggle_local_terminal(window, cx);
            assert!(workspace.dock.read(cx).has_dock(DockPlacement::Bottom));
            assert!(!workspace.local_terminal_is_visible(cx));
            assert_eq!(closes.get(), 0);
            assert_eq!(
                workspace.selected_local(cx).unwrap().handle.item_id(),
                local.entity_id()
            );
            workspace.toggle_local_terminal(window, cx);
            assert!(workspace.local_terminal_is_visible(cx));
            assert_eq!(
                workspace.selected_local(cx).unwrap().handle.item_id(),
                local.entity_id()
            );
            assert_eq!(closes.get(), 0);
            workspace.close_local_terminal(window, cx);
            assert!(!workspace.dock.read(cx).has_dock(DockPlacement::Bottom));
            assert_eq!(closes.get(), 1);
            assert_eq!(
                workspace.active_item().unwrap().item_id(),
                central.entity_id()
            );
        });
        window.render_frame(cx);
    })
    .unwrap();
    cx.run_until_parked();
    assert_eq!(closes.get(), 1);
}

#[gpui_kit::test]
fn hidden_local_terminal_keeps_height_and_closes_exactly_once(cx: &mut TestAppContext) {
    let (window, workspace) = fixture(cx);
    let closes = Rc::new(Cell::new(0));
    cx.update_window(window, |_, window, cx| {
        let local = probe(cx, closes.clone());
        workspace.update(cx, |workspace, cx| {
            workspace.set_local_terminal(local.clone(), window, cx);
            workspace.dock.update(cx, |dock, cx| {
                dock.set_dock_size(DockPlacement::Bottom, px(260.), window, cx);
            });
            workspace.toggle_local_terminal(window, cx);
            assert!(workspace.dock.read(cx).has_dock(DockPlacement::Bottom));
            assert_eq!(
                workspace.local_terminal_cwd(cx),
                Some(PathBuf::from("/tmp"))
            );
            workspace.toggle_local_terminal(window, cx);
            assert_eq!(
                workspace.dock.read(cx).dock_size(DockPlacement::Bottom),
                Some(px(260.))
            );
            workspace.toggle_local_terminal(window, cx);
            workspace.close_local_terminal(window, cx);
            assert_eq!(closes.get(), 1);
            assert!(workspace.selected_local_id.is_none());
            assert!(!workspace.dock.read(cx).has_dock(DockPlacement::Bottom));
        });
        window.render_frame(cx);
    })
    .unwrap();
    cx.run_until_parked();
    assert_eq!(closes.get(), 1);
}

#[gpui_kit::test]
fn close_scopes_follow_reordered_pane_tabs_and_do_not_close_other_panes(cx: &mut TestAppContext) {
    use crate::TabCloseScope;
    let (window, workspace) = fixture(cx);
    let closes = Rc::new(Cell::new(0));
    cx.update_window(window, |_, window, cx| {
        let a = probe(cx, closes.clone());
        let b = probe(cx, closes.clone());
        let c = probe(cx, closes.clone());
        let isolated = probe(cx, closes.clone());
        workspace.update(cx, |workspace, cx| {
            for item in [&a, &b, &c, &isolated] {
                workspace.add_item(item.clone(), window, cx);
            }
            workspace.split_active(Placement::Right, window, cx);
            workspace.activate_item_by_id(c.entity_id(), window, cx);
            workspace.move_active_tab(-1, window, cx); // a, c, b
            workspace.activate_item_by_id(isolated.entity_id(), window, cx);
            assert_eq!(
                workspace.tabs_to_close(c.entity_id(), TabCloseScope::Left, cx),
                [a.entity_id()]
            );
            assert_eq!(
                workspace.tabs_to_close(c.entity_id(), TabCloseScope::Right, cx),
                [b.entity_id()]
            );
            assert_eq!(
                workspace.tabs_to_close(c.entity_id(), TabCloseScope::Others, cx),
                [a.entity_id(), b.entity_id()]
            );
            workspace.close_tabs(c.entity_id(), TabCloseScope::Others, window, cx);
            assert_eq!(
                workspace.active_item().unwrap().item_id(),
                isolated.entity_id()
            );
            assert_eq!(workspace.items.len(), 2);
            assert_eq!(groups(workspace, cx), 2);
            assert_eq!(closes.get(), 2);
            let local = probe(cx, Rc::new(Cell::new(0)));
            workspace.set_local_terminal(local, window, cx);
            workspace.close_tabs(c.entity_id(), TabCloseScope::All, window, cx);
            assert!(workspace.items().next().is_none());
            assert!(workspace.selected_local_id.is_some());
            assert!(workspace.dock.read(cx).has_dock(DockPlacement::Bottom));
            assert_eq!(closes.get(), 4);
        });
        window.render_frame(cx);
    })
    .unwrap();
    cx.run_until_parked();
    assert_eq!(
        closes.get(),
        4,
        "native deferred removal must not close twice"
    );
}

#[gpui_kit::test]
fn footer_spans_the_window_when_sidebar_is_hidden(cx: &mut TestAppContext) {
    let (handle, workspace) = fixture(cx);
    cx.update_window(handle, |_, window, cx| {
        workspace.update(cx, |workspace, cx| {
            workspace.sidebar_open = false;
            cx.notify();
        });
        window.render_frame(cx);
        let settings = window.find("toggle-local-terminal").bounds();
        assert!(window.viewport_size().width - settings.right() < gpui_kit::px(16.));
        assert!(window.try_find("toggle-local-terminal").is_some());
        assert!(window.try_find("open-settings").is_none());
    })
    .unwrap();
}

#[gpui_kit::test]
fn tab_context_menu_closes_clicked_inactive_tab_and_not_active_tab(cx: &mut TestAppContext) {
    let (handle, workspace) = fixture(cx);
    let closes = Rc::new(Cell::new(0));
    let (a, b) = cx
        .update_window(handle, |_, window, cx| {
            let a = probe(cx, closes.clone());
            let b = probe(cx, closes.clone());
            workspace.update(cx, |workspace, cx| {
                workspace.add_item(a.clone(), window, cx);
                workspace.add_item(b.clone(), window, cx);
            });
            window.render_frame(cx);
            window.right_click(("tab-title", a.entity_id().as_u64()), cx);
            (a, b)
        })
        .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("popup-menu").is_some());
        window.press("down", cx);
        window.press("enter", cx);
    })
    .unwrap();
    cx.run_until_parked();
    workspace.read_with(cx, |workspace, _| {
        assert_eq!(workspace.items.len(), 1);
        assert_eq!(workspace.active_item().unwrap().item_id(), b.entity_id());
        assert_ne!(workspace.active_item().unwrap().item_id(), a.entity_id());
    });
    assert_eq!(closes.get(), 1);
}

#[gpui_kit::test]
fn tab_context_menu_splits_clicked_tab_without_recreating_it(cx: &mut TestAppContext) {
    let (handle, workspace) = fixture(cx);
    let closes = Rc::new(Cell::new(0));
    let clicked = cx
        .update_window(handle, |_, window, cx| {
            let a = probe(cx, closes.clone());
            let b = probe(cx, closes.clone());
            let c = probe(cx, closes.clone());
            workspace.update(cx, |workspace, cx| {
                for item in [a, b.clone(), c] {
                    workspace.add_item(item, window, cx);
                }
            });
            window.render_frame(cx);
            window.right_click(("tab-title", b.entity_id().as_u64()), cx);
            b
        })
        .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.within("popup-menu").click(6usize, cx);
    })
    .unwrap();
    cx.run_until_parked();
    workspace.read_with(cx, |workspace, cx| {
        assert_eq!(groups(workspace, cx), 2);
        assert_eq!(workspace.items.len(), 3);
        assert!(
            workspace
                .items
                .iter()
                .any(|item| item.handle.item_id() == clicked.entity_id())
        );
    });
    assert_eq!(closes.get(), 0);
}

#[gpui_kit::test]
fn aliases_accept_cancel_and_empty_restore_without_renaming_items(cx: &mut TestAppContext) {
    let (window, workspace) = fixture(cx);
    let closes = Rc::new(Cell::new(0));
    cx.update_window(window, |_, window, cx| {
        let item = probe(cx, closes.clone());
        workspace.update(cx, |workspace, cx| {
            workspace.add_item(item, window, cx);
            workspace.rename_active_tab(window, cx);
        });
        window.render_frame(cx);
        window.press("ctrl-a", cx);
        window.input("Production", cx);
        window.press("enter", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(window, |_, window, cx| {
        workspace.update(cx, |workspace, cx| {
            assert_eq!(
                workspace.items[0].dock_item.read(cx).alias.as_deref(),
                Some("Production")
            );
            assert_eq!(workspace.items[0].handle.tab_title(cx).as_ref(), "Original");
            workspace.rename_active_tab(window, cx);
        });
        window.render_frame(cx);
        window.press("ctrl-a", cx);
        window.input("Discard", cx);
        window.press("escape", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(window, |_, window, cx| {
        workspace.update(cx, |workspace, cx| {
            assert_eq!(
                workspace.items[0].dock_item.read(cx).alias.as_deref(),
                Some("Production")
            );
            workspace.rename_active_tab(window, cx);
        });
        window.render_frame(cx);
        window.press("ctrl-a", cx);
        window.press("backspace", cx);
        window.press("enter", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(window, |_, _, cx| {
        assert!(
            workspace.read(cx).items[0]
                .dock_item
                .read(cx)
                .alias
                .is_none()
        );
    })
    .unwrap();
    assert_eq!(closes.get(), 0);
}

#[gpui_kit::test]
fn focused_split_content_changes_active_remote_but_bottom_keeps_it(cx: &mut TestAppContext) {
    let (window, workspace) = fixture(cx);
    let closes = Rc::new(Cell::new(0));
    let (a, b) = cx
        .update_window(window, |_, window, cx| {
            let a = probe(cx, closes.clone());
            let b = probe(cx, closes.clone());
            a.update(cx, |item, _| {
                item.target = Some(nocterm_session::Target::new("user", "a.test", 22))
            });
            b.update(cx, |item, _| {
                item.target = Some(nocterm_session::Target::new("user", "b.test", 22))
            });
            workspace.update(cx, |workspace, cx| {
                workspace.add_item(a.clone(), window, cx);
                workspace.add_item(b.clone(), window, cx);
            });
            window.render_frame(cx);
            workspace.update(cx, |workspace, cx| {
                workspace.split_active(Placement::Bottom, window, cx)
            });
            window.render_frame(cx);
            (a, b)
        })
        .unwrap();
    cx.run_until_parked();
    cx.update_window(window, |_, window, cx| {
        window.focus(&a.read(cx).focus_handle(cx), cx);
        assert!(a.read(cx).focus_handle(cx).is_focused(window));
        window.render_frame(cx);
        assert!(a.read(cx).focus_handle(cx).is_focused(window));
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(window, |_, window, cx| {
        assert_eq!(
            workspace.read(cx).active_session(cx).unwrap().target.host,
            "a.test"
        );
        window.focus(&b.read(cx).focus_handle(cx), cx);
        window.render_frame(cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(window, |_, window, cx| {
        assert_eq!(
            workspace.read(cx).active_session(cx).unwrap().target.host,
            "b.test"
        );
        let local = probe(cx, closes.clone());
        workspace.update(cx, |workspace, cx| {
            workspace.set_local_terminal(local, window, cx)
        });
        window.render_frame(cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(window, |_, _, cx| {
        assert_eq!(
            workspace.read(cx).active_session(cx).unwrap().target.host,
            "b.test"
        )
    })
    .unwrap();
}

#[gpui_kit::test]
fn alternate_child_focus_tracks_split_and_bottom_command_ownership(cx: &mut TestAppContext) {
    let (window, workspace) = fixture(cx);
    let (a, b, local) = cx
        .update_window(window, |_, window, cx| {
            let make = |cx: &mut App| {
                let item = probe(cx, Rc::new(Cell::new(0)));
                item.update(cx, |item, cx| {
                    item.alternate_focus = Some(cx.focus_handle());
                });
                item
            };
            let a = make(cx);
            let b = make(cx);
            let local = make(cx);
            workspace.update(cx, |workspace, cx| {
                workspace.add_item(a.clone(), window, cx);
                workspace.add_item(b.clone(), window, cx);
                workspace.split_active(Placement::Right, window, cx);
                workspace.set_local_terminal(local.clone(), window, cx);
            });
            window.render_frame(cx);
            (a, b, local)
        })
        .unwrap();
    cx.run_until_parked();
    for item in [&a, &b, &local] {
        cx.update_window(window, |_, window, cx| {
            let alternate = item.read(cx).alternate_focus.as_ref().unwrap().clone();
            window.focus(&alternate, cx);
            assert!(!item.read(cx).focus.contains_focused(window, cx));
            window.render_frame(cx);
        })
        .unwrap();
        cx.run_until_parked();
        cx.update_window(window, |_, window, cx| {
            workspace.update(cx, |workspace, cx| {
                assert_eq!(
                    workspace.command_item(window, cx).unwrap().item_id(),
                    item.entity_id(),
                    "a sibling of the preferred control still belongs to its Item"
                );
                let expected_central = if item == &local { &b } else { item };
                assert_eq!(
                    workspace.active_item().unwrap().item_id(),
                    expected_central.entity_id(),
                    "bottom focus must preserve the active central pane"
                );
                workspace.execute_item_command(crate::ItemCommand::Copy, window, cx);
            });
            assert_eq!(*item.read(cx).executed.borrow(), [crate::ItemCommand::Copy]);
        })
        .unwrap();
    }
}

struct MarkerFs;
impl nocterm_session::RemoteFs for MarkerFs {
    fn home(&self) -> nocterm_session::FsFuture<String> {
        Box::pin(async { Ok("/home".into()) })
    }
    fn read_dir(&self, _: &str) -> nocterm_session::FsFuture<Vec<nocterm_session::DirEntry>> {
        Box::pin(async { Ok(Vec::new()) })
    }
}

#[gpui_kit::test]
#[expect(clippy::too_many_lines, reason = "predates the limit")]
fn explicit_target_snapshot_survives_utility_tab_and_inactive_disconnect(cx: &mut TestAppContext) {
    use std::sync::Arc;
    let (window, workspace) = fixture(cx);
    let target = nocterm_session::Target::new("user", "same.test", 22);
    let first_fs: Arc<dyn nocterm_session::RemoteFs> = Arc::new(MarkerFs);
    let second_fs: Arc<dyn nocterm_session::RemoteFs> = Arc::new(MarkerFs);
    let (first, second, utility) = cx
        .update_window(window, |_, window, cx| {
            let first = probe(cx, Rc::new(Cell::new(0)));
            let second = probe(cx, Rc::new(Cell::new(0)));
            let utility = probe(cx, Rc::new(Cell::new(0)));
            first.update(cx, |item, _| {
                item.target = Some(target.clone());
                item.fs = Some(first_fs.clone());
            });
            second.update(cx, |item, _| {
                item.target = Some(target.clone());
                item.fs = Some(second_fs.clone());
            });
            workspace.update(cx, |workspace, cx| {
                workspace.add_item(first.clone(), window, cx);
                workspace.add_item(second.clone(), window, cx);
                let active = workspace.connected_session_for_target(&target).unwrap();
                assert!(
                    Arc::ptr_eq(active.fs.as_ref().unwrap(), &second_fs),
                    "active matching session wins"
                );
                workspace.add_item(utility.clone(), window, cx);
                assert!(workspace.active_session(cx).is_none());
                let fallback = workspace.connected_session_for_target(&target).unwrap();
                assert!(Arc::ptr_eq(fallback.fs.as_ref().unwrap(), &first_fs));
                assert!(
                    workspace
                        .connected_session_for_target(&nocterm_session::Target::new(
                            "other",
                            "same.test",
                            22
                        ))
                        .is_none()
                );
            });
            // A live Item holds its GPUI entity borrow, exactly as during rendering.
            first.update(cx, |_, cx| {
                assert!(
                    workspace
                        .read(cx)
                        .connected_session_for_target(&target)
                        .is_some()
                )
            });
            (first, second, utility)
        })
        .unwrap();
    cx.update_window(window, |_, _, cx| {
        first.update(cx, |item, cx| {
            item.connected = false;
            cx.emit(ItemEvent::Changed);
        })
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(window, |_, _, cx| {
        let session = workspace
            .read(cx)
            .connected_session_for_target(&target)
            .unwrap();
        assert!(
            Arc::ptr_eq(session.fs.as_ref().unwrap(), &second_fs),
            "inactive disconnected snapshot is rejected"
        );
        assert!(workspace.read(cx).active_session(cx).is_none());
        second.update(cx, |item, cx| {
            item.connected = false;
            cx.emit(ItemEvent::Changed);
        });
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(window, |_, window, cx| {
        assert!(
            workspace
                .read(cx)
                .connected_session_for_target(&target)
                .is_none()
        );
        workspace.update(cx, |workspace, cx| {
            workspace.activate_item_by_id(first.entity_id(), window, cx)
        });
        assert!(!workspace.read(cx).active_session(cx).unwrap().connected);
        first.update(cx, |item, cx| {
            item.connected = true;
            cx.emit(ItemEvent::Changed);
        });
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(window, |_, window, cx| {
        assert!(workspace.read(cx).active_session(cx).unwrap().connected);
        workspace.update(cx, |workspace, cx| {
            workspace.activate_item_by_id(utility.entity_id(), window, cx);
            workspace.close_item(0, window, cx);
        });
        assert!(
            workspace
                .read(cx)
                .connected_session_for_target(&target)
                .is_none(),
            "closed Item removes its cached connected session"
        );
    })
    .unwrap();
}

pub(super) struct RightProbe {
    pub(super) focus: FocusHandle,
    pub(super) maximized: bool,
}
impl EventEmitter<crate::RightPanelEvent> for RightProbe {}
impl Focusable for RightProbe {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}
impl Render for RightProbe {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .id("right-probe")
            .test_support()
            .size_full()
            .track_focus(&self.focus)
            .child("AI")
    }
}
impl crate::Panel for RightProbe {
    fn title(&self, _: &App) -> gpui_kit::SharedString {
        "AI Agents".into()
    }
    fn icon(&self, _: &App) -> nocterm_ui::IconName {
        nocterm_ui::IconName::Bot
    }
}
impl crate::RightPanel for RightProbe {
    fn set_maximized(&mut self, maximized: bool, cx: &mut Context<Self>) {
        self.maximized = maximized;
        cx.notify();
    }
}

#[gpui_kit::test]
fn independent_right_panel_supports_empty_workspace_maximize_and_disable(cx: &mut TestAppContext) {
    let (handle, workspace) = fixture(cx);
    cx.update_window(handle, |_, window, cx| {
        window.resize(gpui_kit::size(px(1800.), px(760.)));
        let panel = cx.new(|cx| RightProbe {
            focus: cx.focus_handle(),
            maximized: false,
        });
        workspace.update(cx, |workspace, cx| {
            workspace.set_right_panel(panel.clone(), window, cx);
            workspace.toggle_right_panel(window, cx);
            assert!(!workspace.right_panel_is_open());
            workspace.set_right_panel_available(true, window, cx);
            workspace.toggle_right_panel(window, cx);
            assert!(workspace.right_panel_is_open());
            assert!(panel.read(cx).focus.is_focused(window));
        });
        window.render_frame(cx);
        assert!(window.try_find("right-probe").is_some());
        assert!(window.try_find("empty-new-tab").is_some());
        assert!(window.try_find("open-settings").is_none());
        let toggle = window.find("toggle-right-panel").bounds();
        assert!(toggle.left() > window.find("toggle-local-terminal").bounds().right());
        workspace.update(cx, |workspace, cx| {
            workspace.set_right_panel_maximized(true, cx)
        });
        window.render_frame(cx);
        assert!(window.try_find("empty-new-tab").is_none());
        assert!(window.try_find("toggle-right-panel").is_some());
        assert!(panel.read(cx).maximized);
        let inset = FloatingCards::get(cx).map_or(px(0.), |cards| cards.gap * 2. + px(2.));
        let probe = window.find("right-probe").bounds().size.width;
        assert!((probe - (window.viewport_size().width - inset)).abs() < px(2.));
        workspace.update(cx, |workspace, cx| {
            workspace.set_right_panel_available(false, window, cx)
        });
        window.render_frame(cx);
        assert!(window.try_find("toggle-right-panel").is_none());
        assert!(window.try_find("right-probe").is_none());
        assert!(window.try_find("empty-new-tab").is_some());
        assert!(!panel.read(cx).maximized);
        assert!(workspace.read(cx).focus_handle.is_focused(window));
    })
    .unwrap();
}

#[path = "menu_tests.rs"]
mod menus;
