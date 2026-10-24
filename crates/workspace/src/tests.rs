use crate::test_support::{Toggle, toggle_id};
use crate::{Item, ItemEvent, LocalTerminal, Workspace};
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{
    AnyWindowHandle, App, Context, Entity, EventEmitter, FocusHandle, Focusable, TestAppContext,
    Window, WindowOptions,
    component::{Placement, WindowExt as _, dock::DockPlacement},
    div,
    prelude::*,
    px,
};
use std::{
    cell::{Cell, RefCell},
    path::PathBuf,
    rc::Rc,
};
#[path = "tests/local_directory.rs"]
mod local_directory;
#[path = "tests/project_tabs.rs"]
mod project_tabs;
#[path = "tests/provider_resolution.rs"]
mod provider_resolution;
#[path = "tests/test_plugin.rs"]
pub(super) mod test_plugin;

pub(super) struct Probe {
    focus: FocusHandle,
    alternate_focus: Option<FocusHandle>,
    closes: Rc<Cell<usize>>,
    target: Option<nocterm_session::Target>,
    connected: bool,
    fs: Option<std::sync::Arc<dyn nocterm_session::RemoteFs>>,
    commands: Vec<Command>,
    executed: Rc<RefCell<Vec<Command>>>,
}

/// The commands a [`Probe`] answers while they are in its `commands`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Command {
    Copy,
    Paste,
    Find,
}

fn record<A: gpui_kit::Action>(
    command: Command,
) -> impl Fn(&mut Probe, &A, &mut Window, &mut Context<Probe>) + 'static {
    move |probe, _, _, _| probe.executed.borrow_mut().push(command)
}

impl Probe {
    fn on_commands(&self, element: gpui_kit::Div, cx: &mut Context<Self>) -> gpui_kit::Div {
        let has = |command| self.commands.contains(&command);
        element
            .when(has(Command::Copy), |el| {
                el.on_action(cx.listener(record::<gpui_kit::component::input::Copy>(Command::Copy)))
            })
            .when(has(Command::Paste), |el| {
                el.on_action(
                    cx.listener(record::<gpui_kit::component::input::Paste>(Command::Paste)),
                )
            })
            .when(has(Command::Find), |el| {
                el.on_action(cx.listener(record::<crate::Find>(Command::Find)))
            })
    }
}
impl EventEmitter<ItemEvent> for Probe {}
impl Focusable for Probe {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}
impl Render for Probe {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        match &self.alternate_focus {
            Some(alternate) => self
                .on_commands(div(), cx)
                .size_full()
                .child(div().h_1_2().track_focus(&self.focus))
                .child(div().h_1_2().track_focus(alternate))
                .into_any_element(),
            None => self
                .on_commands(div(), cx)
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
    fn on_close(&mut self, _: &mut Window, cx: &mut Context<Self>) {
        self.closes.set(self.closes.get() + 1);
        cx.emit(ItemEvent::Changed);
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
    ) -> Result<(), crate::LocalDirectoryError> {
        Ok(())
    }
}
pub(super) fn fixture(cx: &mut TestAppContext) -> (AnyWindowHandle, Entity<Workspace>) {
    let (window, workspace, _) = fixture_with_layout(cx);
    (window, workspace)
}
pub(super) fn fixture_with_layout(
    cx: &mut TestAppContext,
) -> (AnyWindowHandle, Entity<Workspace>, nocterm_ui::LayoutHandle) {
    fixture_with_deps(cx, |layout, _| crate::Deps::new(layout))
}
/// A window composed with the dependencies `deps` builds from the shared layout memory.
pub(super) fn fixture_with_deps(
    cx: &mut TestAppContext,
    deps: impl FnOnce(nocterm_ui::LayoutHandle, &mut App) -> crate::Deps,
) -> (AnyWindowHandle, Entity<Workspace>, nocterm_ui::LayoutHandle) {
    cx.update(|cx| {
        gpui_kit::init(cx);
        nocterm_ui::init(
            nocterm_ui::DesignTokens::builtin(),
            (nocterm_store::SettingsConfig::in_memory(
                nocterm_settings::SettingsDocument::default(),
            ))
            .install(cx),
            cx,
        );
        let layout = nocterm_ui::LayoutHandle::in_memory(cx);
        let deps = deps(layout.clone(), cx);
        let (window, workspace) =
            gpui_kit::open_window(WindowOptions::default(), cx, |window, cx| {
                cx.new(|cx| Workspace::new(deps, window, cx))
            })
            .unwrap();
        window
            .update(cx, |_, window, _| window.activate_window())
            .unwrap();
        (window, workspace, layout)
    })
}
pub(super) fn probe(cx: &mut App, closes: Rc<Cell<usize>>) -> Entity<Probe> {
    cx.new(|cx| Probe {
        focus: cx.focus_handle(),
        alternate_focus: None,
        closes,
        target: None,
        connected: true,
        fs: None,
        commands: vec![Command::Copy, Command::Find],
        executed: Rc::new(RefCell::new(Vec::new())),
    })
}

fn groups(workspace: &Workspace, cx: &App) -> usize {
    workspace
        .tab_panes(DockPlacement::Center, cx)
        .iter()
        .filter(|pane| !pane.items().is_empty())
        .count()
}

/// Exercise native pointer dispatch with the shell's immutable size observation.
pub(super) fn resize_bottom(
    window: &mut Window,
    workspace: &Entity<Workspace>,
    height: gpui_kit::Pixels,
    cx: &mut App,
) {
    window.render_frame(cx);
    let current = workspace
        .read(cx)
        .dock_region_size(DockPlacement::Bottom, cx)
        .unwrap();
    let area = window.find("dock-area").bounds();
    let from = gpui_kit::point(area.center().x, area.bottom() - current + px(1.));
    let to = gpui_kit::point(from.x, area.bottom() - height);
    window.drag(from, to, cx);
    window.render_frame(cx);
}

pub(super) fn drag_tab(
    window: &mut Window,
    from: gpui_kit::EntityId,
    to: gpui_kit::EntityId,
    cx: &mut App,
) {
    window.render_frame(cx);
    window.drag_to(("tab-title", from.as_u64()), ("tab-title", to.as_u64()), cx);
    window.render_frame(cx);
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
            assert_eq!(workspace.active_item(cx).unwrap().item_id(), b.entity_id());
            workspace.split_active(Placement::Right, window, cx);
            assert_eq!(groups(workspace, cx), 2);
            workspace.focus_pane(-1, window, cx);
            assert_eq!(workspace.active_item(cx).unwrap().item_id(), a.entity_id());
            workspace.close_item(0, window, cx);
            assert_eq!(groups(workspace, cx), 1);
            assert_eq!(workspace.active_item(cx).unwrap().item_id(), b.entity_id());
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
                workspace.active_item(cx).unwrap().item_id(),
                central.entity_id()
            );
            assert_eq!(
                workspace.local_terminal_cwd(cx),
                Some(PathBuf::from("/tmp"))
            );
            workspace.toggle_local_terminal(window, cx);
            assert!(workspace.dock_region_present(DockPlacement::Bottom, cx));
            assert!(!workspace.local_terminal_is_visible(cx));
            assert_eq!(closes.get(), 0);
            assert_eq!(
                workspace.selected_local_terminal(cx).unwrap(),
                local.entity_id()
            );
            workspace.toggle_local_terminal(window, cx);
            assert!(workspace.local_terminal_is_visible(cx));
            assert_eq!(
                workspace.selected_local_terminal(cx).unwrap(),
                local.entity_id()
            );
            assert_eq!(closes.get(), 0);
            workspace.close_local_terminal(window, cx);
            assert!(!workspace.dock_region_present(DockPlacement::Bottom, cx));
            assert_eq!(closes.get(), 1);
            assert_eq!(
                workspace.active_item(cx).unwrap().item_id(),
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
        });
        resize_bottom(window, &workspace, px(260.), cx);
        workspace.update(cx, |workspace, cx| {
            workspace.toggle_local_terminal(window, cx);
            assert!(workspace.dock_region_present(DockPlacement::Bottom, cx));
            assert_eq!(
                workspace.local_terminal_cwd(cx),
                Some(PathBuf::from("/tmp"))
            );
            workspace.toggle_local_terminal(window, cx);
            assert_eq!(
                workspace.dock_region_size(DockPlacement::Bottom, cx),
                Some(px(260.))
            );
            workspace.toggle_local_terminal(window, cx);
            workspace.close_local_terminal(window, cx);
            assert_eq!(closes.get(), 1);
            assert!(workspace.selected_local_terminal(cx).is_none());
            assert!(!workspace.dock_region_present(DockPlacement::Bottom, cx));
        });
        window.render_frame(cx);
    })
    .unwrap();
    cx.run_until_parked();
    assert_eq!(closes.get(), 1);
}

#[path = "tests/close_scopes.rs"]
mod close_scopes;

#[gpui_kit::test]
fn footer_spans_the_window_when_sidebar_is_hidden(cx: &mut TestAppContext) {
    let (handle, workspace) = fixture(cx);
    cx.update_window(handle, |_, window, cx| {
        workspace.update(cx, |workspace, cx| {
            workspace.toggle_sidebar(window, cx);
        });
        window.render_frame(cx);
        let settings = window.find(toggle_id(Toggle::LocalTerminal)).bounds();
        assert!(window.viewport_size().width - settings.right() < gpui_kit::px(16.));
        assert!(window.try_find(toggle_id(Toggle::LocalTerminal)).is_some());
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
    workspace.read_with(cx, |workspace, cx| {
        assert_eq!(workspace.tabs(cx).len(), 1);
        assert_eq!(workspace.active_item(cx).unwrap().item_id(), b.entity_id());
        assert_ne!(workspace.active_item(cx).unwrap().item_id(), a.entity_id());
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
        assert_eq!(workspace.tabs(cx).len(), 3);
        assert!(
            workspace
                .tabs(cx)
                .iter()
                .any(|item| item.id() == clicked.entity_id())
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
            assert_eq!(workspace.tabs(cx)[0].alias(), Some("Production"));
            assert_eq!(
                workspace.items(cx).next().unwrap().tab_title(cx).as_ref(),
                "Original"
            );
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
            assert_eq!(workspace.tabs(cx)[0].alias(), Some("Production"));
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
        assert!(workspace.read(cx).tabs(cx)[0].alias().is_none());
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
                    workspace.active_item(cx).unwrap().item_id(),
                    expected_central.entity_id(),
                    "bottom focus must preserve the active central pane"
                );
                workspace.dispatch_command(Box::new(gpui_kit::component::input::Copy), window, cx);
            });
        })
        .unwrap();
        cx.run_until_parked();
        item.read_with(cx, |item, _| {
            assert_eq!(*item.executed.borrow(), [Command::Copy]);
        });
    }
}

pub(crate) struct MarkerFs;
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
    let host = nocterm_session::HostAddress::direct(target.clone());
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
                let active = workspace.connected_session_for_host(&host, cx).unwrap();
                assert!(
                    Arc::ptr_eq(active.fs.as_ref().unwrap(), &second_fs),
                    "active matching session wins"
                );
                workspace.add_item(utility.clone(), window, cx);
                assert!(workspace.active_session(cx).is_none());
                let fallback = workspace.connected_session_for_host(&host, cx).unwrap();
                assert!(Arc::ptr_eq(fallback.fs.as_ref().unwrap(), &first_fs));
                assert!(
                    workspace
                        .connected_session_for_host(
                            &nocterm_session::HostAddress::direct(nocterm_session::Target::new(
                                "other",
                                "same.test",
                                22
                            )),
                            cx
                        )
                        .is_none()
                );
            });
            // A live Item holds its GPUI entity borrow, exactly as during rendering.
            first.update(cx, |_, cx| {
                assert!(
                    workspace
                        .read(cx)
                        .connected_session_for_host(&host, cx)
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
            .connected_session_for_host(&host, cx)
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
                .connected_session_for_host(&host, cx)
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
                .connected_session_for_host(&host, cx)
                .is_none(),
            "closed Item removes its cached connected session"
        );
    })
    .unwrap();
}

#[path = "menu_tests.rs"]
mod menus;

#[gpui_kit::test]
fn close_hook_metadata_notification_cannot_republish_removed_membership(cx: &mut TestAppContext) {
    let (window, workspace) = fixture(cx);
    let changes = Rc::new(Cell::new(0));
    let closes = Rc::new(Cell::new(0));
    cx.update_window(window, |_, window, cx| {
        let item = probe(cx, closes.clone());
        workspace.update(cx, |workspace, cx| workspace.add_item(item, window, cx));
    })
    .unwrap();
    cx.run_until_parked();
    let counted = changes.clone();
    let _subscription = cx.update(|cx| {
        cx.subscribe(&workspace, move |_, event, _| {
            if matches!(event, crate::WorkspaceEvent::ItemsChanged) {
                counted.set(counted.get() + 1);
            }
        })
    });
    cx.update_window(window, |_, window, cx| {
        workspace.update(cx, |workspace, cx| workspace.close_item(0, window, cx));
    })
    .unwrap();
    cx.run_until_parked();
    assert_eq!(closes.get(), 1);
    assert_eq!(
        changes.get(),
        1,
        "only committed removal publishes membership"
    );
    cx.update(|cx| assert_eq!(workspace.read(cx).items(cx).count(), 0));
}
