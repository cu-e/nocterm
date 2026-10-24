//! The real application catalog exposes workspace policy from its original store.
use super::*;
use nocterm_project_providers::WorkspacesSettings;

#[gpui_kit::test]
fn the_workspaces_settings_page_updates_the_bootstrapped_policy(cx: &mut TestAppContext) {
    let (handle, workspace, _, _, _, policy) = fixture_with_settings(cx, false, None, false, None);
    cx.update_window(handle, |_, window, cx| {
        window.dispatch_action(Box::new(nocterm_workspace::OpenSettings), cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.dispatch_action(
            Box::new(nocterm_workspace::OpenPluginPage {
                plugin: "projects".into(),
            }),
            cx,
        );
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let settings = workspace
            .read(cx)
            .find_item::<nocterm_settings_ui::SettingsView>(cx)
            .unwrap();
        assert_eq!(settings.read(cx).selected_page_id(), "workspaces");
        assert_eq!(
            policy.setting::<WorkspacesSettings>(cx).container_directory,
            "/home"
        );
        window.click("workspace-default-directory", cx);
        window.press("ctrl-a", cx);
        window.input("/home/shared project", cx);
        window.press("enter", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update(|cx| {
        assert_eq!(
            policy.setting::<WorkspacesSettings>(cx).container_directory,
            "/home/shared project"
        )
    });
}
