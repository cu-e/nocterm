//! Native server dragging preserves the rendered source and target geometry.
use super::*;
use gpui_kit::{
    InputEvent as _, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent,
    component::{Theme, ThemeMode},
    point, px,
};

#[gpui_kit::test]
#[expect(clippy::too_many_lines, reason = "predates the limit")]
fn native_append_tails_reorder_the_last_group_position_and_preserve_empty_targets(
    cx: &mut TestAppContext,
) {
    let first = gp("First", Some("Homelab"));
    let last = gp("Last", Some("Homelab"));
    let root = gp("Root", None);
    let (handle, _, opened) = setup(cx, &[first.clone(), last.clone(), root.clone()]);
    let order = |cx: &mut TestAppContext, group| {
        profiles_of(cx)
            .in_group(group, "")
            .iter()
            .map(|profile| profile.id)
            .collect::<Vec<_>>()
    };
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let source = window
            .find(SharedString::from(format!("connection-{}", first.id)))
            .bounds();
        let last_id = SharedString::from(format!("connection-{}", last.id));
        let last_bounds = window.find(last_id.clone()).bounds();
        let tail = window.find("connections-append-group-Homelab").bounds();
        assert_eq!(tail.size.height, px(8.));
        assert!(tail.origin.y >= last_bounds.bottom());
        let origin = source.origin + point(px(24.), px(12.));
        window.dispatch_event(
            MouseMoveEvent {
                position: origin,
                pressed_button: None,
                modifiers: Default::default(),
            }
            .to_platform_input(),
            cx,
        );
        window.dispatch_event(
            MouseDownEvent {
                position: origin,
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
                position: tail.center(),
                pressed_button: Some(MouseButton::Left),
                modifiers: Default::default(),
            }
            .to_platform_input(),
            cx,
        );
        window.render_frame(cx);
        assert!(cx.has_active_drag());
        assert!(
            window
                .find("connections-append-group-Homelab-marker")
                .visible()
        );
        assert_eq!(window.find(last_id).bounds(), last_bounds);
        assert_eq!(
            window.find("connections-append-group-Homelab").bounds(),
            tail
        );
        window.dispatch_event(
            MouseUpEvent {
                position: tail.center(),
                button: MouseButton::Left,
                modifiers: Default::default(),
                click_count: 1,
            }
            .to_platform_input(),
            cx,
        );
        window.render_frame(cx);
    })
    .unwrap();
    cx.run_until_parked();
    assert_eq!(order(cx, Some("Homelab")), [last.id, first.id]);
    assert_eq!(
        profiles_of(cx).get(first.id).unwrap().group.as_deref(),
        Some("Homelab")
    );
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.drag_to(
            SharedString::from(format!("connection-{}", first.id)),
            "connections-append-root",
            cx,
        );
    })
    .unwrap();
    cx.run_until_parked();
    assert_eq!(order(cx, None), [root.id, first.id]);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.drag_to(
            SharedString::from(format!("connection-{}", last.id)),
            "connections-append-root",
            cx,
        );
    })
    .unwrap();
    cx.run_until_parked();
    assert!(order(cx, Some("Homelab")).is_empty());
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(
            window.find("connections-append-group-Homelab").visible(),
            "Remembered empty groups remain append targets"
        );
        window.click("group-Homelab", cx);
        window.render_frame(cx);
        assert!(
            window
                .try_find("connections-append-group-Homelab")
                .is_none(),
            "Collapsed groups have no hidden append hitbox"
        );
    })
    .unwrap();
    assert!(opened.borrow().is_empty());
}

#[gpui_kit::test]
#[expect(clippy::too_many_lines, reason = "predates the limit")]
fn source_and_preview_keep_server_metadata_and_dimensions_without_shifting_target(
    cx: &mut TestAppContext,
) {
    let mut server = gp("Production 資料 — long server name", Some("Homelab"));
    server.description = "Описание сервера — deployment commands".into();
    server.icon = Some("ubuntu".into());
    server.icon_color = Some("#E95420".into());
    server.country = Some("de".into());
    let destination = gp("Destination", Some("Homelab"));
    let (handle, _, opened) = setup(cx, &[destination.clone(), server.clone()]);
    cx.simulate_window_resize(handle, gpui_kit::size(px(1400.), px(900.)));
    cx.run_until_parked();
    for mode in [ThemeMode::Light, ThemeMode::Dark] {
        cx.update_window(handle, |_, window, cx| {
            Theme::change(mode, None, cx);
            window.render_frame(cx);
            let source_id = SharedString::from(format!("connection-{}", server.id));
            let target_id = SharedString::from(format!("connection-{}", destination.id));
            let marker_id =
                SharedString::from(format!("connection-drop-marker-{}", destination.id));
            let source = window.find(source_id.clone());
            let target = window.find(target_id.clone()).bounds();
            assert!(
                window
                    .try_find(marker_id.clone())
                    .is_none_or(|marker| !marker.visible())
            );
            let source_bounds = source.bounds();
            let origin = source_bounds.origin + point(px(24.), px(12.));
            window.dispatch_event(
                MouseMoveEvent {
                    position: origin,
                    pressed_button: None,
                    modifiers: Default::default(),
                }
                .to_platform_input(),
                cx,
            );
            window.dispatch_event(
                MouseDownEvent {
                    position: origin,
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
                    position: target.center(),
                    pressed_button: Some(MouseButton::Left),
                    modifiers: Default::default(),
                }
                .to_platform_input(),
                cx,
            );
            window.render_frame(cx);
            let preview = window.find("drag-preview");
            assert_eq!(preview.bounds().size, source_bounds.size);
            assert_eq!(
                window.find(target_id).bounds(),
                target,
                "Insertion overlay must not shift or resize a row"
            );
            let marker = window.find(marker_id);
            assert!(
                marker.visible(),
                "Dragging over another grouped server must show its insertion line"
            );
            assert_eq!(marker.bounds().size.height, px(1.));
            assert_eq!(marker.bounds().size.width, target.size.width);
            for kind in ["name", "target", "description", "icon", "flag"] {
                let child_id = SharedString::from(format!("connection-{kind}-{}", server.id));
                let source_child = window.within(source_id.clone()).find(child_id.clone());
                let preview_child = window.within("drag-preview").find(child_id);
                assert!(
                    source_child.visible() && preview_child.visible(),
                    "Both rows retain {kind}"
                );
                assert_eq!(
                    source_child.bounds().size,
                    preview_child.bounds().size,
                    "Preserve {kind} footprint"
                );
                assert_eq!(
                    source_child.bounds().origin - source_bounds.origin,
                    preview_child.bounds().origin - preview.bounds().origin,
                    "Preserve {kind} placement"
                );
            }
            assert!(
                window
                    .within("drag-preview")
                    .find(SharedString::from(format!("edit-{}", server.id)))
                    .visible()
            );
            assert!(
                window
                    .within("drag-preview")
                    .find(SharedString::from(format!("delete-{}", server.id)))
                    .visible()
            );
            window.dispatch_event(
                MouseUpEvent {
                    position: target.center(),
                    button: MouseButton::Left,
                    modifiers: Default::default(),
                    click_count: 1,
                }
                .to_platform_input(),
                cx,
            );
            window.render_frame(cx);
        })
        .unwrap();
        cx.run_until_parked();
        assert_eq!(
            profiles_of(cx)
                .in_group(Some("Homelab"), "")
                .iter()
                .map(|profile| profile.id)
                .collect::<Vec<_>>(),
            [server.id, destination.id]
        );
    }
    assert!(
        opened.borrow().is_empty(),
        "Drag previews never open a connection"
    );
}
