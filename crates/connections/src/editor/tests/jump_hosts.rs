//! The "Jump Hosts" section of the connection form, driven through its controls.
use super::*;
use futures::FutureExt as _;
use nocterm_connections_model::jump_route::JumpHostEntry;

fn saved(cx: &mut TestAppContext, installed: &crate::Installed, name: &str, host: &str) -> Profile {
    let mut profile = build_profile(
        nocterm_connections_model::test_support::profile_id_for(name),
        &fields(host),
        AuthKind::Auto,
        Some("tester"),
        None,
    )
    .unwrap();
    profile.name = name.into();
    store(cx, installed, &profile);
    profile
}

fn store(cx: &mut TestAppContext, installed: &crate::Installed, profile: &Profile) {
    cx.update(|cx| {
        installed.connections().update(cx, |connections, cx| {
            connections
                .save_profile(profile.clone(), cx)
                .now_or_never()
                .unwrap()
                .unwrap();
        });
    });
}

fn stored(
    cx: &mut TestAppContext,
    installed: &crate::Installed,
    id: nocterm_connections_model::profiles::ProfileId,
) -> Profile {
    cx.read(|cx| {
        installed
            .connections()
            .read(cx)
            .profiles(cx)
            .get(id)
            .cloned()
    })
    .unwrap()
}

fn open(
    cx: &mut TestAppContext,
    handle: gpui_kit::AnyWindowHandle,
    workspace: &gpui_kit::Entity<nocterm_workspace::Workspace>,
    installed: &crate::Installed,
    profile: Option<Profile>,
) {
    cx.simulate_window_resize(handle, gpui_kit::size(px(900.), px(900.)));
    let profile_is_new = profile.is_none();
    cx.update_window(handle, |_, window, cx| {
        open_editor(
            installed.clone(),
            profile,
            workspace.downgrade(),
            window,
            cx,
        );
        window.render_frame(cx);
        if profile_is_new {
            window.input("inner.test", cx);
        }
        window.click("editor-section-jump-hosts", cx);
    })
    .unwrap();
    cx.run_until_parked();
}

/// Runs `steps` against a freshly rendered window, then lets effects settle.
fn act(
    cx: &mut TestAppContext,
    handle: gpui_kit::AnyWindowHandle,
    steps: impl FnOnce(&mut gpui_kit::Window, &mut gpui_kit::App),
) {
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        steps(window, cx);
    })
    .unwrap();
    cx.run_until_parked();
}

fn type_address(window: &mut gpui_kit::Window, text: &str, cx: &mut gpui_kit::App) {
    window.click("editor-field-Jump host address", cx);
    window.input(text, cx);
}

fn pick_first(window: &mut gpui_kit::Window, cx: &mut gpui_kit::App) {
    window.within("jump-host-picker").click("input", cx);
    window.press("down", cx);
    window.press("enter", cx);
}

#[gpui_kit::test]
fn hops_are_added_reordered_removed_and_saved_in_order(cx: &mut TestAppContext) {
    let (handle, workspace, _opened, installed) = crate::test_support::workspace(cx);
    let bastion = saved(cx, &installed, "Bastion", "bastion.test");
    open(cx, handle, &workspace, &installed, None);
    act(cx, handle, |window, cx| {
        type_address(window, "ops@typed.test:2200", cx);
        window.click("jump-host-add-address", cx);
    });
    act(cx, handle, |window, cx| {
        assert!(
            window.try_find(("jump-host", 0usize)).is_some(),
            "address added"
        );
        pick_first(window, cx);
    });
    act(cx, handle, |window, cx| {
        window.click("jump-host-add-profile", cx);
    });
    act(cx, handle, |window, _| {
        assert!(
            window.try_find(("jump-host", 1usize)).is_some(),
            "profile added"
        );
    });
    act(cx, handle, |window, cx| {
        type_address(window, "extra.test", cx);
        window.click("jump-host-add-address", cx);
    });
    act(cx, handle, |window, cx| {
        window.click(("jump-host-up", 1usize), cx);
    });
    act(cx, handle, |window, cx| {
        window.click(("jump-host-down", 0usize), cx);
    });
    act(cx, handle, |window, cx| {
        window.click(("jump-host-remove", 2usize), cx);
    });
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("editor-save", cx);
    })
    .unwrap();
    cx.run_until_parked();
    let inner = cx.read(|cx| {
        installed
            .connections()
            .read(cx)
            .profiles(cx)
            .iter()
            .find(|profile| profile.target.host == "inner.test")
            .cloned()
    });
    let inner = inner.expect("the connection was saved");
    assert_eq!(
        inner.jump_hosts,
        [
            JumpHostEntry::Address(Target::new("ops", "typed.test", 2200)),
            JumpHostEntry::Profile(bastion.id),
        ]
    );
}

#[gpui_kit::test]
fn enter_in_the_address_field_adds_the_hop_instead_of_saving(cx: &mut TestAppContext) {
    let (handle, workspace, opened, installed) = crate::test_support::workspace(cx);
    open(cx, handle, &workspace, &installed, None);
    act(cx, handle, |window, cx| {
        type_address(window, "ops@typed.test", cx);
        window.press("enter", cx);
    });
    act(cx, handle, |window, _| {
        assert!(window.try_find("dialog").is_some(), "the form stays open");
        assert_eq!(
            window.find(("jump-host", 0usize)).label(),
            Some("1. ops@typed.test")
        );
    });
    assert!(opened.borrow().is_empty());
    assert!(cx.read(|cx| installed.connections().read(cx).profiles(cx).is_empty()));
}

#[gpui_kit::test]
fn an_unparsable_address_adds_nothing_and_keeps_the_draft_focused(cx: &mut TestAppContext) {
    let (handle, workspace, _opened, installed) = crate::test_support::workspace(cx);
    open(cx, handle, &workspace, &installed, None);
    act(cx, handle, |window, cx| {
        type_address(window, "ops@bad host", cx);
        window.click("jump-host-add-address", cx);
    });
    act(cx, handle, |window, cx| {
        assert!(window.try_find(("jump-host", 0usize)).is_none());
        assert_eq!(
            window.focused_input(cx).unwrap().value(cx).as_ref(),
            "ops@bad host"
        );
    });
}

#[gpui_kit::test]
fn the_picker_never_offers_the_connection_being_edited(cx: &mut TestAppContext) {
    let (handle, workspace, _opened, installed) = crate::test_support::workspace(cx);
    let edited = saved(cx, &installed, "Edited", "edited.test");
    let other = saved(cx, &installed, "Other", "other.test");
    open(cx, handle, &workspace, &installed, Some(edited.clone()));
    act(cx, handle, pick_first);
    act(cx, handle, |window, cx| {
        window.click("jump-host-add-profile", cx);
    });
    act(cx, handle, |window, cx| {
        window.click("editor-save", cx);
    });
    assert_eq!(
        stored(cx, &installed, edited.id).jump_hosts,
        [JumpHostEntry::Profile(other.id)]
    );
}

#[gpui_kit::test]
fn a_hop_that_would_loop_back_blocks_saving_and_shows_the_route(cx: &mut TestAppContext) {
    let (handle, workspace, _opened, installed) = crate::test_support::workspace(cx);
    let edited = saved(cx, &installed, "Edited", "edited.test");
    let mut other = saved(cx, &installed, "Other", "other.test");
    other.jump_hosts = vec![JumpHostEntry::Profile(edited.id)];
    store(cx, &installed, &other);
    open(cx, handle, &workspace, &installed, Some(edited.clone()));
    act(cx, handle, pick_first);
    act(cx, handle, |window, cx| {
        window.click("jump-host-add-profile", cx);
    });
    act(cx, handle, |window, cx| {
        window.click("editor-section-connection", cx);
    });
    act(cx, handle, |window, cx| {
        assert!(window.try_find("jump-host-add-address").is_none());
        window.click("editor-save", cx);
    });
    act(cx, handle, |window, _| {
        assert!(window.try_find("dialog").is_some(), "saving was refused");
        assert!(
            window.try_find("jump-host-add-address").is_some(),
            "the Jump Hosts section is shown"
        );
    });
    assert_eq!(stored(cx, &installed, edited.id), edited);
}

#[gpui_kit::test]
fn a_deleted_hop_connection_is_marked_and_blocks_saving_until_removed(cx: &mut TestAppContext) {
    let (handle, workspace, _opened, installed) = crate::test_support::workspace(cx);
    let gone = nocterm_connections_model::test_support::profile_id_for("Gone");
    let mut edited = saved(cx, &installed, "Edited", "edited.test");
    edited.jump_hosts = vec![JumpHostEntry::Profile(gone)];
    store(cx, &installed, &edited);
    open(cx, handle, &workspace, &installed, Some(edited.clone()));
    act(cx, handle, |window, cx| {
        assert_eq!(
            window.find(("jump-host", 0usize)).label(),
            Some("1. Deleted connection")
        );
        window.click("editor-save", cx);
    });
    act(cx, handle, |window, cx| {
        assert!(window.try_find("dialog").is_some(), "saving was refused");
        window.click(("jump-host-remove", 0usize), cx);
    });
    act(cx, handle, |window, cx| {
        window.click("editor-save", cx);
    });
    act(cx, handle, |window, _| {
        assert!(
            window.try_find("dialog").is_none(),
            "saved once it is removed"
        );
    });
    assert!(stored(cx, &installed, edited.id).jump_hosts.is_empty());
}

#[gpui_kit::test]
fn save_and_connect_opens_the_session_with_the_resolved_route(cx: &mut TestAppContext) {
    let (handle, workspace, opened, installed) = crate::test_support::workspace(cx);
    let bastion = saved(cx, &installed, "Bastion", "bastion.test");
    open(cx, handle, &workspace, &installed, None);
    act(cx, handle, pick_first);
    act(cx, handle, |window, cx| {
        window.click("jump-host-add-profile", cx);
    });
    act(cx, handle, |window, cx| {
        window.click("editor-save-connect", cx);
    });
    let opened = opened.borrow();
    assert_eq!(opened.len(), 1);
    assert_eq!(opened[0].target.host, "inner.test");
    assert_eq!(opened[0].jumps.len(), 1);
    assert_eq!(opened[0].jumps[0].host.target, bastion.target);
}

#[gpui_kit::test]
fn a_connection_without_jump_hosts_is_saved_unchanged(cx: &mut TestAppContext) {
    let (handle, workspace, _opened, installed) = crate::test_support::workspace(cx);
    let direct = saved(cx, &installed, "Direct", "direct.test");
    open(cx, handle, &workspace, &installed, Some(direct.clone()));
    act(cx, handle, |window, cx| {
        window.click("editor-save", cx);
    });
    let after = stored(cx, &installed, direct.id);
    assert_eq!(after, direct);
    assert_eq!(
        toml::to_string(&after).unwrap(),
        toml::to_string(&direct).unwrap()
    );
    assert!(!toml::to_string(&after).unwrap().contains("jump"));
}

#[gpui_kit::test]
fn connections_that_read_alike_stay_distinct_choices_in_the_picker(cx: &mut TestAppContext) {
    let (_handle, _workspace, _opened, installed) = crate::test_support::workspace(cx);
    let first = saved(cx, &installed, "Twin", "twin.test");
    let mut second = saved(cx, &installed, "Twin copy", "twin.test");
    second.name = "Twin".into();
    store(cx, &installed, &second);
    let own = nocterm_connections_model::test_support::profile_id_for("Edited");
    let choices = cx.read(|cx| {
        super::super::jump_hosts::JumpHostsFields::choices(
            own,
            installed.connections().read(cx).profiles(cx),
        )
    });
    let labels: std::collections::HashSet<_> = choices.iter().map(|(label, _)| label).collect();
    assert_eq!(labels.len(), 2, "each label selects exactly one connection");
    let ids: Vec<_> = choices.iter().map(|(_, id)| *id).collect();
    assert!(ids.contains(&first.id) && ids.contains(&second.id));
}
