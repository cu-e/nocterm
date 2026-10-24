//! The panel groups owner: what it hosts, its transitions and the focus they ask for, and a
//! hosted panel's cached content, which invalidates independently of the chrome.
use crate::test_support::{PANEL_A, TestPanel};
use crate::workspace::tests::test_plugin::register_built;
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{TestAppContext, prelude::*};
use std::{cell::Cell, rc::Rc};

#[gpui_kit::test]
fn cached_panel_skips_workspace_redraw_but_updates_its_content_and_badge(cx: &mut TestAppContext) {
    let (handle, workspace, _) = crate::workspace::tests::test_plugin::fixture_with_test_plugin(cx);
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
                register_built(w, PANEL_A, &panel, window, cx);
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
    use crate::test_support::{
        CORE_PANEL, EXPANDABLE, PANEL_A, PANEL_B, TEST_CORE, TEST_PLUGIN, TRAY_PANEL, TestPanel,
    };
    use std::collections::BTreeSet;

    use gpui_kit::{
        App, AppContext as _, Entity, FocusHandle, Focusable as _, TestAppContext, Window,
    };
    use nocterm_plugin_model::{ContributionId, GroupId, Side, SystemContribution};
    use nocterm_window_layout::{
        Bar, BarItem, BuiltIn, GroupEntry, LayoutPreset, ResolvedLayout, resolve,
    };

    /// The default arrangement of the test catalog: `PANEL_A`, `PANEL_B` and `CORE_PANEL` in the
    /// sidebar in this order, `EXPANDABLE` in the assistant group and `TRAY_PANEL` left out.
    fn layout() -> ResolvedLayout {
        let catalog = crate::plugin::catalog(&[TEST_CORE], &[&TEST_PLUGIN]).expect("valid");
        BuiltIn::Default.resolve(&catalog, &BTreeSet::new())
    }

    fn id(contribution: &SystemContribution) -> ContributionId {
        contribution.id.clone()
    }

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

    fn assistant_id() -> ContributionId {
        id(EXPANDABLE)
    }

    /// Navigation with two panels and an expandable side panel the host does not offer yet.
    fn shell(window: &Window, cx: &mut App) -> (PanelGroups, [Entity<TestPanel>; 3]) {
        let (layout, mut groups) = (layout(), PanelGroups::new());
        let (first, hosted) = navigation(cx, "first");
        groups.register(id(PANEL_A), Some(hosted), &layout);
        let (second, hosted) = navigation(cx, "second");
        groups.register(id(PANEL_B), Some(hosted), &layout);
        let (assistant, hosted) = side(cx);
        groups.register(assistant_id(), Some(hosted), &layout);
        groups.set_panel_available(&assistant_id(), false, window, cx);
        (groups, [first, second, assistant])
    }

    #[gpui_kit::test]
    fn the_first_panel_of_the_sidebar_is_shown_and_opening_another_replaces_it(
        cx: &mut TestAppContext,
    ) {
        with_window(cx, |window, cx| {
            let (mut groups, [_, second, _]) = shell(window, cx);
            assert!(groups.panel_is_open(&id(PANEL_A)));
            assert!(!groups.panel_is_open(&id(PANEL_B)));
            let transition = groups.activate(GroupKey::Navigation, 1, cx).unwrap();
            assert_eq!(transition.visibility_changed(), None);
            assert_eq!(focused_panel(transition), second.focus_handle(cx));
            assert!(!groups.panel_is_open(&id(PANEL_A)));
            assert!(groups.panel_is_open(&id(PANEL_B)));
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
            assert_eq!(
                focus_of(hide),
                "preserve",
                "an empty sidebar holds no focus"
            );
            let show = groups.toggle(GroupKey::Navigation, window, cx).unwrap();
            assert_eq!(focus_of(show), "preserve");
            assert!(groups.is_shown(GroupKey::Navigation));
        });
    }

    #[gpui_kit::test]
    fn next_wraps_while_shown_and_reveals_the_current_panel_while_hidden(cx: &mut TestAppContext) {
        with_window(cx, |window, cx| {
            let (mut groups, _) = shell(window, cx);
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
            let (mut groups, [_, _, assistant]) = shell(window, cx);
            assert!(!groups.panel_is_available(&assistant_id()));
            assert!(groups.toggle(GroupKey::Assistant, window, cx).is_none());
            let offered = groups
                .set_panel_available(&assistant_id(), true, window, cx)
                .unwrap();
            assert_eq!(offered.visibility_changed(), None);
            assert!(
                groups
                    .set_panel_available(&assistant_id(), true, window, cx)
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
            let (mut groups, _) = shell(window, cx);
            groups.set_panel_available(&assistant_id(), true, window, cx);
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
            let (mut groups, [first, _, assistant]) = shell(window, cx);
            groups.set_panel_available(&assistant_id(), true, window, cx);
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
    fn hiding_the_sidebar_leaves_focus_alone_when_the_sidebar_does_not_hold_it(
        cx: &mut TestAppContext,
    ) {
        with_window(cx, |window, cx| {
            let (mut groups, _) = shell(window, cx);
            groups.set_panel_available(&assistant_id(), true, window, cx);
            groups.toggle(GroupKey::Assistant, window, cx);
            groups.set_expanded(GroupKey::Assistant, true, cx);
            let hide = groups.toggle(GroupKey::Navigation, window, cx).unwrap();
            assert_eq!(focus_of(hide), "preserve");
        });
    }

    #[gpui_kit::test]
    fn closing_a_group_ends_its_expansion_and_closing_it_again_changes_nothing(
        cx: &mut TestAppContext,
    ) {
        with_window(cx, |window, cx| {
            let (mut groups, [_, _, assistant]) = shell(window, cx);
            groups.set_panel_available(&assistant_id(), true, window, cx);
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
            let (mut groups, _) = shell(window, cx);
            groups.set_panel_available(&assistant_id(), true, window, cx);
            groups.toggle(GroupKey::Assistant, window, cx);
            groups.set_expanded(GroupKey::Assistant, true, cx);
            let withdrawn = groups
                .set_panel_available(&assistant_id(), false, window, cx)
                .unwrap();
            assert_eq!(withdrawn.visibility_changed(), Some(GroupKey::Assistant));
            assert!(!groups.panel_is_available(&assistant_id()));
            assert!(!groups.is_shown(GroupKey::Assistant));
            assert_eq!(groups.expanded(), None);
            assert!(
                groups
                    .set_panel_available(&assistant_id(), false, window, cx)
                    .is_none()
            );
        });
    }

    #[gpui_kit::test]
    fn only_expandable_panels_learn_which_side_they_are_on(cx: &mut TestAppContext) {
        with_window(cx, |window, cx| {
            let (groups, [first, _, assistant]) = shell(window, cx);
            groups.set_docked_left(|_| Side::Left, cx);
            assert!(assistant.read(cx).docked_left());
            assert!(!first.read(cx).docked_left());
            groups.set_docked_left(|_| Side::Right, cx);
            assert!(!assistant.read(cx).docked_left());
        });
    }

    #[gpui_kit::test]
    fn a_changed_presentation_is_reported_once(cx: &mut TestAppContext) {
        with_window(cx, |window, cx| {
            let (mut groups, [first, ..]) = shell(window, cx);
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
    fn the_layout_orders_the_members_whatever_the_registration_order(cx: &mut TestAppContext) {
        with_window(cx, |_, cx| {
            let (layout, mut groups) = (layout(), PanelGroups::new());
            for contribution in [CORE_PANEL, PANEL_B, PANEL_A] {
                let (_, hosted) = navigation(cx, contribution.title);
                groups.register(id(contribution), Some(hosted), &layout);
            }
            let position = |groups: &PanelGroups, contribution: &SystemContribution| {
                groups.offered(&id(contribution)).map(|(_, ix, _)| ix)
            };
            assert_eq!(position(&groups, PANEL_A), Some(0));
            assert_eq!(position(&groups, PANEL_B), Some(1));
            assert_eq!(position(&groups, CORE_PANEL), Some(2));
            groups.activate(GroupKey::Navigation, 0, cx);
            assert_eq!(groups.next(GroupKey::Navigation), Some(1));
            groups.activate(GroupKey::Navigation, 1, cx);
            assert_eq!(groups.next(GroupKey::Navigation), Some(2));
            groups.activate(GroupKey::Navigation, 2, cx);
            assert_eq!(groups.next(GroupKey::Navigation), Some(0));
        });
    }

    #[gpui_kit::test]
    fn a_panel_the_bar_leaves_out_opens_by_id_but_is_not_part_of_next(cx: &mut TestAppContext) {
        with_window(cx, |_, cx| {
            let (layout, mut groups) = (layout(), PanelGroups::new());
            for contribution in [TRAY_PANEL, PANEL_A, PANEL_B] {
                let (_, hosted) = navigation(cx, contribution.title);
                groups.register(id(contribution), Some(hosted), &layout);
            }
            let (host, ..) = groups.offered(&id(TRAY_PANEL)).expect("offered");
            assert_eq!(host, GroupKey::Navigation, "hosted where its hints name");
            groups.activate(GroupKey::Navigation, 1, cx);
            assert_eq!(
                groups.next(GroupKey::Navigation),
                Some(0),
                "not the tray panel"
            );
            assert!(groups.open_panel(&id(TRAY_PANEL), cx).is_some());
            assert!(groups.panel_is_open(&id(TRAY_PANEL)));
            assert_eq!(groups.next(GroupKey::Navigation), Some(0));
        });
    }

    #[gpui_kit::test]
    fn adopting_another_layout_moves_a_panel_to_the_group_it_places_it_in(cx: &mut TestAppContext) {
        with_window(cx, |window, cx| {
            let (mut groups, [_, second, _]) = shell(window, cx);
            groups.set_panel_available(&assistant_id(), true, window, cx);
            let moved = LayoutPreset {
                bar: Bar {
                    right: vec![BarItem::Group {
                        id: GroupId::new("assistant"),
                        side: None,
                        primary: false,
                        members: vec![
                            GroupEntry::Panel(id(PANEL_B)),
                            GroupEntry::Panel(assistant_id()),
                        ],
                    }],
                    ..Bar::default()
                },
                ..BuiltIn::preset()
            };
            let catalog = crate::plugin::catalog(&[TEST_CORE], &[&TEST_PLUGIN]).expect("valid");
            groups.adopt(&resolve(&moved, &catalog, &BTreeSet::new()));
            let (host, ix, _) = groups.offered(&id(PANEL_B)).expect("offered");
            assert_eq!((host, ix), (GroupKey::Assistant, 0));
            let (host, ix, _) = groups.offered(&assistant_id()).expect("offered");
            assert_eq!((host, ix), (GroupKey::Assistant, 1));
            assert_eq!(
                groups.group_holding(second.entity_id()),
                Some(GroupKey::Assistant),
                "requests from the panel reach its new group"
            );
        });
    }

    #[gpui_kit::test]
    fn a_withdrawn_member_keeps_its_position_but_is_not_offered(cx: &mut TestAppContext) {
        with_window(cx, |window, cx| {
            let (layout, mut groups) = (layout(), PanelGroups::new());
            let ids = [id(PANEL_A), id(PANEL_B), id(CORE_PANEL)];
            for (name, id) in ["a", "b", "c"].into_iter().zip(&ids) {
                let (_, hosted) = navigation(cx, name);
                groups.register(id.clone(), Some(hosted), &layout);
            }
            groups.set_panel_available(&ids[1], false, window, cx);
            let offered: Vec<_> = ids
                .iter()
                .map(|id| groups.offered(id).map(|(_, ix, _)| ix))
                .collect();
            assert_eq!(offered, [Some(0), None, Some(2)]);
            assert_eq!(groups.next(GroupKey::Navigation), Some(2));
            groups.activate(GroupKey::Navigation, 2, cx);
            assert_eq!(groups.next(GroupKey::Navigation), Some(0));
            assert!(groups.activate(GroupKey::Navigation, 1, cx).is_none());
            assert!(groups.toggle_panel(&ids[1], window, cx).is_none());
            assert!(groups.open_panel(&ids[1], cx).is_none());
            assert!(groups.panel_is_open(&ids[2]));
        });
    }

    #[gpui_kit::test]
    fn a_registered_panel_is_offered_once_it_is_filled(cx: &mut TestAppContext) {
        with_window(cx, |window, cx| {
            let (layout, mut groups) = (layout(), PanelGroups::new());
            let panel = id(PANEL_A);
            groups.register(panel.clone(), None, &layout);
            assert!(groups.offered(&panel).is_none());
            assert!(!groups.panel_is_available(&panel));
            assert_eq!(groups.next(GroupKey::Navigation), None);
            assert!(groups.open_panel(&panel, cx).is_none());
            let (_, hosted) = navigation(cx, "late");
            assert!(groups.fill(&panel, hosted));
            assert_eq!(groups.offered(&panel).map(|(_, ix, _)| ix), Some(0));
            assert!(groups.panel_is_open(&panel));
            let (_, again) = navigation(cx, "again");
            assert!(!groups.fill(&panel, again), "a panel fills its slot once");
            let (_, stray) = navigation(cx, "stray");
            assert!(!groups.fill(&id(PANEL_B), stray));
            assert!(groups.toggle_panel(&panel, window, cx).is_some());
        });
    }
}
