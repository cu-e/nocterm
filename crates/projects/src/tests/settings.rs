//! Workspace defaults retain their original settings installation and creation snapshot.
use super::{support::*, *};
use gpui_kit::component::WindowExt as _;
use nocterm_project_providers::{
    PreparedProject, SetupMode, WorkspacesSettings, testing::FakeProjectPreparation,
};
use nocterm_ui::SettingsPage as _;

fn save_default(rig: &Fixture, directory: &str, cx: &mut TestAppContext) {
    let directory = directory.to_owned();
    cx.update(|cx| {
        rig.installed
            .settings()
            .update::<WorkspacesSettings>(cx, move |settings| {
                settings.container_directory = directory
            })
            .detach()
    });
    cx.run_until_parked();
}
fn defaults_fixture(cx: &mut TestAppContext) -> (Fixture, FakeProjectPreparation) {
    let preparation = FakeProjectPreparation::default();
    preparation
        .inventory(vec![super::container_selection::inventory(
            nocterm_containers::Engine::Docker,
        )])
        .unwrap();
    for _ in 0..2 {
        preparation
            .respond(Ok(PreparedProject {
                target: definition(20).target().clone(),
                owned: None,
            }))
            .unwrap();
    }
    let (rig, _, _) = Fixture::local(preparation.clone(), cx);
    (rig, preparation)
}
fn open_container(rig: &Fixture, attach: bool, cx: &mut TestAppContext) {
    cx.update_window(rig.window, |_, window, cx| {
        window.render_frame(cx);
        window.click("project-new", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(rig.window, |_, window, cx| {
        window.render_frame(cx);
        window.click(
            if attach {
                "project-mode-existing"
            } else {
                "project-mode-new"
            },
            cx,
        );
        window.render_frame(cx);
        if attach {
            window.click(format!("project-container-Docker-{:064x}", 1), cx);
        }
        window.click("project-next", cx);
        window.render_frame(cx);
        fill(window, "project-name", "Container defaults", cx);
    })
    .unwrap();
}
fn request_root(request: &nocterm_project_providers::ProjectSetupRequest) -> &str {
    match &request.mode {
        SetupMode::Attach {
            root: Some(root), ..
        }
        | SetupMode::Create {
            root: Some(root), ..
        } => root.as_str(),
        _ => panic!("container UI must submit its captured directory"),
    }
}

#[gpui_kit::test]
fn empty_container_directories_use_the_captured_setting_and_explicit_directories_win(
    cx: &mut TestAppContext,
) {
    for attach in [false, true] {
        for explicit in [None, Some("/srv/explicit project")] {
            let (rig, preparation) = defaults_fixture(cx);
            save_default(&rig, "/home/captured", cx);
            open_container(&rig, attach, cx);
            save_default(&rig, "/home/later", cx);
            cx.update_window(rig.window, |_, window, cx| {
                if let Some(explicit) = explicit {
                    fill(window, "project-path", explicit, cx);
                }
                create_workspace(window, cx);
            })
            .unwrap();
            cx.run_until_parked();
            assert_eq!(
                request_root(&preparation.requests().unwrap()[0]),
                explicit.unwrap_or("/home/captured")
            );
            cx.update_window(rig.window, |_, window, cx| window.close_dialog(cx))
                .unwrap();
            open_container(&rig, false, cx);
            cx.update_window(rig.window, |_, window, cx| create_workspace(window, cx))
                .unwrap();
            cx.run_until_parked();
            assert_eq!(
                request_root(&preparation.requests().unwrap()[1]),
                "/home/later"
            );
            cx.update_window(rig.window, |_, window, _| window.remove_window())
                .unwrap();
        }
    }
}

#[gpui_kit::test]
fn workspace_settings_reject_invalid_literals_and_flush_to_the_original_installation(
    cx: &mut TestAppContext,
) {
    let (original, _) = defaults_fixture(cx);
    let (other, _) = defaults_fixture(cx);
    let settings = original.installed.settings().clone();
    let (window, page) = cx.update(|cx| {
        gpui_kit::open_window(Default::default(), cx, |window, cx| {
            cx.new(|cx| crate::settings_page::WorkspacesPage::new(settings.clone(), window, cx))
        })
        .unwrap()
    });
    cx.update_window(window, |_, window, cx| {
        window.render_frame(cx);
        fill(window, "workspace-default-directory", "relative/path", cx);
        window.press("enter", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update(|cx| {
        assert_eq!(
            settings
                .setting::<WorkspacesSettings>(cx)
                .container_directory,
            "/home"
        )
    });
    cx.update_window(window, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(
            window.find("workspace-default-directory").value(),
            Some("relative/path")
        );
        fill(
            window,
            "workspace-default-directory",
            "/home/original owner",
            cx,
        );
        page.update(cx, |page, cx| page.on_deactivate(window, cx));
    })
    .unwrap();
    cx.run_until_parked();
    cx.update(|cx| {
        assert_eq!(
            settings
                .setting::<WorkspacesSettings>(cx)
                .container_directory,
            "/home/original owner"
        );
        assert_eq!(
            other
                .installed
                .settings()
                .setting::<WorkspacesSettings>(cx)
                .container_directory,
            "/home"
        );
    });
    cx.update_window(window, |_, window, cx| {
        fill(window, "workspace-default-directory", "/home/close", cx);
        page.update(cx, |page, cx| page.on_close(window, cx));
    })
    .unwrap();
    cx.run_until_parked();
    cx.update(|cx| {
        assert_eq!(
            settings
                .setting::<WorkspacesSettings>(cx)
                .container_directory,
            "/home/close"
        )
    });
}
