use super::*;
use gpui_kit::{AnyWindowHandle, TestAppContext, WindowOptions, test::TestWindowExt as _};
use nocterm_ui::{Design, DesignTokens};

fn fixture(
    cx: &mut TestAppContext,
) -> (
    AnyWindowHandle,
    Entity<SnippetsPanel>,
    Entity<Workspace>,
    Installed,
) {
    fixture_library(cx, nocterm_snippets::Library::default())
}

fn fixture_library(
    cx: &mut TestAppContext,
    library: nocterm_snippets::Library,
) -> (
    AnyWindowHandle,
    Entity<SnippetsPanel>,
    Entity<Workspace>,
    Installed,
) {
    let (handle, workspace, installed) = cx.update(|cx| {
        cx.set_reduce_motion(true);
        gpui_kit::init(cx);
        cx.set_global(Design::new(DesignTokens::builtin()));
        let settings = nocterm_store::SettingsConfig::in_memory(Default::default()).install(cx);
        let model = nocterm_snippets_store::SnippetsHandle::install(
            nocterm_snippets_store::PreparedLibrary::loaded(Ok(Some(library)), false),
            std::sync::Arc::new(|_| Box::pin(async { Ok(()) })),
            &nocterm_store::ShutdownCoordinator::default(),
            cx,
        )
        .unwrap();
        let installed = crate::init(
            model,
            settings,
            std::sync::Arc::new(nocterm_snippets::test_support::SequenceIds::new(10_000)),
            cx,
        );
        let (handle, workspace) =
            gpui_kit::open_window(WindowOptions::default(), cx, |window, cx| {
                cx.new(|cx| {
                    let deps =
                        nocterm_workspace::test_support::deps_for(&[&crate::plugin::PLUGIN], cx);
                    Workspace::new(deps, window, cx)
                })
            })
            .unwrap();
        (handle, workspace, installed)
    });
    let panel = cx
        .update_window(handle, |_, window, cx| {
            window.activate_window();
            let panel =
                cx.new(|cx| SnippetsPanel::new(installed.clone(), workspace.clone(), window, cx));
            workspace.update(cx, |workspace, cx| {
                // Test seam: the fixture needs the entity for assertions, so the factory
                // returns a panel built earlier. Production factories create their own.
                let existing = panel.clone();
                workspace
                    .register_panel(
                        nocterm_workspace::PanelRegistration::new(
                            &crate::plugin::LIBRARY,
                            move |_, _, _| existing.clone(),
                        ),
                        window,
                        cx,
                    )
                    .unwrap();
                workspace.open_panel(&crate::plugin::LIBRARY.id, window, cx);
            });
            panel
        })
        .unwrap();
    cx.run_until_parked();
    (handle, panel, workspace, installed)
}

fn snippet() -> Snippet {
    Snippet {
        name: "Disk usage 日本語".into(),
        description: "Описание команды".into(),
        content: "#!/bin/bash\necho 'привет'\n\n".into(),
        groups: vec!["Missing group".into()],
        profiles: vec!["missing-server".into()],
        ..Snippet::new(uuid::Uuid::from_u128(1))
    }
}

fn seed(snippet: &Snippet, installed: &Installed, cx: &mut TestAppContext) {
    cx.update(|cx| {
        installed.save(snippet.clone(), None, cx).detach();
    });
    cx.run_until_parked();
}

#[gpui_kit::test]
fn plus_opens_editor_and_cancel_preserves_library(cx: &mut TestAppContext) {
    let (handle, _, _, installed) = fixture(cx);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("snippet-new", cx);
        window.render_frame(cx);
        assert!(window.try_find("dialog").is_some());
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

#[gpui_kit::test]
fn fold_search_copy_and_edit_preserve_exact_saved_snippet(cx: &mut TestAppContext) {
    let (handle, _, _, installed) = fixture(cx);
    let original = snippet();
    seed(&original, &installed, cx);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("copy").is_some());
        window.click("snippets-other", cx);
        window.render_frame(cx);
        assert!(window.try_find("copy").is_none());
        window.click("snippets-other", cx);
        window.render_frame(cx);
        window.click("copy", cx);
        assert_eq!(
            cx.read_from_clipboard().unwrap().text().unwrap(),
            original.content
        );
        window.click("snippet-library-search", cx);
        window.input("unmatched query", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("copy").is_none());
        window.press("ctrl-a", cx);
        window.input("ПРИВЕТ", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("copy").is_some());
        window.click("edit", cx);
        window.render_frame(cx);
        assert!(window.try_find("dialog").is_some());
        window.click("snippet-section-attachments", cx);
        window.render_frame(cx);
        assert!(window.try_find("profile-missing-server").is_some());
        assert!(window.try_find("group-Missing group").is_some());
        window.click("snippet-save", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("dialog").is_none());
        assert_eq!(
            installed.library(cx).snippets.as_slice(),
            std::slice::from_ref(&original)
        );
        window.click("edit", cx);
        window.render_frame(cx);
        window.click("snippet-section-attachments", cx);
        window.render_frame(cx);
        window.click("profile-missing-server", cx);
        window.click("group-Missing group", cx);
        window.click("snippet-save", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("dialog").is_none());
        let mut unassigned = original;
        unassigned.profiles.clear();
        unassigned.groups.clear();
        assert_eq!(installed.library(cx).snippets, [unassigned]);
    })
    .unwrap();
}

#[gpui_kit::test]
fn deletion_requires_separate_confirmation_and_cancel_keeps_snippet(cx: &mut TestAppContext) {
    let (handle, _, _, installed) = fixture(cx);
    let original = snippet();
    seed(&original, &installed, cx);
    let separate = cx.update(|cx| {
        let settings = nocterm_store::SettingsConfig::in_memory(Default::default()).install(cx);
        crate::test_support::install(settings, &nocterm_store::ShutdownCoordinator::default(), cx)
            .unwrap()
    });
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("delete", cx);
        window.render_frame(cx);
        assert!(window.try_find("snippet-save").is_none());
        assert!(window.try_find("ok").is_some());
        assert_eq!(
            installed.library(cx).snippets.as_slice(),
            std::slice::from_ref(&original)
        );
        window.click("cancel", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(installed.library(cx).snippets, [original]);
        window.click("delete", cx);
        window.render_frame(cx);
        window.click("ok", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("ok").is_none());
        assert!(installed.library(cx).snippets.is_empty());
        assert!(separate.library(cx).snippets.is_empty());
    })
    .unwrap();
}
#[path = "run_tests.rs"]
mod run_tests;

fn rendered_snippets(window: &Window) -> Vec<usize> {
    (1..=256)
        .filter(|index| {
            window
                .try_find(SharedString::from(format!(
                    "snippet-{}",
                    uuid::Uuid::from_u128(*index as u128)
                )))
                .is_some()
        })
        .collect()
}

fn scroll_snippets_to_end(window: &mut Window, cx: &mut gpui_kit::App) {
    let mut visible = rendered_snippets(window);
    // Variable row heights replace estimates as we arrive. Native extent can therefore
    // grow at the end of a long jump; convergence must stay bounded and make progress.
    for _ in 0..3 {
        if visible.contains(&256) {
            break;
        }
        let previous = visible.last().copied().unwrap_or(0);
        window.scroll(
            "snippets-list",
            gpui_kit::ScrollDelta::Pixels(gpui_kit::point(
                gpui_kit::px(0.),
                gpui_kit::px(-100_000.),
            )),
            cx,
        );
        visible = rendered_snippets(window);
        assert!(
            visible.last().is_some_and(|last| *last > previous),
            "native wheel stopped progressing: {visible:?}"
        );
    }
    assert!(
        visible.contains(&256),
        "native wheel must reach the exact final row within four jumps: {visible:?}"
    );
}

#[gpui_kit::test]
fn a_large_library_builds_visible_rows_and_scrolls_to_the_exact_last_content(
    cx: &mut TestAppContext,
) {
    let snippets: Vec<_> = (0..256)
        .map(|index| Snippet {
            name: format!("Snippet {index:03}"),
            content: format!("echo {index}\n{}", "# padding\n".repeat(2000)),
            ..Snippet::new(uuid::Uuid::from_u128(index + 1))
        })
        .collect();
    let last = snippets.last().unwrap().clone();
    let (handle, _, _, _) = fixture_library(cx, nocterm_snippets::Library { snippets });
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let materialized = rendered_snippets(window).len();
        assert!(
            (1..40).contains(&materialized),
            "materialized {materialized}/256 rows"
        );
        assert!(
            window
                .try_find(SharedString::from(format!("snippet-{}", last.id)))
                .is_none()
        );
        window.scroll(
            "snippets-list",
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
        scroll_snippets_to_end(window, cx);
        window.click("snippet-library-search", cx);
        window.input("Snippet 255", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("copy", cx);
        assert_eq!(
            cx.read_from_clipboard().unwrap().text().unwrap(),
            last.content
        );
    })
    .unwrap();
}

#[gpui_kit::test]
fn a_filtered_library_updates_after_a_saved_record_changes(cx: &mut TestAppContext) {
    let original = snippet();
    let (handle, _, _, installed) = fixture_library(
        cx,
        nocterm_snippets::Library {
            snippets: vec![original.clone()],
        },
    );
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("snippet-library-search", cx);
        window.input("future", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("copy").is_none());
    })
    .unwrap();
    let mut changed = original.clone();
    changed.content = "future exact content".into();
    cx.update(|cx| installed.save(changed.clone(), Some(original), cx).detach());
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("copy", cx);
        assert_eq!(
            cx.read_from_clipboard().unwrap().text().unwrap(),
            changed.content
        );
    })
    .unwrap();
}
