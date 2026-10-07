use super::*;
use futures::FutureExt as _;
use gpui_kit::{TestAppContext, test::TestWindowExt as _};

mod drag;
mod reorder;

#[test]
fn description_preview_and_tooltip_are_bounded_without_losing_unicode() {
    let long = format!("\n  {}\nsecond line", "я".repeat(1000));
    let preview = description_preview(&long);
    assert_eq!(preview.chars().count(), 160);
    assert!(!preview.contains('\n'));
    let tooltip = description_tooltip(&long);
    assert_eq!(tooltip.chars().count(), 513);
    assert!(tooltip.ends_with('…'));
    assert_eq!(description_preview("\n\t"), "");
}

#[gpui_kit::test]
fn native_drag_moves_to_collapsed_folder_root_and_remembered_empty_folder(cx: &mut TestAppContext) {
    let (handle, workspace, opened) = crate::test_support::workspace(cx);
    let profile = |name: &str, group: &str| Profile {
        id: ProfileId::generate(),
        name: name.into(),
        group: Some(group.into()),
        target: nocterm_session::Target::new("ci", name, 22),
        description: String::new(),
        options: Default::default(),
        auth: Default::default(),
        credential: None,
        launch: None,
        icon: None,
        icon_color: None,
        country: None,
    };
    let a = profile("a", "Work");
    let b = profile("b", "Personal");
    cx.update_window(handle, |_, window, cx| {
        let connections = Connections::global(cx);
        connections.update(cx, |connections, cx| {
            connections
                .save_profile(a.clone(), cx)
                .now_or_never()
                .unwrap()
                .unwrap();
            connections
                .save_profile(b, cx)
                .now_or_never()
                .unwrap()
                .unwrap();
        });
        let panel = cx.new(|cx| {
            ConnectionsPanel::new(connections.clone(), workspace.downgrade(), window, cx)
        });
        workspace.update(cx, |workspace, cx| workspace.add_panel(panel, cx));
        window.render_frame(cx);
        window.click("group-Personal", cx);
        window.render_frame(cx);
        use gpui_kit::{
            InputEvent as _, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent,
        };
        let source = window
            .find(SharedString::from(format!("connection-{}", a.id)))
            .bounds();
        let from = source.center();
        let to = window.find("group-Personal").bounds().center();
        window.dispatch_event(
            MouseMoveEvent {
                position: from,
                pressed_button: None,
                modifiers: Default::default(),
            }
            .to_platform_input(),
            cx,
        );
        window.dispatch_event(
            MouseDownEvent {
                position: from,
                button: MouseButton::Left,
                modifiers: Default::default(),
                click_count: 1,
                first_mouse: false,
            }
            .to_platform_input(),
            cx,
        );
        window.render_frame(cx);
        window.dispatch_event(
            MouseMoveEvent {
                position: to,
                pressed_button: Some(MouseButton::Left),
                modifiers: Default::default(),
            }
            .to_platform_input(),
            cx,
        );
        window.render_frame(cx);
        assert!(
            cx.has_active_drag(),
            "a real row creates its typed drag payload"
        );
        let preview = window.find("drag-preview").bounds();
        assert!(
            preview.size.height > gpui_kit::px(20.) && preview.size.height < gpui_kit::px(80.),
            "preview geometry must be independent of terminal font size"
        );
        assert_eq!(
            preview.size, source.size,
            "drag preview keeps the actual row footprint"
        );
        window.dispatch_event(
            MouseUpEvent {
                position: to,
                button: MouseButton::Left,
                modifiers: Default::default(),
                click_count: 1,
            }
            .to_platform_input(),
            cx,
        );
        window.render_frame(cx);
        assert!(!cx.has_active_drag());
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        assert_eq!(
            Connections::global(cx)
                .read(cx)
                .profiles()
                .get(a.id)
                .unwrap()
                .group
                .as_deref(),
            Some("Personal")
        );
        window.render_frame(cx);
        assert!(
            window.try_find("group-Work").is_some(),
            "last-member folder remains a drop target"
        );
        window.click("group-Personal", cx);
        window.render_frame(cx);
        window.drag_to(
            SharedString::from(format!("connection-{}", a.id)),
            "connections-ungrouped",
            cx,
        );
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        assert!(
            Connections::global(cx)
                .read(cx)
                .profiles()
                .get(a.id)
                .unwrap()
                .group
                .is_none()
        );
        window.render_frame(cx);
        window.drag_to(
            SharedString::from(format!("connection-{}", a.id)),
            "group-Work",
            cx,
        );
    })
    .unwrap();
    cx.run_until_parked();
    cx.read(|cx| {
        assert_eq!(
            Connections::global(cx)
                .read(cx)
                .profiles()
                .get(a.id)
                .unwrap()
                .group
                .as_deref(),
            Some("Work")
        )
    });
    assert!(
        opened.borrow().is_empty(),
        "a drag must not open an SSH session"
    );
}

fn gp(name: &str, group: Option<&str>) -> Profile {
    Profile {
        id: ProfileId::generate(),
        name: name.into(),
        group: group.map(Into::into),
        target: nocterm_session::Target::new("ci", name, 22),
        description: String::new(),
        options: Default::default(),
        auth: Default::default(),
        credential: None,
        launch: None,
        icon: None,
        icon_color: None,
        country: None,
    }
}

fn setup(
    cx: &mut TestAppContext,
    profiles: &[Profile],
) -> (
    gpui_kit::AnyWindowHandle,
    Entity<ConnectionsPanel>,
    crate::test_support::Opened,
) {
    let (handle, workspace, opened) = crate::test_support::workspace(cx);
    let panel = cx
        .update_window(handle, |_, window, cx| {
            let connections = Connections::global(cx);
            for profile in profiles {
                connections.update(cx, |connections, cx| {
                    connections
                        .save_profile(profile.clone(), cx)
                        .now_or_never()
                        .unwrap()
                        .unwrap();
                });
            }
            let panel = cx.new(|cx| {
                ConnectionsPanel::new(connections.clone(), workspace.downgrade(), window, cx)
            });
            workspace.update(cx, |workspace, cx| workspace.add_panel(panel.clone(), cx));
            window.render_frame(cx);
            panel
        })
        .unwrap();
    (handle, panel, opened)
}

fn profiles_of(cx: &mut TestAppContext) -> crate::store::Profiles {
    cx.read(|cx| Connections::global(cx).read(cx).profiles().clone())
}

fn start_rename_via_double_click(
    cx: &mut TestAppContext,
    handle: gpui_kit::AnyWindowHandle,
    group: &str,
) {
    cx.update_window(handle, |_, window, cx| {
        window.double_click(SharedString::from(format!("group-name-{group}")), cx);
        window.render_frame(cx);
        assert!(
            window
                .try_find(SharedString::from(format!("group-rename-{group}")))
                .is_some()
        );
    })
    .unwrap();
}

fn set_rename_value(
    cx: &mut TestAppContext,
    handle: gpui_kit::AnyWindowHandle,
    panel: &Entity<ConnectionsPanel>,
    value: &str,
) {
    cx.update_window(handle, |_, window, cx| {
        let input = panel.read(cx).renaming.as_ref().unwrap().input.clone();
        input.update(cx, |input, cx| {
            input.set_value(value.to_owned(), window, cx)
        });
    })
    .unwrap();
}

#[gpui_kit::test]
fn double_click_renames_group_keeping_collapsed_state(cx: &mut TestAppContext) {
    let a = gp("a", Some("Work"));
    let (handle, panel, opened) = setup(cx, std::slice::from_ref(&a));
    cx.update_window(handle, |_, window, cx| {
        window.click("group-Work", cx);
        window.render_frame(cx);
        assert!(panel.read(cx).collapsed.contains("Work"));
    })
    .unwrap();
    start_rename_via_double_click(cx, handle, "Work");
    cx.update_window(handle, |_, _, cx| {
        assert!(panel.read(cx).collapsed.contains("Work"));
    })
    .unwrap();
    set_rename_value(cx, handle, &panel, "  Team ");
    cx.update_window(handle, |_, window, cx| window.press("enter", cx))
        .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let panel = panel.read(cx);
        assert!(panel.renaming.is_none());
        assert!(panel.collapsed.contains("Team") && !panel.collapsed.contains("Work"));
        assert!(window.try_find("group-Team").is_some());
        assert!(window.try_find("group-Work").is_none());
    })
    .unwrap();
    assert_eq!(
        profiles_of(cx).get(a.id).unwrap().group.as_deref(),
        Some("Team")
    );
    assert!(opened.borrow().is_empty());
}

#[gpui_kit::test]
fn rename_escape_cancels_and_noop_values_change_nothing(cx: &mut TestAppContext) {
    let (handle, panel, _) = setup(cx, &[gp("a", Some("Work"))]);
    let before = profiles_of(cx);
    for value in ["Other", "  ", "Work"] {
        start_rename_via_double_click(cx, handle, "Work");
        set_rename_value(cx, handle, &panel, value);
        if value == "Other" {
            cx.update_window(handle, |_, window, cx| window.press("escape", cx))
                .unwrap();
        } else {
            cx.update_window(handle, |_, window, cx| window.press("enter", cx))
                .unwrap();
        }
        cx.run_until_parked();
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            assert!(panel.read(cx).renaming.is_none());
        })
        .unwrap();
        assert_eq!(profiles_of(cx), before, "{value:?}");
    }
}

#[gpui_kit::test]
fn rename_commits_on_blur_and_merges_into_existing_group(cx: &mut TestAppContext) {
    let (a, b) = (gp("a", Some("Work")), gp("b", Some("Personal")));
    let (handle, panel, _) = setup(cx, &[a.clone(), b.clone()]);
    // Blur events are only dispatched while the window counts as active.
    cx.update_window(handle, |_, window, _| window.activate_window())
        .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| window.render_frame(cx))
        .unwrap();
    cx.run_until_parked();
    start_rename_via_double_click(cx, handle, "Work");
    set_rename_value(cx, handle, &panel, "Personal");
    cx.update_window(handle, |_, window, cx| {
        let filter = panel.read(cx).filter.read(cx).focus_handle(cx);
        let input = panel.read(cx).renaming.as_ref().unwrap().input.clone();
        assert!(
            input.read(cx).focus_handle(cx).is_focused(window),
            "rename input focused"
        );
        window.focus(&filter, cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| window.render_frame(cx))
        .unwrap();
    cx.run_until_parked();
    let profiles = profiles_of(cx);
    assert_eq!(
        profiles.get(a.id).unwrap().group.as_deref(),
        Some("Personal")
    );
    assert_eq!(profiles.groups(), ["Personal"]);
}

#[gpui_kit::test]
fn header_click_toggles_once_and_hover_buttons_do_not_double_fire(cx: &mut TestAppContext) {
    let (handle, panel, _) = setup(cx, &[gp("a", Some("Work"))]);
    cx.update_window(handle, |_, window, cx| {
        window.click("group-Work", cx);
        assert!(panel.read(cx).collapsed.contains("Work"));
        window.hover("group-Work", cx);
        window.render_frame(cx);
        window.click("group-toggle-Work", cx);
        assert!(!panel.read(cx).collapsed.contains("Work"));
        assert!(!window.has_active_dialog(cx));
    })
    .unwrap();
}

fn open_group_dialog(cx: &mut TestAppContext, handle: gpui_kit::AnyWindowHandle) {
    cx.update_window(handle, |_, window, cx| {
        window.hover("group-Work", cx);
        window.render_frame(cx);
        window.click("group-delete-Work", cx);
        window.render_frame(cx);
        assert!(window.has_active_dialog(cx));
    })
    .unwrap();
    // Let the dialog's entrance animation finish so button hit boxes are final.
    std::thread::sleep(
        *gpui_kit::component::dialog::ANIMATION_DURATION + std::time::Duration::from_millis(50),
    );
    cx.update_window(handle, |_, window, cx| window.render_frame(cx))
        .unwrap();
}

#[gpui_kit::test]
fn delete_dialog_cancel_escape_and_enter_change_nothing(cx: &mut TestAppContext) {
    let (handle, _, _) = setup(cx, &[gp("a", Some("Work"))]);
    let before = profiles_of(cx);
    open_group_dialog(cx, handle);
    cx.update_window(handle, |_, window, cx| {
        window.press("enter", cx);
        window.render_frame(cx);
    })
    .unwrap();
    cx.run_until_parked();
    assert_eq!(profiles_of(cx), before);
    cx.update_window(handle, |_, window, cx| {
        assert!(window.has_active_dialog(cx), "Enter must not dismiss it");
        window.press("escape", cx);
        window.render_frame(cx);
        assert!(!window.has_active_dialog(cx));
    })
    .unwrap();
    open_group_dialog(cx, handle);
    cx.update_window(handle, |_, window, cx| {
        window.click("group-delete-cancel", cx);
        window.render_frame(cx);
        assert!(!window.has_active_dialog(cx));
    })
    .unwrap();
    cx.run_until_parked();
    assert_eq!(profiles_of(cx), before);
}

#[gpui_kit::test]
fn delete_dialog_ungroups_or_deletes_members(cx: &mut TestAppContext) {
    let (a, b, c) = (
        gp("a", Some("Work")),
        gp("b", Some("Work")),
        gp("c", Some("Other")),
    );
    let (handle, _, opened) = setup(cx, &[a.clone(), b.clone(), c.clone()]);
    cx.update(|cx| {
        let spec = spec_for_profile(&a);
        Connections::global(cx).update(cx, |connections, cx| {
            connections.record_use(&spec, Some(a.id), cx)
        });
    });
    open_group_dialog(cx, handle);
    cx.update_window(handle, |_, window, cx| {
        window.click("group-delete-ungroup", cx);
        window.render_frame(cx);
        assert!(!window.has_active_dialog(cx));
    })
    .unwrap();
    cx.run_until_parked();
    let profiles = profiles_of(cx);
    assert_eq!(profiles.get(a.id).unwrap().group, None);
    assert_eq!(profiles.get(b.id).unwrap().group, None);
    assert_eq!(profiles.groups(), ["Other"]);

    let (a, b, c) = (gp("a", Some("Work")), gp("b", Some("Work")), c);
    cx.update_window(handle, |_, _, cx| {
        Connections::global(cx).update(cx, |connections, cx| {
            for p in [&a, &b] {
                connections.save_profile(p.clone(), cx).now_or_never();
            }
            connections.record_use(&spec_for_profile(&a), Some(a.id), cx);
        });
    })
    .unwrap();
    cx.run_until_parked();
    open_group_dialog(cx, handle);
    cx.update_window(handle, |_, window, cx| {
        window.click("group-delete-all", cx);
        window.render_frame(cx);
        assert!(!window.has_active_dialog(cx));
    })
    .unwrap();
    cx.run_until_parked();
    let profiles = profiles_of(cx);
    assert!(profiles.get(a.id).is_none() && profiles.get(b.id).is_none());
    assert!(profiles.get(c.id).is_some());
    assert_eq!(profiles.groups(), ["Other"]);
    cx.read(|cx| {
        let connections = Connections::global(cx);
        assert!(
            connections
                .read(cx)
                .recents()
                .iter()
                .all(|recent| recent.profile != Some(a.id))
        );
    });
    assert!(opened.borrow().is_empty());
}

#[gpui_kit::test]
fn empty_group_dialog_offers_only_delete(cx: &mut TestAppContext) {
    let item = gp("a", Some("Work"));
    let (handle, _, _) = setup(cx, std::slice::from_ref(&item));
    cx.update_window(handle, |_, _, cx| {
        Connections::global(cx).update(cx, |connections, cx| {
            connections.move_profile(item.id, None, cx).now_or_never();
        });
    })
    .unwrap();
    cx.run_until_parked();
    open_group_dialog(cx, handle);
    cx.update_window(handle, |_, window, cx| {
        assert!(window.try_find("group-delete-ungroup").is_none());
        window.click("group-delete-all", cx);
    })
    .unwrap();
    cx.run_until_parked();
    assert!(profiles_of(cx).groups().is_empty());
}

fn collapse(cx: &mut TestAppContext, handle: gpui_kit::AnyWindowHandle, group: &str) {
    cx.update_window(handle, |_, window, cx| {
        window.click(SharedString::from(format!("group-{group}")), cx);
        window.render_frame(cx);
    })
    .unwrap();
}

fn commit_rename(
    cx: &mut TestAppContext,
    handle: gpui_kit::AnyWindowHandle,
    panel: &Entity<ConnectionsPanel>,
    value: &str,
) {
    set_rename_value(cx, handle, panel, value);
    cx.update_window(handle, |_, window, cx| window.press("enter", cx))
        .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| window.render_frame(cx))
        .unwrap();
}

fn collapsed_of(cx: &mut TestAppContext, panel: &Entity<ConnectionsPanel>) -> Vec<String> {
    let mut names: Vec<String> = cx.read(|cx| panel.read(cx).collapsed.iter().cloned().collect());
    names.sort();
    names
}

#[gpui_kit::test]
fn merge_rename_collapsed_state_follows_the_renamed_group(cx: &mut TestAppContext) {
    let (a, b) = (gp("a", Some("Work")), gp("b", Some("Personal")));
    let (handle, panel, _) = setup(cx, &[a.clone(), b.clone()]);
    // Work folded, Personal open: the merged group stays folded.
    collapse(cx, handle, "Work");
    start_rename_via_double_click(cx, handle, "Work");
    commit_rename(cx, handle, &panel, "Personal");
    assert_eq!(collapsed_of(cx, &panel), ["Personal"]);
    assert_eq!(profiles_of(cx).groups(), ["Personal"]);

    // Work open, Personal folded: the merged group opens.
    let c = gp("c", Some("Work"));
    cx.update_window(handle, |_, _, cx| {
        Connections::global(cx).update(cx, |connections, cx| {
            connections.save_profile(c.clone(), cx).now_or_never();
        });
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| window.render_frame(cx))
        .unwrap();
    assert_eq!(collapsed_of(cx, &panel), ["Personal"]);
    start_rename_via_double_click(cx, handle, "Work");
    commit_rename(cx, handle, &panel, "Personal");
    assert!(collapsed_of(cx, &panel).is_empty());
    assert_eq!(profiles_of(cx).in_group(Some("Personal"), "").len(), 3);
}

#[gpui_kit::test]
fn rename_in_progress_is_dropped_when_its_group_is_deleted(cx: &mut TestAppContext) {
    let (a, b) = (gp("a", Some("Work")), gp("b", Some("Personal")));
    let (handle, panel, _) = setup(cx, &[a.clone(), b.clone()]);
    collapse(cx, handle, "Personal");
    start_rename_via_double_click(cx, handle, "Work");
    // The group disappears while its name is being edited.
    cx.update_window(handle, |_, _, cx| {
        Connections::global(cx).update(cx, |connections, cx| {
            connections
                .delete_group("Work".into(), vec![a.id], cx)
                .now_or_never()
                .unwrap()
                .unwrap();
        });
    })
    .unwrap();
    cx.run_until_parked();
    // The stale rename is dropped, so nothing can be committed later.
    cx.read(|cx| assert!(panel.read(cx).renaming.is_none()));
    assert_eq!(collapsed_of(cx, &panel), ["Personal"]);
    let profiles = profiles_of(cx);
    assert_eq!(profiles.groups(), ["Personal"]);
    assert!(profiles.get(a.id).is_none() && profiles.get(b.id).is_some());
}

#[gpui_kit::test]
fn rename_keeps_unicode_and_interior_whitespace_and_trims_unicode_space(cx: &mut TestAppContext) {
    let a = gp("a", Some("Work"));
    let (handle, panel, _) = setup(cx, std::slice::from_ref(&a));
    // Only Unicode whitespace trims to nothing: a no-op.
    start_rename_via_double_click(cx, handle, "Work");
    commit_rename(cx, handle, &panel, "\u{3000} \u{a0}");
    assert_eq!(
        profiles_of(cx).get(a.id).unwrap().group.as_deref(),
        Some("Work")
    );
    start_rename_via_double_click(cx, handle, "Work");
    commit_rename(cx, handle, &panel, "\u{3000}Zürich  \u{1f680} 東京\u{a0}");
    assert_eq!(
        profiles_of(cx).get(a.id).unwrap().group.as_deref(),
        Some("Zürich  \u{1f680} 東京")
    );
    // Names differ only by case: a new group, not a merge.
    cx.update_window(handle, |_, window, cx| window.render_frame(cx))
        .unwrap();
    start_rename_via_double_click(cx, handle, "Zürich  \u{1f680} 東京");
    commit_rename(cx, handle, &panel, "ZÜRICH  \u{1f680} 東京");
    assert_eq!(
        profiles_of(cx).get(a.id).unwrap().group.as_deref(),
        Some("ZÜRICH  \u{1f680} 東京")
    );
}

#[gpui_kit::test]
fn rename_with_active_filter_moves_hidden_members_too(cx: &mut TestAppContext) {
    let (a, b) = (gp("alpha", Some("Work")), gp("zulu", Some("Work")));
    let (handle, panel, _) = setup(cx, &[a.clone(), b.clone()]);
    cx.update_window(handle, |_, window, cx| {
        let filter = panel.read(cx).filter.clone();
        filter.update(cx, |input, cx| input.set_value("alp", window, cx));
        window.render_frame(cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| window.render_frame(cx))
        .unwrap();
    start_rename_via_double_click(cx, handle, "Work");
    commit_rename(cx, handle, &panel, "Team");
    let profiles = profiles_of(cx);
    assert_eq!(profiles.get(a.id).unwrap().group.as_deref(), Some("Team"));
    assert_eq!(profiles.get(b.id).unwrap().group.as_deref(), Some("Team"));
    assert_eq!(profiles.groups(), ["Team"]);
}

#[gpui_kit::test]
fn header_toggle_button_round_trips(cx: &mut TestAppContext) {
    let (handle, panel, opened) = setup(cx, &[gp("a", Some("Work"))]);
    for expected in [true, false, true] {
        cx.update_window(handle, |_, window, cx| {
            window.hover("group-Work", cx);
            window.render_frame(cx);
            window.click("group-toggle-Work", cx);
            window.render_frame(cx);
            assert_eq!(panel.read(cx).collapsed.contains("Work"), expected);
        })
        .unwrap();
    }
    assert!(opened.borrow().is_empty());
}

#[gpui_kit::test]
fn double_click_on_name_leaves_collapsed_state_unchanged(cx: &mut TestAppContext) {
    let (handle, panel, _) = setup(cx, &[gp("a", Some("Work"))]);
    for _ in 0..2 {
        // A real double click is click counts 1 and 2: the first toggles,
        // the second undoes it and starts the rename.
        start_rename_via_double_click(cx, handle, "Work");
        assert!(collapsed_of(cx, &panel).is_empty());
        cx.update_window(handle, |_, window, cx| window.press("escape", cx))
            .unwrap();
        cx.run_until_parked();
        cx.update_window(handle, |_, window, cx| window.render_frame(cx))
            .unwrap();
    }
    assert!(collapsed_of(cx, &panel).is_empty());
}

#[gpui_kit::test]
fn rename_input_is_dropped_when_its_group_disappears(cx: &mut TestAppContext) {
    let (handle, panel, _) = setup(cx, &[gp("a", Some("Work"))]);
    start_rename_via_double_click(cx, handle, "Work");
    cx.update_window(handle, |_, _, cx| {
        Connections::global(cx).update(cx, |connections, cx| {
            connections.ungroup("Work".into(), cx).now_or_never();
        });
    })
    .unwrap();
    cx.run_until_parked();
    cx.read(|cx| assert!(panel.read(cx).renaming.is_none()));
}

#[gpui_kit::test]
fn failed_merge_rename_restores_the_target_collapsed_state(cx: &mut TestAppContext) {
    let (a, b) = (gp("a", Some("Work")), gp("b", Some("Personal")));
    let (handle, panel, _) = setup(cx, &[a, b]);
    let before = profiles_of(cx);
    collapse(cx, handle, "Personal");
    let directory = tempfile::tempdir().unwrap();
    cx.update(|cx| {
        Connections::global(cx).update(cx, |connections, _| {
            connections.set_profiles_file_for_test(Some(directory.path().into()))
        })
    });
    // Work is open, so the optimistic merge unfolds Personal until it fails.
    start_rename_via_double_click(cx, handle, "Work");
    commit_rename(cx, handle, &panel, "Personal");
    cx.run_until_parked();
    assert_eq!(collapsed_of(cx, &panel), ["Personal"]);
    assert_eq!(profiles_of(cx), before);
}

#[gpui_kit::test]
fn failed_rename_rolls_back_collapsed_state_and_profiles(cx: &mut TestAppContext) {
    let (handle, panel, _) = setup(cx, &[gp("a", Some("Work"))]);
    let before = profiles_of(cx);
    collapse(cx, handle, "Work");
    // Saving fails: the profiles path is a directory.
    let directory = tempfile::tempdir().unwrap();
    cx.update(|cx| {
        Connections::global(cx).update(cx, |connections, _| {
            connections.set_profiles_file_for_test(Some(directory.path().into()))
        })
    });
    start_rename_via_double_click(cx, handle, "Work");
    commit_rename(cx, handle, &panel, "Team");
    cx.run_until_parked();
    assert_eq!(collapsed_of(cx, &panel), ["Work"]);
    assert_eq!(profiles_of(cx), before);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("group-Work").is_some());
        assert!(window.try_find("group-Team").is_none());
    })
    .unwrap();
}
