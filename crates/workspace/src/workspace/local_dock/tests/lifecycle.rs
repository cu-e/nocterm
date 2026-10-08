//! Zoom cleanup and explicit terminal access across dock placements.
use super::*;

#[gpui_kit::test]
fn hiding_and_closing_the_last_zoomed_local_tab_clear_zoom(cx: &mut TestAppContext) {
    let (handle, workspace, closes) = fixture(cx);
    cx.update_window(handle, |_, window, cx| {
        workspace.update(cx, |workspace, cx| {
            workspace.new_local_terminal(window, cx);
            workspace.new_local_terminal(window, cx);
        });
        window.render_frame(cx);
        window.click("zoom-in", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        assert!(workspace.read(cx).dock.read(cx).zoomed_group().is_some());
        workspace.update(cx, |workspace, cx| {
            workspace.toggle_local_terminal(window, cx);
        });
        window.render_frame(cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        let state = workspace.read(cx);
        assert_eq!(count(&workspace, cx), 2);
        assert!(!state.local_terminal_is_visible(cx));
        assert!(state.dock.read(cx).has_dock(DockPlacement::Bottom));
        assert!(state.dock.read(cx).zoomed_group().is_none());
        assert_eq!(closes.get(), 0);
        workspace.update(cx, |workspace, cx| {
            workspace.toggle_local_terminal(window, cx);
        });
        window.render_frame(cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        assert!(workspace.read(cx).local_terminal_is_visible(cx));
        assert!(workspace.read(cx).dock.read(cx).zoomed_group().is_none());
        workspace.update(cx, |workspace, cx| {
            workspace.close_local_terminal(window, cx);
        });
        window.render_frame(cx);
        window.click("zoom-in", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        assert_eq!(count(&workspace, cx), 1);
        assert!(workspace.read(cx).dock.read(cx).zoomed_group().is_some());
        workspace.update(cx, |workspace, cx| {
            workspace.close_local_terminal(window, cx);
        });
        window.render_frame(cx);
    })
    .unwrap();
    cx.run_until_parked();
    workspace.read_with(cx, |workspace, cx| {
        assert!(workspace.selected_local_id.is_none());
        assert!(!workspace.dock.read(cx).has_dock(DockPlacement::Bottom));
        assert!(workspace.dock.read(cx).zoomed_group().is_none());
    });
    assert_eq!(closes.get(), 2);
}

struct Access;
impl crate::TerminalAccess for Access {
    fn info(&self, _: &App) -> Option<crate::TerminalInfo> {
        None
    }
    fn read(&self, _: crate::TextRequest, _: &App) -> Result<crate::TerminalText, String> {
        Err("unused access operation".into())
    }
    fn send_text(&self, _: &str, _: &mut App) -> Result<(), String> {
        Err("unused access operation".into())
    }
    fn run_command(&self, _: &str, _: &mut App) -> Result<(), String> {
        Err("unused access operation".into())
    }
    fn answer_sign_in(&self, _: String, _: &mut App) -> Result<(), String> {
        Err("unused access operation".into())
    }
}

struct AccessProbe {
    focus: FocusHandle,
    cwd: PathBuf,
    access: Rc<dyn crate::TerminalAccess>,
}
impl EventEmitter<ItemEvent> for AccessProbe {}
impl Focusable for AccessProbe {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}
impl Render for AccessProbe {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div().size_full().track_focus(&self.focus)
    }
}
impl Item for AccessProbe {
    fn tab_title(&self, _: &App) -> SharedString {
        self.cwd.display().to_string().into()
    }
    fn terminal_access(&self) -> Option<Rc<dyn crate::TerminalAccess>> {
        Some(self.access.clone())
    }
}
impl crate::LocalTerminal for AccessProbe {
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

#[gpui_kit::test]
#[expect(clippy::too_many_lines, reason = "predates the limit")]
fn all_local_access_handles_remain_selectable_after_move_and_unfocusable_when_hidden(
    cx: &mut TestAppContext,
) {
    let (handle, workspace, _) = fixture(cx);
    let (a, b, central) = cx
        .update_window(handle, |_, window, cx| {
            let central = probe(cx, "/central", Rc::default());
            let mut make = |path: &str| {
                cx.new(|cx| AccessProbe {
                    focus: cx.focus_handle(),
                    cwd: path.into(),
                    access: Rc::new(Access),
                })
            };
            let a = make("/a");
            let b = make("/b");
            workspace.update(cx, |workspace, cx| {
                workspace.add_item(central.clone(), window, cx);
                workspace.add_local_terminal(a.clone(), window, cx);
                workspace.add_local_terminal(b.clone(), window, cx);
            });
            window.render_frame(cx);
            (a, b, central)
        })
        .unwrap();
    cx.run_until_parked();

    for (item, path) in [(&a, "/a"), (&b, "/b")] {
        cx.update_window(handle, |_, window, cx| {
            workspace.update(cx, |workspace, cx| {
                let terminals = workspace.terminals(cx);
                assert_eq!(terminals.len(), 2);
                for terminal in &terminals {
                    let expected = if terminal.item == a.entity_id() {
                        &a
                    } else {
                        &b
                    };
                    assert_eq!(terminal.item, expected.entity_id());
                    assert!(Rc::ptr_eq(&terminal.access, &expected.read(cx).access));
                    assert!(terminal.bottom && !terminal.background);
                }
                assert!(workspace.focus_terminal(item.entity_id(), window, cx));
            });
            window.render_frame(cx);
        })
        .unwrap();
        cx.run_until_parked();
        workspace.read_with(cx, |workspace, cx| {
            assert_eq!(workspace.active_terminal(cx), Some(item.entity_id()));
            assert_eq!(workspace.local_terminal_cwd(cx), Some(path.into()));
            assert_eq!(
                workspace.active_item().unwrap().item_id(),
                central.entity_id()
            );
            assert_eq!(
                workspace
                    .terminals(cx)
                    .iter()
                    .filter(|entry| entry.active)
                    .count(),
                1
            );
        });
    }

    cx.update_window(handle, |_, window, cx| {
        workspace.update(cx, |workspace, cx| {
            let b_panel = PanelId::from(
                workspace
                    .local_entry(b.entity_id())
                    .unwrap()
                    .dock_item
                    .entity_id(),
            );
            let central_panel = PanelId::from(workspace.items[0].dock_item.entity_id());
            workspace.dock.update(cx, |dock, cx| {
                let node = dock
                    .layout(DockPlacement::Center)
                    .unwrap()
                    .find_panel_node(central_panel)
                    .unwrap();
                dock.move_panel(
                    b_panel,
                    InsertTarget::Tabs {
                        node,
                        ix: None,
                        activate: false,
                    },
                    window,
                    cx,
                );
            });
            assert_eq!(
                workspace
                    .item_location(b.entity_id(), cx)
                    .map(|(placement, _)| placement),
                Some(DockPlacement::Center)
            );
            assert!(workspace.focus_terminal(b.entity_id(), window, cx));
        });
        window.render_frame(cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        workspace.update(cx, |workspace, cx| {
            assert_eq!(workspace.active_terminal(cx), Some(b.entity_id()));
            assert_eq!(workspace.local_terminal_cwd(cx), Some("/b".into()));
            workspace.toggle_local_terminal(window, cx);
        });
        window.render_frame(cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        workspace.update(cx, |workspace, cx| {
            assert_eq!(workspace.terminals(cx).len(), 2);
            assert_eq!(workspace.local_terminal_cwd(cx), Some("/b".into()));
            assert!(!workspace.focus_terminal(a.entity_id(), window, cx));
            assert!(workspace.focus_terminal(b.entity_id(), window, cx));
            assert!(b.read(cx).focus_handle(cx).is_focused(window));
        });
    })
    .unwrap();
}
