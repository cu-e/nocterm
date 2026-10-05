//! Placing connections by dropping them on other rows.
use super::*;

#[gpui_kit::test]
fn dropping_on_a_row_places_the_connection_above_it(cx: &mut TestAppContext) {
    let (handle, workspace, opened) = crate::test_support::workspace(cx);
    let profile = |name: &str, group: Option<&str>| Profile {
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
    };
    let (a, b, c) = (
        profile("a", None),
        profile("b", None),
        profile("c", Some("Work")),
    );
    cx.update_window(handle, |_, window, cx| {
        let connections = Connections::global(cx);
        connections.update(cx, |connections, cx| {
            for p in [&a, &b, &c] {
                connections
                    .save_profile(p.clone(), cx)
                    .now_or_never()
                    .unwrap()
                    .unwrap();
            }
        });
        let panel = cx.new(|cx| {
            ConnectionsPanel::new(connections.clone(), workspace.downgrade(), window, cx)
        });
        workspace.update(cx, |workspace, cx| workspace.add_panel(panel, cx));
        window.render_frame(cx);
        window.drag_to(
            SharedString::from(format!("connection-{}", b.id)),
            SharedString::from(format!("connection-{}", a.id)),
            cx,
        );
    })
    .unwrap();
    cx.run_until_parked();
    let order = |cx: &mut TestAppContext, group: Option<&'static str>| {
        cx.read(|cx| {
            Connections::global(cx)
                .read(cx)
                .profiles()
                .in_group(group, "")
                .iter()
                .map(|p| p.name.clone())
                .collect::<Vec<_>>()
        })
    };
    assert_eq!(order(cx, None), ["b", "a"]);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.drag_to(
            SharedString::from(format!("connection-{}", a.id)),
            SharedString::from(format!("connection-{}", c.id)),
            cx,
        );
    })
    .unwrap();
    cx.run_until_parked();
    assert_eq!(order(cx, None), ["b"]);
    assert_eq!(
        order(cx, Some("Work")),
        ["a", "c"],
        "joins the row's folder"
    );
    assert!(opened.borrow().is_empty(), "a drag must not open a session");
}
