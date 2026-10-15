use super::*;
use gpui_kit::base::TestSupportExt as _;
use gpui_kit::{AnyWindowHandle, TestAppContext, WindowOptions, test::TestWindowExt as _};
use nocterm_ui::{Design, DesignTokens};
fn workspace(cx: &mut TestAppContext) -> (AnyWindowHandle, Entity<Workspace>, Installed) {
    cx.update(|cx| {
        cx.set_reduce_motion(true);
        gpui_kit::init(cx);
        cx.set_global(Design::new(DesignTokens::builtin()));
        let settings = nocterm_store::SettingsConfig::in_memory(Default::default()).install(cx);
        let installed = crate::init(
            None,
            settings,
            &nocterm_store::ShutdownCoordinator::default(),
            cx,
        )
        .unwrap();
        let (handle, workspace) =
            gpui_kit::open_window(WindowOptions::default(), cx, |window, cx| {
                cx.new(|cx| {
                    let deps =
                        nocterm_workspace::Deps::new(nocterm_ui::LayoutHandle::in_memory(cx));
                    Workspace::new(deps, window, cx)
                })
            })
            .unwrap();
        (handle, workspace, installed)
    })
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
fn begin(installed: &Installed, workspace: &Entity<Workspace>, window: &mut Window, cx: &mut App) {
    open(installed.clone(), None, workspace.downgrade(), window, cx);
    window.render_frame(cx);
}
#[gpui_kit::test]
fn multiline_enter_does_not_save_then_explicit_save_preserves_code(cx: &mut TestAppContext) {
    let (handle, workspace, installed) = workspace(cx);
    cx.update_window(handle, |_, window, cx| {
        begin(&installed, &workspace, window, cx);
        window.input("Disk usage", cx);
        window.click("snippet-code", cx);
        window.input("echo first", cx);
        window.press("enter", cx);
        window.input("echo second", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("dialog").is_some());
        assert!(installed.library(cx).snippets.is_empty());
        assert!(window.focused_input(cx).unwrap().value(cx).contains('\n'));
        window.click("python", cx);
        window.click("bash", cx);
        window.click("snippet-save", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("dialog").is_none());
        let snippets = &installed.library(cx).snippets;
        assert_eq!(snippets.len(), 1);
        assert_eq!(snippets[0].name, "Disk usage");
        assert_eq!(snippets[0].content, "echo first\necho second");
        assert_eq!(snippets[0].language, Language::Bash);
    })
    .unwrap();
}
#[gpui_kit::test]
fn validation_and_cancel_keep_library_unchanged(cx: &mut TestAppContext) {
    let (handle, workspace, installed) = workspace(cx);
    cx.update_window(handle, |_, window, cx| {
        begin(&installed, &workspace, window, cx);
        window.click("snippet-save", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("dialog").is_some());
        assert!(window.try_find("snippet-error").is_some());
        window.click("snippet-cancel", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("dialog").is_none());
        assert!(installed.library(cx).snippets.is_empty());
    })
    .unwrap();
}
#[test]
fn selected_languages_have_real_bundled_parsers() {
    use gpui_kit::component::{Rope, highlighter::SyntaxHighlighter};
    for (language, code) in [
        (Language::Bash, "echo \"hello\""),
        (Language::Json, "{\"ok\": true}"),
        (Language::Python, "print('hello')"),
        (Language::Yaml, "enabled: true\n"),
        (Language::Toml, "enabled = true\n"),
    ] {
        let mut highlighter = SyntaxHighlighter::new(language.id());
        assert_eq!(highlighter.language().as_ref(), language.id());
        assert!(highlighter.update(None, &Rope::from(code), None));
        assert!(highlighter.tree().is_some(), "{} must parse", language.id());
    }
}
#[gpui_kit::test]
fn save_under_stacked_dialog_keeps_both_dialogs_usable(cx: &mut TestAppContext) {
    let (handle, workspace, installed) = workspace(cx);
    cx.update_window(handle, |_, window, cx| {
        begin(&installed, &workspace, window, cx);
        window.input("Inspect", cx);
        replace(window, "snippet-code", "echo first", cx);
        window.click("snippet-save", cx);
        let editor_focus = window.focused(cx);
        window.open_dialog(cx, |dialog, _, _| {
            dialog.title("About").child(
                gpui_kit::div()
                    .id("about-content")
                    .test_support()
                    .child("Nocterm"),
            )
        });
        let about = window.focused(cx);
        assert!(
            about.is_some() && about != editor_focus,
            "About initially owns focus"
        );
        window.render_frame(cx);
        assert!(window.focused(cx) == about, "About owns focus after render");
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(
            window.try_find("about-content").is_some(),
            "Saving must preserve the newer dialog"
        );
        assert_eq!(installed.library(cx).snippets[0].content, "echo first");
        window.close_dialog(cx);
        window.render_frame(cx);
        assert!(window.try_find("snippet-saved").is_some());
        replace(window, "snippet-code", "echo second", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(
            window.try_find("snippet-saved").is_none(),
            "Editing clears saved status"
        );
        window.click("snippet-save", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(
            window.try_find("dialog").is_none(),
            "Resurfaced editor saves and closes"
        );
        assert_eq!(installed.library(cx).snippets[0].content, "echo second");
    })
    .unwrap();
}
#[gpui_kit::test]
fn attachment_navigation_preserves_draft_and_validation_returns_to_snippet(
    cx: &mut TestAppContext,
) {
    let (handle, workspace, installed) = workspace(cx);
    let original = Snippet {
        name: "Original".into(),
        description: "First\nSecond".into(),
        content: "echo preserved".into(),
        profiles: vec!["specific-server".into()],
        groups: vec!["Work".into()],
        ..Snippet::new(uuid::Uuid::from_u128(11))
    };
    cx.update(|cx| installed.save(original.clone(), None, cx).detach());
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        open(
            installed.clone(),
            Some(original.clone()),
            workspace.downgrade(),
            window,
            cx,
        );
        replace(window, "snippet-name", "", cx);
        assert_input(window, "snippet-name", "", cx);
        window.click("snippet-section-attachments", cx);
        window.render_frame(cx);
        assert!(window.try_find("group-Work").is_some());
        window.click("group-Work", cx);
        replace(window, "snippet-attachment-search", "unmatched", cx);
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
        assert_eq!(window.focused_input(cx).unwrap().value(cx).as_ref(), "");
        replace(window, "snippet-name", "Accepted", cx);
        window.click("snippet-description", cx);
        assert_eq!(
            window.focused_input(cx).unwrap().value(cx).as_ref(),
            "First\nSecond"
        );
        window.click("snippet-code", cx);
        assert_eq!(
            window.focused_input(cx).unwrap().value(cx).as_ref(),
            "echo preserved"
        );
        window.click("snippet-save", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update(|cx| {
        let saved = &installed.library(cx).snippets[0];
        assert_eq!(
            saved.profiles,
            ["specific-server"],
            "Group choice never expands into direct profiles"
        );
        assert!(saved.groups.is_empty());
        assert_eq!(saved.description, "First\nSecond");
        assert_eq!(saved.content, "echo preserved");
    });
}
#[gpui_kit::test]
fn minimum_window_keeps_footer_visible_on_both_pages(cx: &mut TestAppContext) {
    let (handle, workspace, installed) = workspace(cx);
    cx.simulate_window_resize(handle, gpui_kit::size(px(640.), px(400.)));
    cx.update_window(handle, |_, window, cx| {
        begin(&installed, &workspace, window, cx)
    })
    .unwrap();
    for section in ["snippet-section-snippet", "snippet-section-attachments"] {
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            window.click(section, cx);
        })
        .unwrap();
        cx.run_until_parked();
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            for button in ["snippet-save", "snippet-cancel"] {
                assert!(
                    window.find(button).bounds().bottom() <= window.viewport_size().height,
                    "{section}: {button} must stay visible"
                );
            }
        })
        .unwrap();
    }
}
#[gpui_kit::test]
fn an_existing_editor_saves_only_its_original_installation_without_settings_global(
    cx: &mut TestAppContext,
) {
    let (handle, workspace, first) = workspace(cx);
    cx.update_window(handle, |_, window, cx| {
        begin(&first, &workspace, window, cx);
        window.input("Original owner", cx);
        replace(window, "snippet-code", "echo original", cx);
    })
    .unwrap();
    let second = cx.update(|cx| {
        let settings = nocterm_store::SettingsConfig::in_memory(Default::default()).install(cx);
        crate::init(
            None,
            settings,
            &nocterm_store::ShutdownCoordinator::default(),
            cx,
        )
        .unwrap()
    });
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("snippet-save", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update(|cx| {
        assert_eq!(first.library(cx).snippets[0].name, "Original owner");
        assert_eq!(first.library(cx).snippets[0].content, "echo original");
        assert!(second.library(cx).snippets.is_empty());
    });
}

#[gpui_kit::test]
fn shutdown_registration_failure_returns_no_editable_installation(cx: &mut TestAppContext) {
    cx.update(|cx| {
        let settings = nocterm_store::SettingsConfig::in_memory(Default::default()).install(cx);
        let shutdown = nocterm_store::ShutdownCoordinator::default();
        drop(shutdown.capture_all(cx));
        assert!(matches!(
            crate::init(None, settings, &shutdown, cx),
            Err(nocterm_store::StoreError::Closing)
        ));
    });
}

#[gpui_kit::test]
fn conflicting_save_keeps_literal_controls_and_original_expected_record_for_retry(
    cx: &mut TestAppContext,
) {
    let (handle, workspace, installed) = workspace(cx);
    let original = Snippet {
        name: "Original".into(),
        content: "echo original".into(),
        ..Snippet::new(uuid::Uuid::from_u128(12))
    };
    cx.update(|cx| installed.save(original.clone(), None, cx).detach());
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        open(
            installed.clone(),
            Some(original.clone()),
            workspace.downgrade(),
            window,
            cx,
        );
        replace(window, "snippet-name", "Literal edit", cx);
        replace(window, "snippet-description", "First\nSecond 日本語", cx);
        replace(window, "snippet-code", "print('kept')\n", cx);
        window.click("python", cx);
    })
    .unwrap();
    let mut external = original.clone();
    external.name = "External".into();
    cx.update(|cx| {
        installed
            .save(external.clone(), Some(original.clone()), cx)
            .detach()
    });
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
        assert_eq!(installed.library(cx).snippets[0], external);
        assert_input(window, "snippet-name", "Literal edit", cx);
        assert_input(window, "snippet-description", "First\nSecond 日本語", cx);
        assert_input(window, "snippet-code", "print('kept')\n", cx);
    })
    .unwrap();
    cx.update(|cx| {
        installed
            .save(original.clone(), Some(external), cx)
            .detach()
    });
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("snippet-save", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update(|cx| {
        let saved = &installed.library(cx).snippets[0];
        assert_eq!(saved.name, "Literal edit");
        assert_eq!(saved.description, "First\nSecond 日本語");
        assert_eq!(saved.content, "print('kept')\n");
        assert_eq!(saved.language, Language::Python);
    });
}

fn native_installation(
    paths: &nocterm_core::Paths,
    original: &Snippet,
    cx: &mut TestAppContext,
) -> Installed {
    nocterm_core::persist::save(
        &paths.snippets_file(),
        &nocterm_snippets::Library {
            snippets: vec![original.clone()],
        },
    )
    .unwrap();
    cx.update(|cx| {
        let settings = nocterm_store::SettingsConfig::in_memory(Default::default()).install(cx);
        crate::init(
            Some(paths),
            settings,
            &nocterm_store::ShutdownCoordinator::default(),
            cx,
        )
        .unwrap()
    })
}

#[gpui_kit::test]
fn native_failed_save_retains_literal_draft_and_original_expectation_for_retry(
    cx: &mut TestAppContext,
) {
    let (handle, workspace, _) = workspace(cx);
    let directory = tempfile::tempdir().unwrap();
    let paths = nocterm_core::Paths::rooted_at(directory.path());
    let original = Snippet {
        name: "Original".into(),
        content: "echo original".into(),
        profiles: vec!["specific-server".into()],
        groups: vec!["Work".into()],
        ..Snippet::new(uuid::Uuid::from_u128(13))
    };
    let installed = native_installation(&paths, &original, cx);
    // Keep the already-loaded library writable but refuse its next physical atomic publication.
    std::fs::remove_file(paths.snippets_file()).unwrap();
    std::fs::create_dir(paths.snippets_file()).unwrap();
    cx.update_window(handle, |_, window, cx| {
        open(
            installed.clone(),
            Some(original.clone()),
            workspace.downgrade(),
            window,
            cx,
        );
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
