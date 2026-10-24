//! The panel groups owner: what it hosts, its transitions and the focus they ask for, and a
//! hosted panel's cached content, which invalidates independently of the chrome.
use crate::test_support::TestPanel;
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{TestAppContext, prelude::*};
use std::{cell::Cell, rc::Rc};

#[gpui_kit::test]
fn cached_panel_skips_workspace_redraw_but_updates_its_content_and_badge(cx: &mut TestAppContext) {
    let (handle, workspace) = crate::workspace::tests::fixture(cx);
    let renders = Rc::new(Cell::new(0));
    let observed = Rc::new(Cell::new(0));
    let panel = cx
        .update_window(handle, |_, window, cx| {
            let panel = TestPanel::builder("probe")
                .title("Probe")
                .text("First")
                .render_counter(renders.clone())
                .build(cx);
            workspace.update(cx, |w, cx| {
                w.add_panel(panel.clone(), cx);
                w.activate_panel(0, window, cx);
            });
            window.render_frame(cx);
            panel
        })
        .unwrap();
    // Registration/activation effects are flushed after the initial draw.
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| window.draw(cx).clear(cx))
        .unwrap();
    cx.run_until_parked();
    let count = renders.get();
    let log = observed.clone();
    let _subscription = cx.update(|cx| {
        cx.observe(&workspace, move |_, _| {
            log.set(log.get() + 1);
        })
    });
    cx.update(|cx| workspace.update(cx, |_, cx| cx.notify()));
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.draw(cx).clear(cx);
    })
    .unwrap();
    assert_eq!(
        renders.get(),
        count,
        "workspace notification should reuse the panel subtree"
    );
    let before = observed.get();
    cx.update(|cx| panel.update(cx, |panel, cx| panel.set_text("Second", cx)));
    cx.run_until_parked();
    assert_eq!(
        observed.get(),
        before,
        "content-only notifications do not affect workspace chrome"
    );
    cx.update_window(handle, |_, window, cx| {
        window.draw(cx).clear(cx);
    })
    .unwrap();
    assert!(renders.get() > count, "changed panel content must repaint");
    cx.update(|cx| panel.update(cx, |panel, cx| panel.set_badge(Some("1".into()), cx)));
    cx.run_until_parked();
    assert!(
        observed.get() > before,
        "changed badge must update the footer"
    );
}

#[test]
fn the_shells_group_declarations_name_the_group_keys_in_order() {
    use super::GroupKey;
    let declared: Vec<_> = crate::plugin::GROUPS
        .iter()
        .map(|spec| spec.id.as_str())
        .collect();
    let keys: Vec<_> = GroupKey::ALL.iter().map(|key| key.id()).collect();
    assert_eq!(declared, keys);
    assert_eq!(
        GroupKey::ALL.map(GroupKey::primary),
        [true, false],
        "only the sidebar is primary"
    );
}

/// Transitions of the owner itself, driven the way the workspace drives them.
mod owner {
    use super::super::{GroupKey, HostedPanel, PanelFocus, PanelGroups, PanelTransition};
    use crate::test_support::TestPanel;
    use gpui_kit::{
        App, AppContext as _, Entity, FocusHandle, Focusable as _, TestAppContext, Window,
    };

    fn navigation(cx: &mut App, id: &'static str) -> (Entity<TestPanel>, HostedPanel) {
        let panel = TestPanel::builder(id).title(id).build(cx);
        let subscription = cx.observe(&panel, |_, _| {});
        let presentation = (id.into(), nocterm_ui::IconName::Bot, None);
        let hosted = HostedPanel::plain(Box::new(panel.clone()), presentation, subscription);
        (panel, hosted)
    }

    fn side(cx: &mut App) -> (Entity<TestPanel>, HostedPanel) {
        let panel = TestPanel::builder("side").title("Side").build(cx);
        let subscription = cx.observe(&panel, |_, _| {});
        let presentation = ("Side".into(), nocterm_ui::IconName::PanelRight, None);
        let hosted = HostedPanel::expandable(Box::new(panel.clone()), presentation, subscription);
        (panel, hosted)
    }

    /// The handle a transition asks to focus, which must be a panel's.
    fn focused_panel(transition: PanelTransition) -> FocusHandle {
        match transition.into_focus() {
            PanelFocus::Panel(handle) => handle,
            _ => panic!("the transition focuses no panel"),
        }
    }

    fn focus_of(transition: PanelTransition) -> &'static str {
        match transition.into_focus() {
            PanelFocus::Preserve => "preserve",
            PanelFocus::Panel(_) => "panel",
            PanelFocus::Central => "central",
            PanelFocus::ActiveItem => "active item",
        }
    }

    /// A window to ask `PanelGroups` about focus in.
    fn with_window(cx: &mut TestAppContext, f: impl FnOnce(&mut Window, &mut App)) {
        let (handle, _workspace) = crate::workspace::tests::fixture(cx);
        cx.update_window(handle, |_, window, cx| f(window, cx))
            .unwrap();
    }

    /// Navigation with two panels and an installed, available side panel.
    fn shell(cx: &mut App) -> (PanelGroups, [Entity<TestPanel>; 3]) {
        let mut groups = PanelGroups::new();
        let (first, hosted) = navigation(cx, "first");
        groups.add(GroupKey::Navigation, hosted);
        let (second, hosted) = navigation(cx, "second");
        groups.add(GroupKey::Navigation, hosted);
        let (assistant, hosted) = side(cx);
        groups.install(GroupKey::Assistant, hosted);
        (groups, [first, second, assistant])
    }

    #[gpui_kit::test]
    fn the_first_panel_of_the_sidebar_is_shown_and_opening_another_replaces_it(
        cx: &mut TestAppContext,
    ) {
        with_window(cx, |_, cx| {
            let (mut groups, [_, second, _]) = shell(cx);
            assert!(groups.is_showing(GroupKey::Navigation, 0));
            assert!(!groups.is_showing(GroupKey::Navigation, 1));
            let transition = groups.activate(GroupKey::Navigation, 1, cx).unwrap();
            assert_eq!(transition.visibility_changed(), None);
            assert_eq!(focused_panel(transition), second.focus_handle(cx));
            assert!(!groups.is_showing(GroupKey::Navigation, 0));
            assert!(groups.is_showing(GroupKey::Navigation, 1));
            assert!(groups.activate(GroupKey::Navigation, 2, cx).is_none());
        });
    }

    #[gpui_kit::test]
    fn an_empty_sidebar_is_shown_and_toggles_without_panels(cx: &mut TestAppContext) {
        with_window(cx, |window, cx| {
            let mut groups = PanelGroups::new();
            assert!(groups.is_shown(GroupKey::Navigation));
            assert!(groups.selected(GroupKey::Navigation).is_none());
            assert_eq!(groups.next(GroupKey::Navigation), None);
            let hide = groups.toggle(GroupKey::Navigation, window, cx).unwrap();
            assert_eq!(hide.visibility_changed(), Some(GroupKey::Navigation));
            assert_eq!(focus_of(hide), "active item");
            let show = groups.toggle(GroupKey::Navigation, window, cx).unwrap();
            assert_eq!(focus_of(show), "preserve");
            assert!(groups.is_shown(GroupKey::Navigation));
        });
    }

    #[gpui_kit::test]
    fn next_wraps_while_shown_and_reveals_the_current_panel_while_hidden(cx: &mut TestAppContext) {
        with_window(cx, |window, cx| {
            let (mut groups, _) = shell(cx);
            assert_eq!(groups.next(GroupKey::Navigation), Some(1));
            groups.activate(GroupKey::Navigation, 1, cx);
            assert_eq!(groups.next(GroupKey::Navigation), Some(0));
            groups.toggle(GroupKey::Navigation, window, cx);
            assert_eq!(groups.next(GroupKey::Navigation), Some(1));
        });
    }

    #[gpui_kit::test]
    fn the_side_panel_opens_only_once_the_host_offers_it(cx: &mut TestAppContext) {
        with_window(cx, |window, cx| {
            let (mut groups, [_, _, assistant]) = shell(cx);
            assert!(!groups.is_available(GroupKey::Assistant));
            assert!(groups.toggle(GroupKey::Assistant, window, cx).is_none());
            let offered = groups
                .set_available(GroupKey::Assistant, true, window, cx)
                .unwrap();
            assert_eq!(offered.visibility_changed(), None);
            assert!(
                groups
                    .set_available(GroupKey::Assistant, true, window, cx)
                    .is_none(),
                "offering it again changes nothing"
            );
            let opened = groups.toggle(GroupKey::Assistant, window, cx).unwrap();
            assert_eq!(opened.visibility_changed(), Some(GroupKey::Assistant));
            assert_eq!(focused_panel(opened), assistant.focus_handle(cx));
            assert!(groups.is_shown(GroupKey::Assistant));
        });
    }

    #[gpui_kit::test]
    fn groups_are_shown_independently_of_each_other(cx: &mut TestAppContext) {
        with_window(cx, |window, cx| {
            let (mut groups, _) = shell(cx);
            groups.set_available(GroupKey::Assistant, true, window, cx);
            groups.toggle(GroupKey::Assistant, window, cx);
            groups.toggle(GroupKey::Navigation, window, cx);
            assert!(!groups.is_shown(GroupKey::Navigation));
            assert!(groups.is_shown(GroupKey::Assistant));
            groups.toggle(GroupKey::Navigation, window, cx);
            groups.toggle(GroupKey::Assistant, window, cx);
            assert!(groups.is_shown(GroupKey::Navigation));
            assert!(!groups.is_shown(GroupKey::Assistant));
        });
    }

    #[gpui_kit::test]
    fn only_a_shown_expandable_panel_expands_and_the_panel_is_told(cx: &mut TestAppContext) {
        with_window(cx, |window, cx| {
            let (mut groups, [first, _, assistant]) = shell(cx);
            groups.set_available(GroupKey::Assistant, true, window, cx);
            assert!(
                !groups.set_expanded(GroupKey::Assistant, true, cx),
                "hidden"
            );
            assert!(
                !groups.set_expanded(GroupKey::Navigation, true, cx),
                "a titled panel cannot expand"
            );
            groups.toggle(GroupKey::Assistant, window, cx);
            assert!(groups.set_expanded(GroupKey::Assistant, true, cx));
            assert!(!groups.set_expanded(GroupKey::Assistant, true, cx));
            assert_eq!(groups.expanded(), Some(GroupKey::Assistant));
            assert!(assistant.read(cx).maximized());
            assert!(!first.read(cx).maximized());
            assert!(groups.set_expanded(GroupKey::Assistant, false, cx));
            assert!(!assistant.read(cx).maximized());
            assert_eq!(groups.expanded(), None);
        });
    }

    #[gpui_kit::test]
    fn hiding_the_expanded_sidebar_focuses_the_expanded_panel(cx: &mut TestAppContext) {
        with_window(cx, |window, cx| {
            let (mut groups, [_, _, assistant]) = shell(cx);
            groups.set_available(GroupKey::Assistant, true, window, cx);
            groups.toggle(GroupKey::Assistant, window, cx);
            groups.set_expanded(GroupKey::Assistant, true, cx);
            let hide = groups.toggle(GroupKey::Navigation, window, cx).unwrap();
            assert_eq!(focused_panel(hide), assistant.focus_handle(cx));
        });
    }

    #[gpui_kit::test]
    fn closing_a_group_ends_its_expansion_and_closing_it_again_changes_nothing(
        cx: &mut TestAppContext,
    ) {
        with_window(cx, |window, cx| {
            let (mut groups, [_, _, assistant]) = shell(cx);
            groups.set_available(GroupKey::Assistant, true, window, cx);
            groups.toggle(GroupKey::Assistant, window, cx);
            groups.set_expanded(GroupKey::Assistant, true, cx);
            let closed = groups.close(GroupKey::Assistant, window, cx);
            assert_eq!(closed.visibility_changed(), Some(GroupKey::Assistant));
            assert_eq!(focus_of(closed), "preserve");
            assert!(!groups.is_shown(GroupKey::Assistant));
            assert!(!groups.is_expanded(GroupKey::Assistant));
            assert!(!assistant.read(cx).maximized());
            let again = groups.close(GroupKey::Assistant, window, cx);
            assert_eq!(again.visibility_changed(), None);
        });
    }

    #[gpui_kit::test]
    fn withdrawing_the_side_panel_closes_it_and_hides_its_toggle(cx: &mut TestAppContext) {
        with_window(cx, |window, cx| {
            let (mut groups, _) = shell(cx);
            groups.set_available(GroupKey::Assistant, true, window, cx);
            groups.toggle(GroupKey::Assistant, window, cx);
            groups.set_expanded(GroupKey::Assistant, true, cx);
            let withdrawn = groups
                .set_available(GroupKey::Assistant, false, window, cx)
                .unwrap();
            assert_eq!(withdrawn.visibility_changed(), Some(GroupKey::Assistant));
            assert!(!groups.is_available(GroupKey::Assistant));
            assert!(!groups.is_shown(GroupKey::Assistant));
            assert_eq!(groups.expanded(), None);
            assert!(
                groups
                    .set_available(GroupKey::Assistant, false, window, cx)
                    .is_none()
            );
        });
    }

    #[gpui_kit::test]
    fn a_replacement_side_panel_inherits_attention_and_keeps_availability(cx: &mut TestAppContext) {
        with_window(cx, |window, cx| {
            let (mut groups, _) = shell(cx);
            groups.set_available(GroupKey::Assistant, true, window, cx);
            assert!(groups.set_attention(GroupKey::Assistant, true));
            assert!(!groups.set_attention(GroupKey::Assistant, true));
            let (replacement, hosted) = side(cx);
            groups.install(GroupKey::Assistant, hosted);
            assert!(groups.attention(GroupKey::Assistant));
            assert!(groups.is_available(GroupKey::Assistant));
            groups.toggle(GroupKey::Assistant, window, cx);
            groups.set_expanded(GroupKey::Assistant, true, cx);
            assert!(replacement.read(cx).maximized());
        });
    }

    #[gpui_kit::test]
    fn only_expandable_panels_learn_which_side_they_are_on(cx: &mut TestAppContext) {
        with_window(cx, |_, cx| {
            let (groups, [first, _, assistant]) = shell(cx);
            groups.set_docked_left(true, cx);
            assert!(assistant.read(cx).docked_left());
            assert!(!first.read(cx).docked_left());
            groups.set_docked_left(false, cx);
            assert!(!assistant.read(cx).docked_left());
        });
    }

    #[gpui_kit::test]
    fn a_changed_presentation_is_reported_once(cx: &mut TestAppContext) {
        with_window(cx, |_, cx| {
            let (mut groups, [first, ..]) = shell(cx);
            let id = first.entity_id();
            let same = ("first".into(), nocterm_ui::IconName::Bot, None);
            assert!(!groups.presentation_changed(id, same));
            let badged = ("first".into(), nocterm_ui::IconName::Bot, Some("2".into()));
            assert!(groups.presentation_changed(id, badged.clone()));
            assert!(!groups.presentation_changed(id, badged));
            let unknown = TestPanel::builder("unknown").build(cx).entity_id();
            let other = ("x".into(), nocterm_ui::IconName::Bot, None);
            assert!(!groups.presentation_changed(unknown, other));
        });
    }

    #[gpui_kit::test]
    fn the_sidebar_finds_an_offered_panel_by_type(cx: &mut TestAppContext) {
        with_window(cx, |window, cx| {
            let (mut groups, _) = shell(cx);
            assert_eq!(groups.index_of::<TestPanel>(GroupKey::Navigation), Some(0));
            assert_eq!(groups.index_of::<TestPanel>(GroupKey::Assistant), None);
            groups.set_available(GroupKey::Assistant, true, window, cx);
            assert_eq!(groups.index_of::<TestPanel>(GroupKey::Assistant), Some(0));
            assert_eq!(groups.members(GroupKey::Navigation).count(), 2);
        });
    }

    fn id(name: &'static str) -> nocterm_plugin_model::ContributionId {
        nocterm_plugin_model::ContributionId::new(name)
    }

    #[gpui_kit::test]
    fn a_withdrawn_member_keeps_its_position_but_is_not_offered(cx: &mut TestAppContext) {
        with_window(cx, |window, cx| {
            let mut groups = PanelGroups::new();
            let ids = [id("test.a"), id("test.b"), id("test.c")];
            for (name, id) in ["a", "b", "c"].into_iter().zip(&ids) {
                let (_, hosted) = navigation(cx, name);
                groups.register(id.clone(), GroupKey::Navigation, Some(hosted));
            }
            groups.set_panel_available(&ids[1], false, window, cx);
            let offered: Vec<_> = groups
                .members(GroupKey::Navigation)
                .map(|(ix, _)| ix)
                .collect();
            assert_eq!(offered, [0, 2]);
            assert_eq!(groups.next(GroupKey::Navigation), Some(2));
            groups.activate(GroupKey::Navigation, 2, cx);
            assert_eq!(groups.next(GroupKey::Navigation), Some(0));
            assert!(groups.activate(GroupKey::Navigation, 1, cx).is_none());
            assert!(groups.toggle_panel(&ids[1], window, cx).is_none());
            assert!(groups.open_panel(&ids[1], cx).is_none());
            assert!(groups.is_showing(GroupKey::Navigation, 2));
        });
    }

    #[gpui_kit::test]
    fn a_registered_panel_is_offered_once_it_is_filled(cx: &mut TestAppContext) {
        with_window(cx, |window, cx| {
            let mut groups = PanelGroups::new();
            let panel = id("test.late");
            groups.register(panel.clone(), GroupKey::Navigation, None);
            assert_eq!(groups.members(GroupKey::Navigation).count(), 0);
            assert!(!groups.panel_is_available(&panel));
            assert_eq!(groups.next(GroupKey::Navigation), None);
            assert!(groups.open_panel(&panel, cx).is_none());
            let (_, hosted) = navigation(cx, "late");
            assert!(groups.fill(&panel, hosted));
            let offered: Vec<_> = groups
                .members(GroupKey::Navigation)
                .map(|(ix, _)| ix)
                .collect();
            assert_eq!(offered, [0]);
            assert!(groups.panel_is_open(&panel));
            let (_, again) = navigation(cx, "again");
            assert!(!groups.fill(&panel, again), "a panel fills its slot once");
            let (_, stray) = navigation(cx, "stray");
            assert!(!groups.fill(&id("test.unknown"), stray));
            assert!(groups.toggle_panel(&panel, window, cx).is_some());
        });
    }
}
