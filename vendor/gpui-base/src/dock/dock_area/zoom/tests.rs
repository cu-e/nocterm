// Modified by Nocterm contributors; see NOCTERM.md. Licensed under Apache-2.0.
//! Queue bursts must settle without emitting synchronization echoes.
use super::*;
use crate::dock::test_support::{Log, PanelSignal, TestPanel, drain, log_of};
use gpui::{TestAppContext, VisualTestContext};
use std::{cell::Cell, rc::Rc};

fn settle(cx: &VisualTestContext) {
    for _ in 0..1_000 {
        if !cx.cx.dispatcher.tick(false) {
            return;
        }
    }
    panic!("zoom request queue failed to settle within 1,000 scheduler steps");
}

struct Fixture {
    area: Entity<DockArea>,
    a: Entity<TabGroup>,
    b: Entity<TabGroup>,
    a_panel: Entity<TestPanel>,
    log: Log,
    events: Rc<Cell<usize>>,
    _subscriptions: Vec<gpui::Subscription>,
}

fn fixture(cx: &mut TestAppContext) -> (Fixture, &mut VisualTestContext) {
    cx.update(|cx| {
        let _ = crate::Theme::global_mut(cx);
    });
    let (area, cx) = cx.add_window_view(|window, cx| DockArea::new("zoom-queue", None, window, cx));
    let log = log_of();
    let (a_panel, b_panel) = cx.update(|window, cx| {
        let a = TestPanel::logging("a", &log, cx);
        let b = TestPanel::logging("b", &log, cx);
        area.update(cx, |area, cx| {
            area.set_center(
                DockLayout::h_split()
                    .child(DockLayout::tabs().panel(a.clone()), None)
                    .child(DockLayout::tabs().panel(b.clone()), None),
                window,
                cx,
            );
        });
        (a, b)
    });
    settle(cx);
    drain(&log);
    let (a, b) = cx.read(|cx| {
        let dock = area.read(cx);
        let a_node = dock
            .center
            .find_panel_node(PanelId::from(a_panel.entity_id()))
            .unwrap();
        let b_node = dock
            .center
            .find_panel_node(PanelId::from(b_panel.entity_id()))
            .unwrap();
        (
            dock.groups[&a_node].entity.clone(),
            dock.groups[&b_node].entity.clone(),
        )
    });
    let events = Rc::new(Cell::new(0));
    let subscriptions = cx.update(|_, cx| {
        [&a, &b]
            .into_iter()
            .map(|group| {
                let events = events.clone();
                cx.subscribe(group, move |_, event: &TabGroupEvent, _| {
                    if matches!(
                        event,
                        TabGroupEvent::ZoomIn { .. } | TabGroupEvent::ZoomOut { .. }
                    ) {
                        events.set(events.get() + 1);
                    }
                })
            })
            .collect()
    });
    (
        Fixture {
            area,
            a,
            b,
            a_panel,
            log,
            events,
            _subscriptions: subscriptions,
        },
        cx,
    )
}

fn assert_zoom(f: &Fixture, expected: Option<&Entity<TabGroup>>, cx: &VisualTestContext) {
    cx.read(|cx| {
        assert_eq!(
            f.area.read(cx).zoomed_group(),
            expected.map(|group| group.read(cx).node())
        );
        assert_eq!(f.a.read(cx).is_zoomed(), expected == Some(&f.a));
        assert_eq!(f.b.read(cx).is_zoomed(), expected == Some(&f.b));
    });
}

#[gpui::test]
fn same_group_in_out_and_in_out_in_bursts_emit_only_user_requests(cx: &mut TestAppContext) {
    let (f, cx) = fixture(cx);
    cx.update(|window, cx| {
        f.a.update(cx, |group, cx| {
            group.toggle_zoom(window, cx);
            group.toggle_zoom(window, cx);
        })
    });
    settle(cx);
    assert_zoom(&f, None, cx);
    assert_eq!(f.events.get(), 2);
    assert_eq!(
        drain(&f.log)
            .into_iter()
            .filter(|(_, signal)| matches!(signal, PanelSignal::Zoomed(_)))
            .count(),
        2
    );
    cx.update(|window, cx| {
        f.a.update(cx, |group, cx| {
            for _ in 0..3 {
                group.toggle_zoom(window, cx);
            }
        })
    });
    settle(cx);
    assert_zoom(&f, Some(&f.a), cx);
    assert_eq!(f.events.get(), 5);
}

#[gpui::test]
fn latest_cross_group_intent_survives_silent_synchronization(cx: &mut TestAppContext) {
    let (f, cx) = fixture(cx);
    cx.update(|window, cx| {
        f.a.update(cx, |group, cx| group.toggle_zoom(window, cx));
        f.b.update(cx, |group, cx| group.toggle_zoom(window, cx));
        f.a.update(cx, |group, cx| {
            group.toggle_zoom(window, cx);
            group.toggle_zoom(window, cx);
        });
    });
    settle(cx);
    assert_zoom(&f, Some(&f.a), cx);
    assert_eq!(f.events.get(), 4);
}

#[gpui::test]
fn explicit_out_cancels_queued_in_while_area_is_still_unzoomed(cx: &mut TestAppContext) {
    let (f, cx) = fixture(cx);
    cx.update(|window, cx| {
        f.a.update(cx, |group, cx| group.toggle_zoom(window, cx));
        f.area.update(cx, |area, cx| {
            assert!(!area.is_zoomed());
            area.set_zoomed_out(window, cx);
        });
    });
    settle(cx);
    assert_zoom(&f, None, cx);
    assert_eq!(f.events.get(), 1);
}

#[gpui::test]
fn explicit_in_overrides_pending_groups_even_when_area_target_is_unchanged(
    cx: &mut TestAppContext,
) {
    let (f, cx) = fixture(cx);
    let a_node = cx.read(|cx| f.a.read(cx).node());
    cx.update(|window, cx| {
        f.area
            .update(cx, |area, cx| area.set_zoomed_in(a_node, window, cx))
    });
    settle(cx);
    cx.update(|window, cx| {
        f.a.update(cx, |group, cx| group.toggle_zoom(window, cx));
        f.b.update(cx, |group, cx| group.toggle_zoom(window, cx));
        f.area
            .update(cx, |area, cx| area.set_zoomed_in(a_node, window, cx));
    });
    settle(cx);
    assert_zoom(&f, Some(&f.a), cx);
    assert_eq!(f.events.get(), 2);
}

#[gpui::test]
fn removed_group_request_cannot_zoom_the_replacement_layout(cx: &mut TestAppContext) {
    let (f, cx) = fixture(cx);
    cx.update(|window, cx| {
        f.a.update(cx, |group, cx| group.toggle_zoom(window, cx));
        f.area.update(cx, |area, cx| {
            let replacement = TestPanel::new("replacement", cx);
            area.set_center(DockLayout::tabs().panel(replacement), window, cx);
        });
    });
    settle(cx);
    cx.read(|cx| assert!(!f.area.read(cx).is_zoomed()));
    assert_eq!(f.events.get(), 1);
}

#[gpui::test]
fn cached_entity_replacement_rejects_old_request_even_with_same_node_and_revision(
    cx: &mut TestAppContext,
) {
    let (f, cx) = fixture(cx);
    cx.update(|window, cx| {
        let (node, revision) = f.a.update(cx, |group, cx| {
            group.toggle_zoom(window, cx);
            (group.node(), group.zoom_revision())
        });
        f.area.update(cx, |area, cx| {
            area.groups.remove(&node);
            let replacement = area.group_entity(node, window, cx);
            replacement.update(cx, |group, _| {
                for _ in 0..revision {
                    group.invalidate_zoom_requests();
                }
            });
            // Simulate delivery from a retained outgoing entity, not a new request.
            area.reconcile_zoom_request(&f.a, revision, true, window, cx);
            assert!(!area.is_zoomed());
        });
    });
    settle(cx);
    cx.read(|cx| assert!(!f.area.read(cx).is_zoomed()));
}

#[gpui::test]
fn unavailable_or_unzoomable_panels_refuse_requests_and_callbacks(cx: &mut TestAppContext) {
    let (f, cx) = fixture(cx);
    cx.update(|window, cx| {
        f.a_panel
            .update(cx, |panel, cx| panel.set_zoomable(false, cx));
        f.a.update(cx, |group, cx| group.toggle_zoom(window, cx));
        let node = f.a.read(cx).node();
        f.area
            .update(cx, |area, cx| area.set_zoomed_in(node, window, cx));
        f.a_panel
            .update(cx, |panel, cx| panel.set_visible(false, cx));
        f.a.update(cx, |group, cx| group.toggle_zoom(window, cx));
    });
    settle(cx);
    assert_zoom(&f, None, cx);
    assert_eq!(f.events.get(), 0);
    assert!(
        !drain(&f.log)
            .iter()
            .any(|(_, signal)| matches!(signal, PanelSignal::Zoomed(_)))
    );
}

#[gpui::test]
fn panel_becoming_unzoomable_before_delivery_clears_pending_zoom(cx: &mut TestAppContext) {
    let (f, cx) = fixture(cx);
    cx.update(|window, cx| {
        f.a.update(cx, |group, cx| group.toggle_zoom(window, cx));
        f.a_panel
            .update(cx, |panel, cx| panel.set_zoomable(false, cx));
    });
    settle(cx);
    assert_zoom(&f, None, cx);
    assert_eq!(f.events.get(), 1);
    let callbacks: Vec<_> = drain(&f.log)
        .into_iter()
        .filter(|(_, signal)| matches!(signal, PanelSignal::Zoomed(_)))
        .collect();
    assert_eq!(callbacks.len(), 2);
    assert!(callbacks.contains(&("a", PanelSignal::Zoomed(true))));
    assert!(callbacks.contains(&("a", PanelSignal::Zoomed(false))));
}

mod mutations;
