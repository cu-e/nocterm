//! Menus and command routing reach the focused item.
use super::*;

#[gpui_kit::test]
fn standard_menu_opens_and_dispatches_to_the_focused_bottom_terminal(cx: &mut TestAppContext) {
    let (handle, workspace) = fixture(cx);
    let closes = Rc::new(Cell::new(0));
    let (central, local) = cx
        .update_window(handle, |_, window, cx| {
            let central = probe(cx, closes.clone());
            let local = probe(cx, closes);
            workspace.update(cx, |workspace, cx| {
                workspace.add_item(central.clone(), window, cx);
                workspace.set_local_terminal(local.clone(), window, cx);
                workspace.set_menu_builder(
                    |workspace, window, cx| {
                        vec![
                            gpui_kit::Menu::new("Edit").items([gpui_kit::MenuItem::action(
                                "Copy",
                                crate::EditCopy,
                            )
                            .disabled(!workspace.command_available(
                                &gpui_kit::component::input::Copy,
                                window,
                                cx,
                            ))]),
                            gpui_kit::Menu::new("Search")
                                .items([gpui_kit::MenuItem::action("Find", crate::Find)]),
                        ]
                    },
                    window,
                    cx,
                );
            });
            window.render_frame(cx);
            window
                .within("app-menu-bar")
                .within(0usize)
                .click("menu", cx);
            window.render_frame(cx);
            assert!(
                window.try_find("popup-menu").is_some(),
                "standard menu must actually open after state refresh"
            );
            assert_eq!(
                workspace
                    .read(cx)
                    .command_item(window, cx)
                    .unwrap()
                    .item_id(),
                local.entity_id(),
                "popup focus keeps the bottom terminal context"
            );
            window.within("popup-menu").click(0usize, cx);
            (central, local)
        })
        .unwrap();
    cx.run_until_parked();
    assert_eq!(
        local.read_with(cx, |item, _| item.executed.borrow().clone()),
        vec![Command::Copy]
    );
    assert!(central.read_with(cx, |item, _| item.executed.borrow().is_empty()));
}

#[gpui_kit::test]
fn native_menu_rename_focuses_alias_input_after_dismissal(cx: &mut TestAppContext) {
    let (handle, workspace) = fixture(cx);
    cx.update_window(handle, |_, window, cx| {
        let item = probe(cx, Rc::new(Cell::new(0)));
        workspace.update(cx, |workspace, cx| {
            workspace.add_item(item, window, cx);
            workspace.set_menu_builder(
                |_, _, _| {
                    vec![
                        gpui_kit::Menu::new("Session")
                            .items([gpui_kit::MenuItem::action("Rename Tab", crate::RenameTab)]),
                    ]
                },
                window,
                cx,
            );
        });
        window.render_frame(cx);
        window
            .within("app-menu-bar")
            .within(0usize)
            .click("menu", cx);
        window.within("popup-menu").click(0usize, cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("popup-menu").is_none());
        assert!(
            window.has_focused_input(cx),
            "menu dismissal cannot steal alias input focus"
        );
        window.press("ctrl-a", cx);
        window.input("Native menu alias", cx);
        window.press("enter", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        assert_eq!(
            workspace.read(cx).command_title(window, cx).as_deref(),
            Some("Native menu alias")
        );
    })
    .unwrap();
}

#[gpui_kit::test]
fn native_new_session_focuses_the_connection_menu_after_dismissal(cx: &mut TestAppContext) {
    let (handle, workspace) = fixture(cx);
    let menu = cx
        .update_window(handle, |_, window, cx| {
            let item = probe(cx, Rc::new(Cell::new(0)));
            let menu = probe(cx, Rc::new(Cell::new(0)));
            workspace.update(cx, |workspace, cx| {
                workspace.add_item(item, window, cx);
                workspace.set_new_tab_menu(menu.clone(), cx);
                workspace.set_menu_builder(
                    |_, _, _| {
                        vec![
                            gpui_kit::Menu::new("Session")
                                .items([gpui_kit::MenuItem::action("New Session", crate::NewTab)]),
                        ]
                    },
                    window,
                    cx,
                );
            });
            window.render_frame(cx);
            window
                .within("app-menu-bar")
                .within(0usize)
                .click("menu", cx);
            window.within("popup-menu").click(0usize, cx);
            menu
        })
        .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(workspace.read(cx).new_tab_menu_open);
        assert!(menu.read(cx).focus_handle(cx).contains_focused(window, cx));
        assert!(window.try_find("popup-menu").is_none());
    })
    .unwrap();
}

#[gpui_kit::test]
fn command_routing_prefers_focus_and_rejects_unsupported_commands(cx: &mut TestAppContext) {
    let (handle, workspace) = fixture(cx);
    let (a, b, local) = cx
        .update_window(handle, |_, window, cx| {
            let a = probe(cx, Rc::new(Cell::new(0)));
            let b = probe(cx, Rc::new(Cell::new(0)));
            let local = probe(cx, Rc::new(Cell::new(0)));
            workspace.update(cx, |workspace, cx| {
                assert!(workspace.command_item(window, cx).is_none());
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
    cx.update_window(handle, |_, window, cx| {
        workspace.update(cx, |workspace, cx| {
            assert_eq!(
                workspace.command_item(window, cx).unwrap().item_id(),
                local.entity_id()
            );
            let paste = gpui_kit::component::input::Paste;
            assert!(!workspace.command_available(&paste, window, cx));
            assert!(workspace.command_available(&crate::Find, window, cx));
            workspace.dispatch_command(Box::new(paste), window, cx);
        });
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        assert!(local.read(cx).executed.borrow().is_empty());
        window.focus(&a.read(cx).focus_handle(cx), cx);
        window.render_frame(cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        workspace.update(cx, |workspace, cx| {
            assert_eq!(
                workspace.command_item(window, cx).unwrap().item_id(),
                a.entity_id()
            );
            workspace.dispatch_command(Box::new(crate::Find), window, cx);
        });
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        assert_eq!(*a.read(cx).executed.borrow(), vec![Command::Find]);
        assert!(b.read(cx).executed.borrow().is_empty());
        workspace.update(cx, |workspace, cx| {
            window.focus(&workspace.focus_handle(cx), cx);
            assert_eq!(
                workspace.command_item(window, cx).unwrap().item_id(),
                workspace.active_item().unwrap().item_id(),
                "focus outside Item containers falls back to the active central pane"
            );
        });
    })
    .unwrap();
}

#[gpui_kit::test]
fn copy_connection_name_uses_the_display_alias_without_editing_it(cx: &mut TestAppContext) {
    let (handle, workspace) = fixture(cx);
    cx.update_window(handle, |_, window, cx| {
        let item = probe(cx, Rc::new(Cell::new(0)));
        workspace.update(cx, |workspace, cx| {
            workspace.add_item(item, window, cx);
            workspace.items[0]
                .dock_item
                .update(cx, |dock, _| dock.alias = Some("Production west".into()));
        });
        window.render_frame(cx);
        window.dispatch_action(Box::new(crate::CopyConnectionName), cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.read(|cx| {
        assert_eq!(
            cx.read_from_clipboard().unwrap().text().unwrap(),
            "Production west"
        );
        assert_eq!(
            workspace.read(cx).items[0]
                .dock_item
                .read(cx)
                .alias
                .as_deref(),
            Some("Production west")
        );
    });
}

#[gpui_kit::test]
#[expect(clippy::too_many_lines, reason = "predates the limit")]
fn menu_snapshots_are_window_local_and_refresh_only_on_opening(cx: &mut TestAppContext) {
    let (first, workspace) = fixture(cx);
    let (second, other) = cx.update(|cx| {
        gpui_kit::open_window(WindowOptions::default(), cx, |window, cx| {
            cx.new(|cx| Workspace::new(window, cx))
        })
        .unwrap()
    });
    let calls = Rc::new(Cell::new(0));
    cx.update(|cx| {
        gpui_kit::base::GlobalState::global_mut(cx)
            .set_app_menus(vec![gpui_kit::Menu::new("Sentinel").owned()]);
    });
    let item = cx
        .update_window(first, |_, window, cx| {
            let item = probe(cx, Rc::new(Cell::new(0)));
            let calls = calls.clone();
            workspace.update(cx, |workspace, cx| {
                workspace.add_item(item.clone(), window, cx);
                workspace.set_menu_builder(
                    move |workspace, window, cx| {
                        calls.set(calls.get() + 1);
                        vec![gpui_kit::Menu::new("First window").items([
                            gpui_kit::MenuItem::action("Copy", crate::EditCopy).disabled(
                                !workspace.command_available(
                                    &gpui_kit::component::input::Copy,
                                    window,
                                    cx,
                                ),
                            ),
                        ])]
                    },
                    window,
                    cx,
                );
            });
            item
        })
        .unwrap();
    cx.update_window(second, |_, window, cx| {
        other.update(cx, |other, cx| {
            other.set_menu_builder(
                |_, _, _| {
                    vec![
                        gpui_kit::Menu::new("Second window").items([gpui_kit::MenuItem::action(
                            "Find",
                            crate::Find,
                        )
                        .disabled(true)]),
                    ]
                },
                window,
                cx,
            )
        });
        window.render_frame(cx);
        assert_eq!(
            window
                .within("app-menu-bar")
                .within(0usize)
                .find("menu")
                .label(),
            Some("Second window")
        );
    })
    .unwrap();
    assert_eq!(calls.get(), 1);
    cx.update_window(first, |_, window, cx| {
        item.update(cx, |item, cx| {
            item.commands.clear();
            cx.emit(ItemEvent::Changed);
        });
        window.render_frame(cx);
        assert_eq!(
            window
                .within("app-menu-bar")
                .within(0usize)
                .find("menu")
                .label(),
            Some("First window")
        );
        assert_eq!(
            calls.get(),
            1,
            "output changes do not rebuild menu snapshots"
        );
        window
            .within("app-menu-bar")
            .within(0usize)
            .click("menu", cx);
        assert_eq!(calls.get(), 2, "initial opening takes fresh command state");
        window.render_frame(cx);
        assert!(window.try_find("popup-menu").is_some());
        window.within("popup-menu").click(0usize, cx);
        window.render_frame(cx);
        assert!(
            window.try_find("popup-menu").is_some(),
            "disabled item cannot invoke or dismiss the menu"
        );
        item.update(cx, |_, cx| cx.emit(ItemEvent::Changed));
        window.render_frame(cx);
        assert_eq!(
            calls.get(),
            2,
            "an open popup is stable during session updates"
        );
        assert!(item.read(cx).executed.borrow().is_empty());
        window.press("escape", cx);
    })
    .unwrap();
    cx.read(|cx| {
        assert_eq!(
            gpui_kit::base::GlobalState::global(cx).app_menus()[0]
                .name
                .as_ref(),
            "Sentinel"
        )
    });
}

#[gpui_kit::test]
fn native_menu_keyboard_navigation_keeps_the_original_item(cx: &mut TestAppContext) {
    let (handle, workspace) = fixture(cx);
    let item = cx
        .update_window(handle, |_, window, cx| {
            let item = probe(cx, Rc::new(Cell::new(0)));
            workspace.update(cx, |workspace, cx| {
                workspace.add_item(item.clone(), window, cx);
                workspace.set_menu_builder(
                    |_, _, _| {
                        vec![
                            gpui_kit::Menu::new("Edit")
                                .items([gpui_kit::MenuItem::action("Copy", crate::EditCopy)]),
                            gpui_kit::Menu::new("Search")
                                .items([gpui_kit::MenuItem::action("Find", crate::Find)]),
                        ]
                    },
                    window,
                    cx,
                );
            });
            window.render_frame(cx);
            // Use real tab navigation to focus the native menu trigger.
            for _ in 0..30 {
                if window
                    .within("app-menu-bar")
                    .within(0usize)
                    .find("menu")
                    .focused()
                    == Some(true)
                {
                    break;
                }
                window.focus_next(cx);
                window.render_frame(cx);
            }
            assert_eq!(
                window
                    .within("app-menu-bar")
                    .within(0usize)
                    .find("menu")
                    .focused(),
                Some(true)
            );
            window.press("enter", cx);
            window.render_frame(cx);
            assert!(window.try_find("popup-menu").is_some());
            window.press("right", cx);
            window.render_frame(cx);
            assert_eq!(
                workspace
                    .read(cx)
                    .command_item(window, cx)
                    .unwrap()
                    .item_id(),
                item.entity_id()
            );
            window.within("popup-menu").click(0usize, cx);
            item
        })
        .unwrap();
    cx.run_until_parked();
    assert_eq!(
        item.read_with(cx, |item, _| item.executed.borrow().clone()),
        vec![Command::Find]
    );
}
