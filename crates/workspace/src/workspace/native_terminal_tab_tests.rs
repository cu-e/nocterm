//! Independent native interaction checks shared by central and local terminal tabs.
mod deferred_open;

use super::*;
use gpui_kit::{
    AnyWindowHandle, TestAppContext, component::Placement, px, test::TestWindowExt as _,
};
use std::cell::Cell;

struct Tab {
    focus: FocusHandle,
    cwd: PathBuf,
    closed: Rc<Cell<usize>>,
    executed: Rc<Cell<usize>>,
    target: Option<Target>,
}
impl EventEmitter<ItemEvent> for Tab {}
impl Focusable for Tab {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}
impl Render for Tab {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div().size_full().track_focus(&self.focus)
    }
}
impl Item for Tab {
    fn tab_title(&self, _: &App) -> SharedString {
        self.cwd.display().to_string().into()
    }
    fn session(&self, _: &App) -> Option<SessionContext> {
        self.target
            .clone()
            .map(|target| SessionContext::new(target, None, true))
    }
    fn on_close(&mut self, _: &mut Window, _: &mut Context<Self>) {
        self.closed.set(self.closed.get() + 1);
    }
    fn command_enabled(&self, _: ItemCommand, _: &App) -> bool {
        true
    }
    fn execute(&mut self, _: ItemCommand, _: &mut Window, _: &mut Context<Self>) {
        self.executed.set(self.executed.get() + 1);
    }
}
impl crate::LocalTerminal for Tab {
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
fn tab(cx: &mut App, name: &str) -> Entity<Tab> {
    cx.new(|cx| Tab {
        focus: cx.focus_handle(),
        cwd: name.into(),
        closed: Rc::default(),
        executed: Rc::default(),
        target: None,
    })
}
fn with(
    cx: &mut TestAppContext,
    window: AnyWindowHandle,
    workspace: &Entity<Workspace>,
    f: impl FnOnce(&mut Workspace, &mut Window, &mut Context<Workspace>),
) {
    cx.update_window(window, |_, window, cx| {
        workspace.update(cx, |ws, cx| f(ws, window, cx))
    })
    .unwrap();
    cx.run_until_parked();
}
fn open_three(
    cx: &mut TestAppContext,
    local: bool,
) -> (AnyWindowHandle, Entity<Workspace>, Vec<Entity<Tab>>) {
    let (window, workspace) = super::tests::fixture(cx);
    let tabs = cx
        .update_window(window, |_, window, cx| {
            let tabs = ["/a", "/b", "/c"].map(|name| tab(cx, name));
            workspace.update(cx, |ws, cx| {
                for tab in &tabs {
                    if local {
                        ws.add_local_terminal(tab.clone(), window, cx);
                    } else {
                        ws.add_item(tab.clone(), window, cx);
                    }
                }
            });
            tabs.to_vec()
        })
        .unwrap();
    cx.run_until_parked();
    (window, workspace, tabs)
}
fn menu(cx: &mut TestAppContext, window: AnyWindowHandle, id: EntityId, item: usize) {
    cx.update_window(window, |_, window, cx| {
        window.render_frame(cx);
        window.right_click(("tab-title", id.as_u64()), cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(window, |_, window, cx| {
        window.render_frame(cx);
        window.within("popup-menu").click(item, cx);
    })
    .unwrap();
    cx.run_until_parked();
}
fn zoom(cx: &mut TestAppContext, window: AnyWindowHandle, ws: &Entity<Workspace>, id: EntityId) {
    with(cx, window, ws, |ws, window, cx| {
        let (_, node) = ws.item_location(id, cx).unwrap();
        ws.dock
            .update(cx, |dock, cx| dock.set_zoomed_in(node, window, cx));
    });
}
fn bar(ws: &Workspace, id: EntityId, cx: &App) -> Vec<EntityId> {
    let (placement, node) = ws.item_location(id, cx).unwrap();
    let PaneRef::Tabs { panels, .. } = ws
        .dock
        .read(cx)
        .layout(placement)
        .unwrap()
        .find_node(node)
        .unwrap()
        .kind()
    else {
        panic!("tab pane");
    };
    panels
        .iter()
        .map(|panel| {
            ws.items
                .iter()
                .find(|open| PanelId::from(open.dock_item.entity_id()) == *panel)
                .unwrap()
                .handle
                .item_id()
        })
        .collect()
}

#[gpui_kit::test]
fn native_menu_groups_and_splits_work_in_both_placements_and_zoom_states(cx: &mut TestAppContext) {
    for local in [false, true] {
        for zoomed in [false, true] {
            let (window, ws, tabs) = open_three(cx, local);
            let clicked = tabs[1].entity_id();
            if zoomed {
                zoom(cx, window, &ws, clicked);
            }
            cx.update_window(window, |_, window, cx| {
                window.render_frame(cx);
                window.right_click(("tab-title", clicked.as_u64()), cx);
            })
            .unwrap();
            cx.run_until_parked();
            cx.update_window(window, |_, window, cx| {
                window.render_frame(cx);
                for item in [0usize, 1, 2, 3, 4, 6, 7, 9] {
                    assert!(
                        window.within("popup-menu").find(item).visible(),
                        "local={local}, zoom={zoomed}, menu item={item}"
                    );
                }
                window.within("popup-menu").click(9usize, cx);
            })
            .unwrap();
            cx.run_until_parked();
            ws.read_with(cx, |ws, cx| {
                assert!(ws.is_tab_grouped(clicked));
                assert!(
                    ws.items
                        .iter()
                        .find(|open| open.handle.item_id() == clicked)
                        .unwrap()
                        .dock_item
                        .read(cx)
                        .group_color()
                        .is_some()
                );
            });
            menu(cx, window, clicked, 7);
            ws.read_with(cx, |ws, cx| {
                assert_ne!(
                    ws.item_location(clicked, cx),
                    ws.item_location(tabs[0].entity_id(), cx)
                );
                assert!(ws.dock.read(cx).zoomed_group().is_none());
                assert!(!ws.is_tab_grouped(clicked));
                assert_eq!(ws.items.len(), 3);
            });
            for tab in tabs {
                assert_eq!(tab.read_with(cx, |tab, _| tab.closed.get()), 0);
            }
        }
    }
}

#[gpui_kit::test]
fn native_close_scopes_use_clicked_inactive_tab_and_current_visual_order(cx: &mut TestAppContext) {
    for (item, survivors) in [
        (0usize, vec![0, 2]),
        (1, vec![1]),
        (2, vec![0, 1]),
        (3, vec![1, 2]),
    ] {
        let (window, ws, tabs) = open_three(cx, true);
        // Reorder [a,b,c] to [c,b,a]; c stays active while b is right-clicked.
        with(cx, window, &ws, |ws, window, cx| {
            let (_, node) = ws.item_location(tabs[0].entity_id(), cx).unwrap();
            let a = ws.panel_of(tabs[0].entity_id()).unwrap();
            ws.dock.update(cx, |dock, cx| {
                dock.move_panel(
                    a,
                    InsertTarget::Tabs {
                        node,
                        ix: Some(2),
                        activate: false,
                    },
                    window,
                    cx,
                )
            });
            let c = ws.panel_of(tabs[2].entity_id()).unwrap();
            ws.dock.update(cx, |dock, cx| {
                dock.move_panel(
                    c,
                    InsertTarget::Tabs {
                        node,
                        ix: Some(0),
                        activate: false,
                    },
                    window,
                    cx,
                )
            });
            ws.activate_item_by_id(tabs[2].entity_id(), window, cx);
        });
        zoom(cx, window, &ws, tabs[2].entity_id());
        menu(cx, window, tabs[1].entity_id(), item);
        ws.read_with(cx, |ws, _| {
            for (ix, tab) in tabs.iter().enumerate() {
                assert_eq!(
                    ws.items
                        .iter()
                        .any(|open| open.handle.item_id() == tab.entity_id()),
                    survivors.contains(&ix),
                    "close menu item {item}, original tab {ix}"
                );
            }
        });
        for (ix, tab) in tabs.iter().enumerate() {
            assert_eq!(
                tab.read_with(cx, |tab, _| tab.closed.get()),
                usize::from(!survivors.contains(&ix))
            );
        }
    }
}

#[gpui_kit::test]
fn native_zoom_drag_reorders_and_close_all_leaves_other_placement_alive(cx: &mut TestAppContext) {
    let (window, ws, tabs) = open_three(cx, true);
    let central = cx
        .update_window(window, |_, window, cx| {
            let central = tab(cx, "/remote");
            ws.update(cx, |ws, cx| ws.add_item(central.clone(), window, cx));
            central
        })
        .unwrap();
    cx.run_until_parked();
    zoom(cx, window, &ws, tabs[2].entity_id());
    cx.update_window(window, |_, window, cx| {
        window.render_frame(cx);
        window.drag_to(
            ("tab-title", tabs[0].entity_id().as_u64()),
            "local-terminal-free-header",
            cx,
        );
    })
    .unwrap();
    cx.run_until_parked();
    ws.read_with(cx, |ws, cx| {
        assert!(ws.dock.read(cx).zoomed_group().is_some());
        assert_eq!(
            bar(ws, tabs[0].entity_id(), cx),
            [
                tabs[1].entity_id(),
                tabs[2].entity_id(),
                tabs[0].entity_id()
            ]
        );
    });
    menu(cx, window, tabs[1].entity_id(), 6);
    menu(cx, window, tabs[2].entity_id(), 4);
    ws.read_with(cx, |ws, cx| {
        assert_eq!(ws.items.len(), 1);
        assert!(ws.item_visible(central.entity_id(), cx));
        assert!(!ws.dock.read(cx).has_dock(DockPlacement::Bottom));
    });
    assert_eq!(central.read_with(cx, |tab, _| tab.closed.get()), 0);
    for tab in tabs {
        assert_eq!(tab.read_with(cx, |tab, _| tab.closed.get()), 1);
    }
}

#[gpui_kit::test]
#[expect(clippy::too_many_lines, reason = "predates the limit")]
fn hidden_split_dock_preserves_topology_selection_sizes_and_center_local(cx: &mut TestAppContext) {
    let (window, ws, tabs) = open_three(cx, true);
    let moved = cx
        .update_window(window, |_, window, cx| {
            let anchor = tab(cx, "/anchor");
            let central = tab(cx, "/center");
            ws.update(cx, |ws, cx| {
                ws.add_item(anchor.clone(), window, cx);
                ws.add_local_terminal(central.clone(), window, cx);
                let panel = ws.panel_of(central.entity_id()).unwrap();
                let (_, node) = ws.item_location(anchor.entity_id(), cx).unwrap();
                ws.dock.update(cx, |dock, cx| {
                    dock.move_panel(
                        panel,
                        InsertTarget::Tabs {
                            node,
                            ix: None,
                            activate: true,
                        },
                        window,
                        cx,
                    )
                });
            });
            central
        })
        .unwrap();
    cx.run_until_parked();
    with(cx, window, &ws, |ws, window, cx| {
        ws.split_item(tabs[1].entity_id(), Placement::Right, window, cx);
        ws.activate_item_by_id(tabs[1].entity_id(), window, cx);
        ws.dock.update(cx, |dock, cx| {
            dock.set_dock_size(DockPlacement::Bottom, px(260.), window, cx)
        });
    });
    let before = ws.read_with(cx, |ws, cx| {
        (
            ws.dock
                .read(cx)
                .layout(DockPlacement::Bottom)
                .unwrap()
                .root()
                .clone(),
            ws.dock.read(cx).dump(cx).bottom_dock.unwrap(),
        )
    });
    zoom(cx, window, &ws, tabs[1].entity_id());
    with(cx, window, &ws, |ws, window, cx| {
        ws.toggle_local_terminal(window, cx)
    });
    ws.read_with(cx, |ws, cx| {
        let dock = ws.dock.read(cx);
        assert!(!dock.is_dock_open(DockPlacement::Bottom));
        assert!(dock.zoomed_group().is_none());
        assert_eq!(
            dock.layout(DockPlacement::Bottom).unwrap().root(),
            &before.0
        );
        assert_eq!(dock.dump(cx).bottom_dock.unwrap().size(), before.1.size());
        assert!(ws.item_visible(moved.entity_id(), cx));
        assert!(!ws.item_visible(tabs[0].entity_id(), cx));
    });
    with(cx, window, &ws, |ws, window, cx| {
        ws.toggle_local_terminal(window, cx)
    });
    ws.read_with(cx, |ws, cx| {
        assert_eq!(
            ws.dock
                .read(cx)
                .layout(DockPlacement::Bottom)
                .unwrap()
                .root(),
            &before.0
        );
        assert_eq!(
            ws.dock.read(cx).dump(cx).bottom_dock.as_ref(),
            Some(&before.1)
        );
        assert_eq!(ws.local_terminal_cwd(cx), Some("/b".into()));
    });
    for tab in tabs {
        assert_eq!(tab.read_with(cx, |tab, _| tab.closed.get()), 0);
    }
    assert_eq!(moved.read_with(cx, |tab, _| tab.closed.get()), 0);
}

#[gpui_kit::test]
fn central_public_ordinals_survive_interleaved_local_registry_entries(cx: &mut TestAppContext) {
    let (window, ws) = super::tests::fixture(cx);
    let (local, first, second) = cx
        .update_window(window, |_, window, cx| {
            let local = tab(cx, "/local");
            let first = tab(cx, "/first");
            let second = tab(cx, "/second");
            first.update(cx, |tab, _| {
                tab.target = Some(Target::parse("user@host", None).unwrap())
            });
            ws.update(cx, |ws, cx| {
                ws.add_local_terminal(local.clone(), window, cx);
                ws.add_item(first.clone(), window, cx);
                ws.add_local_terminal(tab(cx, "/interleaved"), window, cx);
                ws.add_item(second.clone(), window, cx);
            });
            (local, first, second)
        })
        .unwrap();
    cx.run_until_parked();
    with(cx, window, &ws, |ws, window, cx| {
        assert_eq!(
            ws.items().map(ItemHandle::item_id).collect::<Vec<_>>(),
            [first.entity_id(), second.entity_id()]
        );
        ws.activate_item(0, window, cx);
        assert_eq!(ws.active_item().unwrap().item_id(), first.entity_id());
        let remote = ws.active_session(cx).unwrap().target;
        ws.activate_item_by_id(local.entity_id(), window, cx);
        assert_eq!(ws.active_item().unwrap().item_id(), first.entity_id());
        assert_eq!(ws.active_session(cx).unwrap().target, remote);
        assert_eq!(ws.local_terminal_cwd(cx), Some("/local".into()));
        ws.close_item(1, window, cx);
        assert_eq!(
            ws.items().map(ItemHandle::item_id).collect::<Vec<_>>(),
            [first.entity_id()]
        );
        ws.close_item(9, window, cx);
        ws.activate_item(9, window, cx);
    });
    assert_eq!(second.read_with(cx, |tab, _| tab.closed.get()), 1);
    assert_eq!(local.read_with(cx, |tab, _| tab.closed.get()), 0);
    assert_eq!(first.read_with(cx, |tab, _| tab.closed.get()), 0);
}

#[gpui_kit::test]
fn zoomed_nonfirst_header_plus_adds_into_clicked_pane_and_keeps_zoom(cx: &mut TestAppContext) {
    let (window, ws, tabs) = open_three(cx, true);
    with(cx, window, &ws, |ws, window, cx| {
        ws.set_local_terminal_opener(|ws, target, window, cx| {
            ws.add_local_terminal_at(tab(cx, "/new"), target, window, cx);
        });
        ws.split_item(tabs[1].entity_id(), Placement::Right, window, cx);
        ws.activate_item_by_id(tabs[0].entity_id(), window, cx);
    });
    zoom(cx, window, &ws, tabs[1].entity_id());
    let target = ws.read_with(cx, |ws, cx| {
        ws.item_location(tabs[1].entity_id(), cx).unwrap()
    });
    cx.update_window(window, |_, window, cx| {
        window.render_frame(cx);
        window.click("local-terminal-new", cx);
    })
    .unwrap();
    cx.run_until_parked();
    ws.read_with(cx, |ws, cx| {
        assert_eq!(ws.items.len(), 4);
        let added = ws.items.last().unwrap().handle.item_id();
        assert_eq!(ws.item_location(added, cx), Some(target));
        assert_eq!(ws.dock.read(cx).zoomed_group(), Some(target.1));
        assert_eq!(bar(ws, added, cx), [tabs[1].entity_id(), added]);
    });
    for tab in tabs {
        assert_eq!(tab.read_with(cx, |tab, _| tab.closed.get()), 0);
    }
}

#[gpui_kit::test]
fn joining_group_in_hidden_bottom_does_not_hide_clicked_central_tab(cx: &mut TestAppContext) {
    let (window, ws, tabs) = open_three(cx, true);
    let central = cx
        .update_window(window, |_, window, cx| {
            let central = tab(cx, "/central");
            ws.update(cx, |ws, cx| ws.add_item(central.clone(), window, cx));
            central
        })
        .unwrap();
    cx.run_until_parked();
    with(cx, window, &ws, |ws, window, cx| {
        ws.new_tab_group(tabs[0].entity_id(), window, cx);
        ws.toggle_local_terminal(window, cx);
        ws.group_tab_with(central.entity_id(), tabs[0].entity_id(), window, cx);
    });
    ws.read_with(cx, |ws, cx| {
        assert!(
            ws.item_visible(central.entity_id(), cx),
            "joining a hidden group must reveal its destination or decline the move"
        );
    });
    assert_eq!(central.read_with(cx, |tab, _| tab.closed.get()), 0);
    for tab in tabs {
        assert_eq!(tab.read_with(cx, |tab, _| tab.closed.get()), 0);
    }
}

#[gpui_kit::test]
fn native_single_bottom_terminal_can_move_to_central_header_without_closing(
    cx: &mut TestAppContext,
) {
    let (window, ws) = super::tests::fixture(cx);
    let (central, local) = cx
        .update_window(window, |_, window, cx| {
            let central = tab(cx, "/central");
            let local = tab(cx, "/local");
            ws.update(cx, |ws, cx| {
                ws.add_item(central.clone(), window, cx);
                ws.add_local_terminal(local.clone(), window, cx);
            });
            (central, local)
        })
        .unwrap();
    cx.run_until_parked();
    cx.update_window(window, |_, window, cx| {
        window.render_frame(cx);
        window.drag_to(
            ("tab-title", local.entity_id().as_u64()),
            ("tab-title", central.entity_id().as_u64()),
            cx,
        );
    })
    .unwrap();
    cx.run_until_parked();
    ws.read_with(cx, |ws, cx| {
        assert_eq!(
            ws.item_location(local.entity_id(), cx).unwrap().0,
            DockPlacement::Center
        );
        assert_eq!(ws.items.len(), 2);
        assert!(!ws.dock.read(cx).has_dock(DockPlacement::Bottom));
        assert_eq!(ws.local_terminal_cwd(cx), Some("/local".into()));
    });
    assert_eq!(local.read_with(cx, |tab, _| tab.closed.get()), 0);
    assert_eq!(central.read_with(cx, |tab, _| tab.closed.get()), 0);
}

#[gpui_kit::test]
fn hidden_mixed_bottom_cannot_receive_commands_or_close_tab_actions(cx: &mut TestAppContext) {
    let (window, ws, tabs) = open_three(cx, true);
    let central = cx
        .update_window(window, |_, window, cx| {
            let central = tab(cx, "/remote");
            central.update(cx, |tab, _| {
                tab.target = Some(Target::parse("user@host", None).unwrap())
            });
            ws.update(cx, |ws, cx| {
                ws.add_item(central.clone(), window, cx);
                ws.new_tab_group(tabs[0].entity_id(), window, cx);
                ws.group_tab_with(central.entity_id(), tabs[0].entity_id(), window, cx);
                assert_eq!(
                    ws.item_location(central.entity_id(), cx).unwrap().0,
                    DockPlacement::Bottom
                );
                ws.activate_item_by_id(central.entity_id(), window, cx);
                ws.toggle_local_terminal(window, cx);
                window.focus(&ws.focus_handle, cx);
            });
            central
        })
        .unwrap();
    cx.run_until_parked();
    cx.update_window(window, |_, window, cx| {
        ws.update(cx, |ws, cx| {
            assert!(
                ws.command_item(window, cx).is_none(),
                "hidden previous central context must not receive commands"
            );
            assert!(!ws.item_command_enabled(ItemCommand::Copy, window, cx));
            assert!(!ws.item_command_enabled(ItemCommand::Find, window, cx));
            ws.execute_item_command(ItemCommand::Copy, window, cx);
            assert_eq!(
                ws.active_session(cx).unwrap().target,
                Target::parse("user@host", None).unwrap()
            );
        });
        window.dispatch_action(Box::new(CloseTab), cx);
    })
    .unwrap();
    cx.run_until_parked();
    ws.read_with(cx, |ws, _| assert_eq!(ws.items.len(), 4));
    for tab in tabs.into_iter().chain([central]) {
        assert_eq!(tab.read_with(cx, |tab, _| tab.executed.get()), 0);
        assert_eq!(tab.read_with(cx, |tab, _| tab.closed.get()), 0);
    }
}

#[gpui_kit::test]
fn deferred_find_is_cancelled_when_its_bottom_target_hides_before_delivery(
    cx: &mut TestAppContext,
) {
    let (window, ws, tabs) = open_three(cx, true);
    cx.update_window(window, |_, window, cx| {
        window.render_frame(cx);
        ws.update(cx, |ws, cx| {
            ws.activate_item_by_id(tabs[1].entity_id(), window, cx);
        });
        window.render_frame(cx);
        ws.update(cx, |ws, cx| {
            assert!(ws.item_command_enabled(ItemCommand::Find, window, cx));
            ws.dispatch_item_command(ItemCommand::Find, window, cx);
            ws.toggle_local_terminal(window, cx);
        });
    })
    .unwrap();
    cx.run_until_parked();
    ws.read_with(cx, |ws, cx| {
        assert!(!ws.item_visible(tabs[1].entity_id(), cx));
        assert_eq!(ws.items.len(), 3);
    });
    for tab in tabs {
        assert_eq!(tab.read_with(cx, |tab, _| tab.executed.get()), 0);
        assert_eq!(tab.read_with(cx, |tab, _| tab.closed.get()), 0);
    }
}
