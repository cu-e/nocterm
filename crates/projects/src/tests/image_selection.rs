//! Local image selection and bounded original-host registry discovery.
use super::{support::*, *};
use nocterm_containers::{Engine, Image};
use nocterm_project_providers::{
    EngineInventory, PreparedProject, SetupMode, testing::FakeProjectPreparation,
};
use nocterm_project_providers::{ImageSearchError, RegistryImage, testing::FakeImageSearch};
use std::time::Duration;

pub(super) fn registry_image(reference: &str) -> RegistryImage {
    RegistryImage {
        reference: reference.into(),
        description: format!("Development image {reference}"),
        official: true,
        stars: Some(10),
    }
}
fn image_form(
    search: FakeImageSearch,
    cx: &mut TestAppContext,
) -> (
    Fixture,
    FakeProjectPreparation,
    Arc<dyn nocterm_session::HostExec>,
) {
    let preparation = FakeProjectPreparation::default();
    let images = vec![
        Image {
            id: "debian-local".into(),
            repository: Some("team/debian13".into()),
            tag: Some("BookWorm".into()),
            size: "120 MB".into(),
            created: "today".into(),
        },
        Image {
            id: "ubuntu-local".into(),
            repository: Some("ubuntu".into()),
            tag: Some("24.04".into()),
            size: "90 MB".into(),
            created: "today".into(),
        },
    ];
    preparation
        .inventory(vec![
            EngineInventory {
                engine: Engine::Docker,
                containers: Ok(vec![]),
                images: Ok(images.clone()),
            },
            EngineInventory {
                engine: Engine::Podman,
                containers: Ok(vec![]),
                images: Ok(images),
            },
        ])
        .unwrap();
    preparation
        .respond(Ok(PreparedProject {
            target: definition(22).target().clone(),
            owned: None,
        }))
        .unwrap();
    let exec: Arc<dyn nocterm_session::HostExec> =
        Arc::new(nocterm_session::testing::FakeExec::default());
    let rig = Fixture::with_installation(
        vec![],
        Arc::new(|_| Box::pin(async { Ok(()) })),
        |installed| {
            installed
                .with_local_host(Some(exec.clone()))
                .with_preparation(Arc::new(preparation.clone()))
                .with_image_search(Arc::new(search))
        },
        Rc::new(nocterm_workspace::test_support::ProjectTerminalProbe::default()),
        cx,
    );
    cx.update_window(rig.window, |_, window, cx| {
        window.render_frame(cx);
        window.click("project-new", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(rig.window, |_, window, cx| {
        window.render_frame(cx);
        fill(window, "project-name", "Image workspace", cx);
        wizard_field(window, "project-mode-new", cx);
        window.click("project-mode-new", cx);
        window.render_frame(cx);
        wizard_field(window, "project-image-Custom image", cx);
        window.click("project-image-Custom image", cx);
        window.render_frame(cx);
    })
    .unwrap();
    (rig, preparation, exec)
}
fn wait_for_search(cx: &mut TestAppContext) {
    cx.run_until_parked();
    cx.executor().advance_clock(Duration::from_millis(500));
    cx.run_until_parked();
}
fn image_query(rig: &Fixture, query: &str, cx: &mut TestAppContext) {
    cx.update_window(rig.window, |_, window, cx| {
        fill(window, "project-custom-image", query, cx);
        window.render_frame(cx);
    })
    .unwrap();
}

#[gpui_kit::test]
fn local_image_filter_matches_literal_repository_tag_and_reference_immediately(
    cx: &mut TestAppContext,
) {
    let search = FakeImageSearch::default();
    let (rig, preparation, _) = image_form(search.clone(), cx);
    image_query(&rig, "BOOKWORM", cx);
    cx.update_window(rig.window, |_, window, _| {
        assert!(
            window
                .try_find("project-local-image-debian-local")
                .is_some()
        );
        assert!(
            window
                .try_find("project-local-image-ubuntu-local")
                .is_none()
        );
        let list = window.find("project-available-images").bounds();
        let row = window.find("project-local-image-debian-local").bounds();
        let text = window
            .find("project-local-image-debian-local-reference")
            .bounds();
        assert_eq!(row.size.width, list.size.width);
        assert!(text.left() - row.left() < gpui_kit::px(20.));
    })
    .unwrap();
    image_query(&rig, "debian13", cx);
    assert!(search.calls().unwrap().is_empty());
    cx.update_window(rig.window, |_, window, cx| {
        create_workspace(window, cx);
    })
    .unwrap();
    cx.run_until_parked();
    assert!(
        matches!(&preparation.requests().unwrap()[0].mode, SetupMode::Create { image: nocterm_project_providers::WorkspaceImage::Custom(reference), languages, .. } if reference == "debian13" && languages.len() == 1)
    );
}

#[gpui_kit::test]
fn registry_search_debounces_caches_and_preserves_the_original_executor(cx: &mut TestAppContext) {
    let search = FakeImageSearch::default();
    search.respond(Ok(vec![registry_image("debian")])).unwrap();
    let (rig, _, exec) = image_form(search.clone(), cx);
    image_query(&rig, "de", cx);
    image_query(&rig, "deb", cx);
    image_query(&rig, "Debian", cx);
    cx.run_until_parked();
    cx.executor().advance_clock(Duration::from_millis(499));
    cx.run_until_parked();
    assert!(search.calls().unwrap().is_empty());
    cx.executor().advance_clock(Duration::from_millis(1));
    cx.run_until_parked();
    let calls = search.calls().unwrap();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].query.as_str(), "Debian");
    assert!(Arc::ptr_eq(&calls[0].exec, &exec));
    image_query(&rig, "", cx);
    wait_for_search(cx);
    image_query(&rig, "debian", cx);
    wait_for_search(cx);
    assert_eq!(search.calls().unwrap().len(), 1);
    cx.update_window(rig.window, |_, window, cx| {
        window.render_frame(cx);
        window.click("project-image-source-registry", cx);
        window.render_frame(cx);
        assert!(window.try_find("project-registry-image-debian").is_some());
        window.click("project-registry-image-debian", cx);
        window.render_frame(cx);
        assert_eq!(
            window.find("project-registry-image-debian").checked(),
            Some(true)
        );
        assert_eq!(
            window.within("project-custom-image").find("input").value(),
            Some("debian")
        );
    })
    .unwrap();
    wait_for_search(cx);
    assert_eq!(search.calls().unwrap().len(), 1);
    cx.executor().advance_clock(Duration::from_secs(91));
    image_query(&rig, "other", cx);
    image_query(&rig, "debian", cx);
    wait_for_search(cx);
    assert_eq!(search.calls().unwrap().len(), 2);
}

#[gpui_kit::test]
fn obsolete_queries_and_engine_results_cannot_replace_the_current_image_results(
    cx: &mut TestAppContext,
) {
    let search = FakeImageSearch::default();
    let (send, receive) = async_channel::bounded::<()>(1);
    search
        .respond_after(
            Box::pin(async move {
                let _ = receive.recv().await;
            }),
            Ok(vec![registry_image("obsolete-debian")]),
        )
        .unwrap();
    search
        .respond(Ok(vec![registry_image("fresh-debian")]))
        .unwrap();
    search
        .respond(Ok(vec![registry_image("podman-debian")]))
        .unwrap();
    let (rig, _, _) = image_form(search.clone(), cx);
    image_query(&rig, "debian", cx);
    wait_for_search(cx);
    image_query(&rig, "ubuntu", cx);
    image_query(&rig, "debian", cx);
    wait_for_search(cx);
    assert!(search.calls().unwrap()[0].cancel.is_cancelled());
    let _ = send.try_send(());
    cx.run_until_parked();
    cx.update_window(rig.window, |_, window, cx| {
        window.render_frame(cx);
        window.click("project-image-source-registry", cx);
        window.render_frame(cx);
        assert!(
            window
                .try_find("project-registry-image-obsolete-debian")
                .is_none()
        );
        window.click("project-image-source-registry", cx);
        window.render_frame(cx);
        assert!(
            window
                .try_find("project-registry-image-fresh-debian")
                .is_some()
        );
        wizard_field(window, "project-engine-Podman", cx);
        window.click("project-engine-Podman", cx);
    })
    .unwrap();
    wait_for_search(cx);
    assert_eq!(search.calls().unwrap()[2].engine, Engine::Podman);
    cx.update_window(rig.window, |_, window, cx| {
        window.render_frame(cx);
        window.click("project-image-source-registry", cx);
        window.render_frame(cx);
        assert!(
            window
                .try_find("project-registry-image-fresh-debian")
                .is_none()
        );
        window.click("project-image-source-registry", cx);
        window.render_frame(cx);
        assert!(
            window
                .try_find("project-registry-image-podman-debian")
                .is_some()
        );
    })
    .unwrap();
}

#[gpui_kit::test]
fn registry_rate_limits_keep_local_images_and_manual_references_usable_without_retries(
    cx: &mut TestAppContext,
) {
    let search = FakeImageSearch::default();
    search
        .respond(Err(ImageSearchError::RateLimited {
            cooldown: Duration::from_secs(60),
        }))
        .unwrap();
    search
        .respond(Ok(vec![registry_image("after-cooldown")]))
        .unwrap();
    let (rig, preparation, _) = image_form(search.clone(), cx);
    image_query(&rig, "debian", cx);
    wait_for_search(cx);
    image_query(&rig, "debian13", cx);
    wait_for_search(cx);
    assert_eq!(search.calls().unwrap().len(), 1);
    cx.update_window(rig.window, |_, window, cx| {
        window.render_frame(cx);
        assert!(
            window
                .try_find("project-local-image-debian-local")
                .is_some()
        );
        window.click("project-image-source-registry", cx);
        window.render_frame(cx);
        assert!(
            window
                .find("project-registry-status")
                .label()
                .unwrap()
                .contains("rate limited")
        );
    })
    .unwrap();
    cx.executor().advance_clock(Duration::from_secs(61));
    cx.run_until_parked();
    assert_eq!(search.calls().unwrap().len(), 1);
    image_query(&rig, "debian:13", cx);
    wait_for_search(cx);
    assert_eq!(search.calls().unwrap().len(), 2);
    cx.update_window(rig.window, |_, window, cx| create_workspace(window, cx))
        .unwrap();
    cx.run_until_parked();
    assert!(
        matches!(&preparation.requests().unwrap()[0].mode, SetupMode::Create { image: nocterm_project_providers::WorkspaceImage::Custom(reference), .. } if reference == "debian:13")
    );
}

#[gpui_kit::test]
fn leaving_the_image_picker_and_saving_cancel_registry_work_and_lock_the_controls(
    cx: &mut TestAppContext,
) {
    let search = FakeImageSearch::default();
    let (_send, receive) = async_channel::bounded::<()>(1);
    search
        .respond_after(
            Box::pin(async move {
                let _ = receive.recv().await;
            }),
            Ok(vec![registry_image("obsolete")]),
        )
        .unwrap();
    let (rig, preparation, _) = image_form(search.clone(), cx);
    image_query(&rig, "debian", cx);
    wait_for_search(cx);
    cx.update_window(rig.window, |_, window, cx| {
        wizard_field(window, "project-mode-direct", cx);
        window.click("project-mode-direct", cx);
        window.render_frame(cx);
    })
    .unwrap();
    assert!(search.calls().unwrap()[0].cancel.is_cancelled());
    let (_send, receive) = async_channel::bounded::<()>(1);
    preparation
        .suspend(Box::pin(async move {
            let _ = receive.recv().await;
        }))
        .unwrap();
    cx.update_window(rig.window, |_, window, cx| {
        wizard_field(window, "project-mode-new", cx);
        window.click("project-mode-new", cx);
        window.render_frame(cx);
        create_workspace(window, cx);
        window.render_frame(cx);
        window.click("project-step-connection", cx);
        window.render_frame(cx);
        assert_eq!(window.find("project-step-heading").label(), Some("Tools"));
        assert!(window.try_find("project-custom-image").is_none());
        assert!(window.try_find("project-engine-Podman").is_none());
        create_workspace(window, cx);
    })
    .unwrap();
    wait_for_search(cx);
    assert_eq!(search.calls().unwrap().len(), 1);
    assert_eq!(preparation.requests().unwrap().len(), 1);
}

pub(super) fn native_creation_variants(
    cx: &mut gpui_kit::HeadlessAppContext,
    window: AnyWindowHandle,
    screenshot: Option<&std::ffi::OsStr>,
) {
    use gpui_kit::component::{Theme, ThemeMode};
    for (mode, name) in [(ThemeMode::Dark, "dark"), (ThemeMode::Light, "light")] {
        cx.update(|cx| Theme::change(mode, None, cx));
        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            wizard_field(window, "project-mode-new", cx);
            window.click("project-mode-new", cx);
            window.render_frame(cx);
            wizard_field(window, "project-image-Custom image", cx);
            window.click("project-image-Custom image", cx);
            window.render_frame(cx);
            fill(window, "project-custom-image", "debian", cx);
        })
        .unwrap();
        cx.run_until_parked();
        cx.advance_clock(Duration::from_millis(500));
        cx.run_until_parked();
        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            window.click("project-image-source-registry", cx);
            window.render_frame(cx);
            assert!(window.try_find("project-registry-image-debian").is_some());
            assert!(
                window.find("project-form-footer").bounds().bottom()
                    <= window.viewport_size().height
            );
        })
        .unwrap();
        cx.update_window(window, |_, window, cx| {
            reveal_form(window, "project-registry-images", cx)
        })
        .unwrap();
        native_screenshot(cx, window, screenshot, name, "images");
        cx.update_window(window, |_, window, cx| {
            reveal_form(window, "project-image-Ubuntu 24.04", cx);
            window.click("project-image-Ubuntu 24.04", cx);
            window.render_frame(cx);
            fill(window, "project-name", "Native wizard", cx);
            wizard_field(window, "project-languages", cx);
            window.render_frame(cx);
            let first = window.find("project-language-row-python").bounds();
            let last = window.find("project-language-row-java").bounds();
            assert!(first.left() == last.left());
            assert!(last.bottom() < window.find("project-form-footer").bounds().top());
            assert!(
                window.find("project-form-footer").bounds().bottom()
                    <= window.viewport_size().height
            );
        })
        .unwrap();
        if mode == ThemeMode::Dark {
            native_language_versions(cx, window);
        }
        cx.update_window(window, |_, window, cx| {
            reveal_form(window, "project-languages", cx);
            let body = window.find("project-form-body").bounds();
            assert!(window.find("project-language-row-python").bounds().top() >= body.top());
            assert!(window.find("project-language-row-java").bounds().bottom() <= body.bottom());
        })
        .unwrap();
        native_screenshot(cx, window, screenshot, name, "languages");
        cx.update_window(window, |_, window, cx| {
            window.scroll(
                "project-form-body",
                gpui_kit::ScrollDelta::Pixels(gpui_kit::point(
                    gpui_kit::px(0.),
                    gpui_kit::px(500.),
                )),
                cx,
            );
        })
        .unwrap();
    }
}
fn native_screenshot(
    cx: &mut gpui_kit::HeadlessAppContext,
    window: AnyWindowHandle,
    screenshot: Option<&std::ffi::OsStr>,
    theme: &str,
    variant: &str,
) {
    if let Some(path) = screenshot {
        let base = std::path::PathBuf::from(path);
        let path = base.with_file_name(format!(
            "{}-{theme}-{variant}.png",
            base.file_stem().unwrap().to_string_lossy()
        ));
        cx.capture_screenshot(window).unwrap().save(path).unwrap();
    }
}

fn native_language_versions(cx: &mut gpui_kit::HeadlessAppContext, window: AnyWindowHandle) {
    for (mark, version) in [
        ("python", "3.13"),
        ("rust", "1.90.0"),
        ("bun", "1.3.3"),
        ("nodejs", "22"),
        ("java", "21"),
    ] {
        cx.update_window(window, |_, window, cx| {
            if mark != "python" {
                reveal_form(window, &format!("project-language-{mark}"), cx);
                window.click(format!("project-language-{mark}"), cx);
            }
            super::creation::open_custom_version(window, mark, cx);
        })
        .unwrap();
        cx.run_until_parked();
        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            reveal_form(window, &format!("project-version-custom-{mark}"), cx);
            fill(
                window,
                &format!("project-version-custom-{mark}"),
                version,
                cx,
            );
        })
        .unwrap();
    }
}

#[gpui_kit::test]
fn registry_search_keeps_original_local_authority_after_a_remote_tab_is_selected(
    cx: &mut TestAppContext,
) {
    let search = FakeImageSearch::default();
    search.respond(Ok(vec![registry_image("debian")])).unwrap();
    let (rig, _, exec) = image_form(search.clone(), cx);
    let _other = rig.parent(remote(None, "selected-later.example"), "Another server", cx);
    image_query(&rig, "debian", cx);
    wait_for_search(cx);
    let calls = search.calls().unwrap();
    assert_eq!(calls.len(), 1);
    assert!(Arc::ptr_eq(&calls[0].exec, &exec));
}

#[gpui_kit::test]
fn changing_engines_cancels_a_pending_search_and_never_reuses_its_results(cx: &mut TestAppContext) {
    let search = FakeImageSearch::default();
    let (send, receive) = async_channel::bounded::<()>(1);
    search
        .respond_after(
            Box::pin(async move {
                let _ = receive.recv().await;
            }),
            Ok(vec![registry_image("docker-only")]),
        )
        .unwrap();
    search
        .respond(Ok(vec![registry_image("podman-only")]))
        .unwrap();
    let (rig, _, _) = image_form(search.clone(), cx);
    image_query(&rig, "debian", cx);
    wait_for_search(cx);
    cx.update_window(rig.window, |_, window, cx| {
        window.click("project-engine-Podman", cx)
    })
    .unwrap();
    assert!(search.calls().unwrap()[0].cancel.is_cancelled());
    wait_for_search(cx);
    let _ = send.try_send(());
    cx.run_until_parked();
    cx.update_window(rig.window, |_, window, cx| {
        window.render_frame(cx);
        window.click("project-image-source-registry", cx);
        window.render_frame(cx);
        assert!(
            window
                .try_find("project-registry-image-docker-only")
                .is_none()
        );
        window.click("project-image-source-registry", cx);
        window.render_frame(cx);
        assert!(
            window
                .try_find("project-registry-image-podman-only")
                .is_some()
        );
    })
    .unwrap();
}

#[gpui_kit::test]
fn registry_cache_evicts_the_oldest_query_after_sixteen_entries(cx: &mut TestAppContext) {
    let search = FakeImageSearch::default();
    for _ in 0..18 {
        search.respond(Ok(vec![])).unwrap();
    }
    let (rig, _, _) = image_form(search.clone(), cx);
    for index in 0..17 {
        image_query(&rig, &format!("image-{index}"), cx);
        wait_for_search(cx);
    }
    image_query(&rig, "image-1", cx);
    wait_for_search(cx);
    assert_eq!(search.calls().unwrap().len(), 17);
    image_query(&rig, "image-0", cx);
    wait_for_search(cx);
    assert_eq!(search.calls().unwrap().len(), 18);
}

#[gpui_kit::test]
fn switching_image_sources_preserves_the_literal_reference_and_selected_local_row(
    cx: &mut TestAppContext,
) {
    let (rig, preparation, _) = image_form(FakeImageSearch::default(), cx);
    image_query(&rig, "team/debian13:BookWorm", cx);
    cx.update_window(rig.window, |_, window, cx| {
        window.click("project-local-image-debian-local", cx);
        window.render_frame(cx);
        window.click("project-image-source-registry", cx);
        window.render_frame(cx);
        assert_eq!(
            window.within("project-custom-image").find("input").value(),
            Some("team/debian13:BookWorm")
        );
        window.click("project-image-source-local", cx);
        window.render_frame(cx);
        assert_eq!(
            window.find("project-local-image-debian-local").checked(),
            Some(true)
        );
        create_workspace(window, cx);
    })
    .unwrap();
    cx.run_until_parked();
    assert!(
        matches!(&preparation.requests().unwrap()[0].mode, SetupMode::Create { image: nocterm_project_providers::WorkspaceImage::Custom(reference), .. } if reference == "team/debian13:BookWorm")
    );
}
