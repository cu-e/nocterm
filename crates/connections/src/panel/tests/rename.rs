use super::*;

#[gpui_kit::test]
fn double_click_renames_group_keeping_collapsed_state(cx: &mut TestAppContext) {
    let a = gp("a", Some("Work"));
    let (handle, panel, opened, installed) = setup(cx, std::slice::from_ref(&a));
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
        profiles_of(&installed, cx)
            .get(a.id)
            .unwrap()
            .group
            .as_deref(),
        Some("Team")
    );
    assert!(opened.borrow().is_empty());
}

#[gpui_kit::test]
fn rename_escape_cancels_and_noop_values_change_nothing(cx: &mut TestAppContext) {
    let (handle, panel, _, installed) = setup(cx, &[gp("a", Some("Work"))]);
    let before = profiles_of(&installed, cx);
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
        assert_eq!(profiles_of(&installed, cx), before, "{value:?}");
    }
}

#[gpui_kit::test]
fn rename_commits_on_blur_and_merges_into_existing_group(cx: &mut TestAppContext) {
    let (a, b) = (gp("a", Some("Work")), gp("b", Some("Personal")));
    let (handle, panel, _, installed) = setup(cx, &[a.clone(), b.clone()]);
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
    let profiles = profiles_of(&installed, cx);
    assert_eq!(
        profiles.get(a.id).unwrap().group.as_deref(),
        Some("Personal")
    );
    assert_eq!(profiles.groups(), ["Personal"]);
}

#[gpui_kit::test]
fn merge_rename_collapsed_state_follows_the_renamed_group(cx: &mut TestAppContext) {
    let (a, b) = (gp("a", Some("Work")), gp("b", Some("Personal")));
    let (handle, panel, _, installed) = setup(cx, &[a.clone(), b.clone()]);
    // Work folded, Personal open: the merged group stays folded.
    collapse(cx, handle, "Work");
    start_rename_via_double_click(cx, handle, "Work");
    commit_rename(cx, handle, &panel, "Personal");
    assert_eq!(collapsed_of(cx, &panel), ["Personal"]);
    assert_eq!(profiles_of(&installed, cx).groups(), ["Personal"]);

    // Work open, Personal folded: the merged group opens.
    let c = gp("c", Some("Work"));
    cx.update_window(handle, |_, _, cx| {
        installed.connections().update(cx, |connections, cx| {
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
    assert_eq!(
        profiles_of(&installed, cx)
            .in_group(Some("Personal"), "")
            .len(),
        3
    );
}

#[gpui_kit::test]
fn rename_in_progress_is_dropped_when_its_group_is_deleted(cx: &mut TestAppContext) {
    let (a, b) = (gp("a", Some("Work")), gp("b", Some("Personal")));
    let (handle, panel, _, installed) = setup(cx, &[a.clone(), b.clone()]);
    collapse(cx, handle, "Personal");
    start_rename_via_double_click(cx, handle, "Work");
    // The group disappears while its name is being edited.
    cx.update_window(handle, |_, _, cx| {
        installed.connections().update(cx, |connections, cx| {
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
    let profiles = profiles_of(&installed, cx);
    assert_eq!(profiles.groups(), ["Personal"]);
    assert!(profiles.get(a.id).is_none() && profiles.get(b.id).is_some());
}

#[gpui_kit::test]
fn rename_keeps_unicode_and_interior_whitespace_and_trims_unicode_space(cx: &mut TestAppContext) {
    let a = gp("a", Some("Work"));
    let (handle, panel, _, installed) = setup(cx, std::slice::from_ref(&a));
    // Only Unicode whitespace trims to nothing: a no-op.
    start_rename_via_double_click(cx, handle, "Work");
    commit_rename(cx, handle, &panel, "\u{3000} \u{a0}");
    assert_eq!(
        profiles_of(&installed, cx)
            .get(a.id)
            .unwrap()
            .group
            .as_deref(),
        Some("Work")
    );
    start_rename_via_double_click(cx, handle, "Work");
    commit_rename(cx, handle, &panel, "\u{3000}Zürich  \u{1f680} 東京\u{a0}");
    assert_eq!(
        profiles_of(&installed, cx)
            .get(a.id)
            .unwrap()
            .group
            .as_deref(),
        Some("Zürich  \u{1f680} 東京")
    );
    // Names differ only by case: a new group, not a merge.
    cx.update_window(handle, |_, window, cx| window.render_frame(cx))
        .unwrap();
    start_rename_via_double_click(cx, handle, "Zürich  \u{1f680} 東京");
    commit_rename(cx, handle, &panel, "ZÜRICH  \u{1f680} 東京");
    assert_eq!(
        profiles_of(&installed, cx)
            .get(a.id)
            .unwrap()
            .group
            .as_deref(),
        Some("ZÜRICH  \u{1f680} 東京")
    );
}

#[gpui_kit::test]
fn rename_with_active_filter_moves_hidden_members_too(cx: &mut TestAppContext) {
    let (a, b) = (gp("alpha", Some("Work")), gp("zulu", Some("Work")));
    let (handle, panel, _, installed) = setup(cx, &[a.clone(), b.clone()]);
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
    let profiles = profiles_of(&installed, cx);
    assert_eq!(profiles.get(a.id).unwrap().group.as_deref(), Some("Team"));
    assert_eq!(profiles.get(b.id).unwrap().group.as_deref(), Some("Team"));
    assert_eq!(profiles.groups(), ["Team"]);
}

#[gpui_kit::test]
fn rename_input_is_dropped_when_its_group_disappears(cx: &mut TestAppContext) {
    let (handle, panel, _, installed) = setup(cx, &[gp("a", Some("Work"))]);
    start_rename_via_double_click(cx, handle, "Work");
    cx.update_window(handle, |_, _, cx| {
        installed.connections().update(cx, |connections, cx| {
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
    let (handle, panel, _, installed) =
        setup_with_writer(cx, &[a, b], Some(failing_profile_writer()));
    let before = profiles_of(&installed, cx);
    collapse(cx, handle, "Personal");
    start_rename_via_double_click(cx, handle, "Work");
    commit_rename(cx, handle, &panel, "Personal");
    cx.run_until_parked();
    assert_eq!(collapsed_of(cx, &panel), ["Personal"]);
    assert_eq!(profiles_of(&installed, cx), before);
}

#[gpui_kit::test]
fn failed_rename_rolls_back_collapsed_state_and_profiles(cx: &mut TestAppContext) {
    let (handle, panel, _, installed) =
        setup_with_writer(cx, &[gp("a", Some("Work"))], Some(failing_profile_writer()));
    let before = profiles_of(&installed, cx);
    collapse(cx, handle, "Work");
    // Saving fails: the profiles path is a directory.
    start_rename_via_double_click(cx, handle, "Work");
    commit_rename(cx, handle, &panel, "Team");
    cx.run_until_parked();
    assert_eq!(collapsed_of(cx, &panel), ["Work"]);
    assert_eq!(profiles_of(&installed, cx), before);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("group-Work").is_some());
        assert!(window.try_find("group-Team").is_none());
    })
    .unwrap();
}
