use crate::{Item, ItemEvent, LocalTerminal, Workspace};
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{
    AnyWindowHandle, App, Context, Entity, EventEmitter, FocusHandle, Focusable, TestAppContext,
    Window, WindowOptions,
    component::{
        Placement,
        dock::{DockPlacement, PaneRef},
    },
    div,
    prelude::*,
};
use std::{cell::Cell, path::PathBuf, rc::Rc};

struct Probe {
    focus: FocusHandle,
    closes: Rc<Cell<usize>>,
    target: Option<nocterm_session::Target>,
    connected: bool,
    fs: Option<std::sync::Arc<dyn nocterm_session::RemoteFs>>,
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
    fn tab_title(&self, _: &App) -> gpui_kit::SharedString {
        "Original".into()
    }
    fn session(&self, _: &App) -> Option<crate::SessionContext> {
        self.target.clone().map(|target| crate::SessionContext {
            target,
            connected: self.connected,
            fs: self.fs.clone(),
        })
    }
    fn on_close(&mut self, _: &mut Window, _: &mut Context<Self>) {
        self.closes.set(self.closes.get() + 1);
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
fn fixture(cx: &mut TestAppContext) -> (AnyWindowHandle, Entity<Workspace>) {
    cx.update(|cx| {
        gpui_kit::init(cx);
        nocterm_ui::init(
            nocterm_ui::DesignTokens::builtin(),
            nocterm_ui::SettingsStore::in_memory(nocterm_settings::Settings::default()),
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
        closes,
        target: None,
        connected: true,
        fs: None,
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
            workspace.set_local_terminal(local, window, cx);
            assert_eq!(
                workspace.active_item().unwrap().item_id(),
                central.entity_id()
            );
            assert_eq!(
                workspace.local_terminal_cwd(cx),
                Some(PathBuf::from("/tmp"))
            );
            workspace.toggle_local_terminal(window, cx);
            workspace.toggle_local_terminal(window, cx);
            assert_eq!(closes.get(), 0);
            workspace.close_local_terminal(window, cx);
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
