//! Browsing and releasing a stored chat must preserve its original file bytes.
use super::*;
use nocterm_ui::SettingsExt as _;

#[gpui_kit::test]
async fn browsing_hydrating_and_disabling_ai_preserve_an_untouched_v1_file(
    cx: &mut TestAppContext,
) {
    let directory = tempfile::tempdir().unwrap();
    let paths = nocterm_core::Paths::rooted_at(directory.path());
    let chats = paths.state_dir().join("agent-chats");
    std::fs::create_dir_all(&chats).unwrap();
    let id = "00000000-0000-4000-8000-000000000024";
    let file = chats.join(format!("{id}.json"));
    let bytes = format!(
        r#"{{
  "entries": [ {{ "value": "Old durable answer", "kind": "agent" }} ],
  "updated": 40, "times": [39], "name": "Preserved raw history",
  "agent_id": "codex", "id": "{id}", "version": 1,
  "pending_history": true
}}
"#
    )
    .into_bytes();
    nocterm_ai::history::decode(&bytes, id).unwrap();
    std::fs::write(&file, &bytes).unwrap();
    let (window, workspace) = cx.update(|cx| {
        let mut settings = nocterm_settings::SettingsDocument::default();
        settings.update::<nocterm_ai::AiSettings>(|ai| ai.sessions.warm_start = false);
        let booted = boot(
            directory,
            settings,
            Arc::new(MockTransport::default()),
            false,
            cx,
        );
        open_workspace(&booted, cx)
    });
    cx.run_until_parked();
    cx.update_window(window, |_, window, cx| {
        window.activate_window();
        window.focus(&workspace.read(cx).focus_handle(cx), cx);
        window.dispatch_action(Box::new(nocterm_workspace::ToggleRightPanel), cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(window, |_, window, cx| {
        window.render_frame(cx);
        let row = gpui_kit::base::test_support::snapshots(window)
            .into_iter()
            .find(|row| row.label() == Some("Preserved raw history"))
            .expect("the physical repository supplied the saved chat row");
        window.click(row.path().last().unwrap().clone(), cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(window, |_, window, cx| {
        window.render_frame(cx);
        assert!(
            window.try_find(("agent-entry", 0usize)).is_some(),
            "the stored transcript was actually hydrated and rendered"
        );
        window.dispatch_action(Box::new(nocterm_workspace::ToggleRightPanel), cx);
        cx.update_setting::<nocterm_ai::AiSettings>(|ai| ai.enabled = false)
            .detach();
    })
    .unwrap();
    cx.run_until_parked();
    cx.update(|cx| assert!(!workspace.read(cx).right_panel_is_available()));
    let final_work = cx.update(|cx| {
        nocterm_agent_runtime::Runtime::global(cx)
            .update(cx, |runtime, cx| runtime.capture_shutdown(cx))
    });
    final_work.await.unwrap();
    assert_eq!(
        std::fs::read(file).unwrap(),
        bytes,
        "untouched v1 bytes, field order, spacing and timestamp must survive lifecycle observation"
    );
}
