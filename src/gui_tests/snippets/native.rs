//! Native snippet editor receipts and literal files through public workspace actions.
use super::*;
use gpui_kit::{App, Window, component::WindowExt as _};
use nocterm_snippets::{Language, Snippet, SnippetRepository as _};
use nocterm_snippets_ui::Installed;
fn native_installation(
    paths: &nocterm_core::Paths,
    ids: Arc<dyn nocterm_snippets::SnippetIds>,
    cx: &mut App,
) -> Installed {
    let settings = nocterm_store::SettingsConfig::in_memory(Default::default()).install(cx);
    let repository = Arc::new(nocterm_snippets_io::FileSnippetRepository::new(
        paths.snippets_file(),
    ));
    let prepared = nocterm_snippets_store::PreparedLibrary::loaded(repository.load(), true);
    let writer: nocterm_store::SnapshotWriter<nocterm_snippets::Library> =
        Arc::new(move |library| {
            let repository = repository.clone();
            Box::pin(async move {
                repository
                    .save(&library)
                    .map_err(nocterm_store::StoreError::backend)
            })
        });
    let model = nocterm_snippets_store::SnippetsHandle::install(
        prepared,
        writer,
        &nocterm_store::ShutdownCoordinator::default(),
        cx,
    )
    .unwrap();
    nocterm_snippets_ui::init(model, settings, ids, cx)
}
fn native_fixture(
    paths: &nocterm_core::Paths,
    ids: Arc<dyn nocterm_snippets::SnippetIds>,
    cx: &mut TestAppContext,
) -> (AnyWindowHandle, Entity<Workspace>, Installed) {
    cx.update(|cx| {
        cx.set_reduce_motion(true);
        gpui_kit::init(cx);
        cx.set_global(nocterm_ui::Design::new(nocterm_ui::DesignTokens::builtin()));
        let installed = native_installation(paths, ids, cx);
        let (handle, workspace) = gpui_kit::open_window(Default::default(), cx, |window, cx| {
            cx.new(|cx| {
                let deps = nocterm_workspace::test_support::deps_for(
                    &[&nocterm_snippets_ui::plugin::PLUGIN],
                    cx,
                );
                let mut workspace = Workspace::new(deps, window, cx);
                nocterm_snippets_ui::register(&mut workspace, &installed, window, cx).unwrap();
                workspace
            })
        })
        .unwrap();
        (handle, workspace, installed)
    })
}
fn loaded_native_fixture(
    paths: &nocterm_core::Paths,
    original: &Snippet,
    cx: &mut TestAppContext,
) -> (AnyWindowHandle, Entity<Workspace>, Installed) {
    nocterm_core::persist::save(
        &paths.snippets_file(),
        &nocterm_snippets::Library {
            snippets: vec![original.clone()],
        },
    )
    .unwrap();
    native_fixture(
        paths,
        Arc::new(nocterm_snippets::test_support::SequenceIds::new(10_000)),
        cx,
    )
}
fn replace(window: &mut Window, id: &'static str, value: &str, cx: &mut App) {
    window.render_frame(cx);
    window.click(id, cx);
    window.press("ctrl-a", cx);
    window.press("backspace", cx);
    window.input(value, cx);
}
fn assert_input(window: &mut Window, id: &'static str, expected: &str, cx: &mut App) {
    window.click(id, cx);
    assert_eq!(
        window.focused_input(cx).unwrap().value(cx).as_ref(),
        expected
    );
}
fn begin(handle: AnyWindowHandle, workspace: &Entity<Workspace>, cx: &mut TestAppContext) {
    cx.update_window(handle, |_, window, cx| {
        window.activate_window();
        window.focus(&workspace.read(cx).focus_handle(cx), cx);
        window.render_frame(cx);
        window.dispatch_action(Box::new(nocterm_snippets_ui::NewSnippet), cx);
    })
    .unwrap();
    cx.run_until_parked();
}
#[gpui_kit::test]
fn native_failed_save_retains_literal_draft_and_original_expectation_for_retry(
    cx: &mut TestAppContext,
) {
    let directory = tempfile::tempdir().unwrap();
    let paths = nocterm_core::Paths::rooted_at(directory.path());
    let original = Snippet {
        name: "Original".into(),
        content: "echo original".into(),
        profiles: vec!["specific-server".into()],
        groups: vec!["Work".into()],
        ..Snippet::new(uuid::Uuid::from_u128(13))
    };
    let (handle, workspace, installed) = loaded_native_fixture(&paths, &original, cx);
    // Keep the already-loaded library writable but refuse its next physical atomic publication.
    std::fs::remove_file(paths.snippets_file()).unwrap();
    std::fs::create_dir(paths.snippets_file()).unwrap();
    cx.update_window(handle, |_, window, cx| {
        show_library(&workspace, window, cx);
        window.click("edit", cx);
        replace(window, "snippet-name", "  Native literal  ", cx);
        replace(window, "snippet-description", "First\nSecond 日本語", cx);
        replace(window, "snippet-code", "print('kept')\n", cx);
        window.click("python", cx);
        window.click("snippet-section-attachments", cx);
        window.render_frame(cx);
        window.click("group-Work", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("snippet-save", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("snippet-error").is_some());
        assert_eq!(installed.library(cx).snippets, [original]);
        assert!(installed.error(cx).is_some());
        window.click("snippet-section-snippet", cx);
        window.render_frame(cx);
        assert_input(window, "snippet-name", "  Native literal  ", cx);
        assert_input(window, "snippet-description", "First\nSecond 日本語", cx);
        assert_input(window, "snippet-code", "print('kept')\n", cx);
    })
    .unwrap();
    std::fs::remove_dir(paths.snippets_file()).unwrap();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("snippet-save", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("dialog").is_none());
        let saved = &installed.library(cx).snippets[0];
        assert_eq!(saved.name, "Native literal");
        assert_eq!(saved.description, "First\nSecond 日本語");
        assert_eq!(saved.content, "print('kept')\n");
        assert_eq!(saved.language, Language::Python);
        assert_eq!(saved.profiles, ["specific-server"]);
        assert!(saved.groups.is_empty());
        assert!(installed.error(cx).is_none());
        assert_eq!(
            nocterm_core::persist::load::<nocterm_snippets::Library>(&paths.snippets_file())
                .unwrap()
                .unwrap()
                .snippets
                .as_slice(),
            std::slice::from_ref(saved)
        );
    })
    .unwrap();
}

#[gpui_kit::test]
fn corrupt_native_library_is_readonly_and_its_literal_text_survives_editor_save(
    cx: &mut TestAppContext,
) {
    let directory = tempfile::tempdir().unwrap();
    let paths = nocterm_core::Paths::rooted_at(directory.path());
    std::fs::create_dir_all(paths.snippets_file().parent().unwrap()).unwrap();
    std::fs::write(
        paths.snippets_file(),
        "# untouched malformed source\nsnippets = =",
    )
    .unwrap();
    let (handle, workspace, installed) = native_fixture(
        &paths,
        Arc::new(nocterm_snippets::test_support::SequenceIds::new(10_000)),
        cx,
    );
    assert!(!cx.update(|cx| installed.writable(cx)));
    begin(handle, &workspace, cx);
    cx.update_window(handle, |_, window, cx| {
        replace(window, "snippet-name", "Unsaved", cx);
        replace(window, "snippet-code", "echo literal", cx);
        window.click("snippet-save", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("snippet-error").is_some());
        assert_input(window, "snippet-name", "Unsaved", cx);
        assert_input(window, "snippet-code", "echo literal", cx);
        assert!(installed.library(cx).snippets.is_empty());
    })
    .unwrap();
    assert_eq!(
        std::fs::read_to_string(paths.snippets_file()).unwrap(),
        "# untouched malformed source\nsnippets = ="
    );
}

#[gpui_kit::test]
fn a_new_native_draft_keeps_its_single_identity_across_publication_failure_and_retry(
    cx: &mut TestAppContext,
) {
    let directory = tempfile::tempdir().unwrap();
    let paths = nocterm_core::Paths::rooted_at(directory.path());
    let ids = Arc::new(nocterm_snippets::test_support::SequenceIds::new(60_000));
    let (handle, workspace, installed) = native_fixture(&paths, ids.clone(), cx);
    std::fs::create_dir_all(paths.snippets_file()).unwrap();
    begin(handle, &workspace, cx);
    cx.update_window(handle, |_, window, cx| {
        replace(window, "snippet-name", "New native", cx);
        replace(window, "snippet-code", "echo kept\n", cx);
        window.click("snippet-save", cx);
    })
    .unwrap();
    cx.run_until_parked();
    assert_eq!(ids.calls(), 1);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("snippet-error").is_some());
        assert_input(window, "snippet-name", "New native", cx);
        assert_input(window, "snippet-code", "echo kept\n", cx);
        assert!(installed.library(cx).snippets.is_empty());
    })
    .unwrap();
    std::fs::remove_dir(paths.snippets_file()).unwrap();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("snippet-save", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("dialog").is_none());
        let saved = &installed.library(cx).snippets[0];
        assert_eq!(saved.id, uuid::Uuid::from_u128(60_000));
        assert_eq!(saved.content, "echo kept\n");
    })
    .unwrap();
    assert_eq!(ids.calls(), 1, "physical retry cannot regenerate the draft");
}
