//! Archived full-text search uses the same admitted native-body service.
use super::*;
use nocterm_agent_runtime::history_admission::{
    MAX_NORMAL_HISTORY_BYTES, MAX_NORMAL_HISTORY_PAYLOADS,
};
use nocterm_ai::metadata::ChatMetadataSource as _;

#[gpui_kit::test]
fn changing_an_archive_search_keeps_the_old_decoded_worker_charged_until_actual_drop(
    cx: &mut TestAppContext,
) {
    let f = fixture(cx);
    cx.run_until_parked();
    let identity = f._directory.metadata.new_identity();
    let mut chat =
        nocterm_ai::history::SavedChat::for_identity("codex".into(), identity.id, identity.created);
    chat.header.name = Some("Neutral saved title".into());
    chat.entries.push(nocterm_ai::thread::Entry::Agent(format!(
        "oldnativeone {}",
        "x".repeat(2 * 1024 * 1024)
    )));
    let thread = super::super::archive::archived(&f, &chat, cx);
    let decoded = f
        .repository
        .block_next(nocterm_ai::testing::StorageOperation::Decoded(
            chat.header.id.clone(),
        ));
    super::super::panel_actions::show_history(&f, cx);
    cx.update_window(f.handle, |_, window, cx| {
        replace_textbox("agent-history-search", "oldnativeone", window, cx);
    })
    .unwrap();
    cx.run_until_parked();
    assert!(
        f.repository
            .operations()
            .contains(&nocterm_ai::testing::StorageOperation::Decoded(
                chat.header.id.clone()
            ))
    );
    let admission = cx.update(|cx| f.installation.installed.runtime().history_admission(cx));
    cx.update_window(f.handle, |_, window, cx| {
        replace_textbox("agent-history-search", "replacement_no_match", window, cx);
    })
    .unwrap();
    cx.run_until_parked();
    let slots = (0..MAX_NORMAL_HISTORY_PAYLOADS)
        .filter_map(|_| admission.reserve_payload().ok())
        .collect::<Vec<_>>();
    assert!(admission.reserve_payload().is_err());
    cx.update(|cx| {
        assert!(
            admission
                .reserve_unowned_bytes(MAX_NORMAL_HISTORY_BYTES - 2 * 1024 * 1024, cx)
                .is_err()
        )
    });
    decoded.send(Ok(())).unwrap();
    cx.run_until_parked();
    let released = admission.reserve_payload().unwrap();
    cx.update_window(f.handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("agent-history-no-match").is_some());
        assert!(
            window
                .try_find(("history-thread", thread.entity_id().as_u64()))
                .is_none()
        );
    })
    .unwrap();
    drop((released, slots));
}
