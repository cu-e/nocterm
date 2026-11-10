//! Rendering and filtering large connection folders through the public sidebar.
use super::*;

fn profile(index: usize) -> Profile {
    let name = format!("Server {index:03}");
    Profile {
        id: nocterm_connections_model::test_support::profile_id_for(&name),
        name,
        group: Some("Work".into()),
        target: nocterm_session::Target::new("ci", "host", 22),
        description: "A saved host".into(),
        options: Default::default(),
        auth: Default::default(),
        credential: None,
        launch: None,
        icon: Some("ubuntu".into()),
        icon_color: None,
        country: None,
        jump_hosts: Vec::new(),
    }
}

fn rendered(window: &Window) -> Vec<usize> {
    (0..256)
        .filter(|index| {
            window
                .try_find(SharedString::from(format!(
                    "connection-{}",
                    profile(*index).id
                )))
                .is_some()
        })
        .collect()
}

#[gpui_kit::test]
fn expanded_large_folders_build_visible_rows_and_open_the_filtered_last_server(
    cx: &mut TestAppContext,
) {
    let (handle, workspace, opened, installed) = crate::test_support::workspace(cx);
    cx.update_window(handle, |_, window, cx| {
        installed.connections().update(cx, |connections, cx| {
            for index in 0..256 {
                connections
                    .save_profile(profile(index), cx)
                    .now_or_never()
                    .unwrap()
                    .unwrap();
            }
        });
        let panel = cx
            .new(|cx| ConnectionsPanel::new(installed.clone(), workspace.downgrade(), window, cx));
        workspace.update(cx, |workspace, cx| {
            add_servers(workspace, panel, window, cx)
        });
        window.render_frame(cx);
        let materialized = rendered(window).len();
        assert!(
            (1..40).contains(&materialized),
            "materialized {materialized}/256 rows"
        );
        assert!(
            window
                .try_find(SharedString::from(format!(
                    "connection-{}",
                    profile(255).id
                )))
                .is_none()
        );
        window.scroll(
            "connections-list",
            gpui_kit::ScrollDelta::Pixels(gpui_kit::point(
                gpui_kit::px(0.),
                gpui_kit::px(-100_000.),
            )),
            cx,
        );
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let visible = rendered(window);
        assert!(
            visible.contains(&255),
            "native wheel reaches the last row: {visible:?}"
        );
        window.click("connections-filter", cx);
        window.input("Server 255", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(
            window
                .try_find(SharedString::from(format!("connection-{}", profile(0).id)))
                .is_none()
        );
        window.click(
            SharedString::from(format!("connection-{}", profile(255).id)),
            cx,
        );
    })
    .unwrap();
    assert_eq!(opened.borrow().len(), 1);
}
