//! Empty provider sessions have no conversation to restore.
use super::*;
use nocterm_ai::{
    history::{SavedChat, SavedPrompt},
    thread::Entry,
};

/// Opens `chat` without warm start, so it restores when work starts.
fn adopt_saved(
    f: &mut Fixture,
    chat: SavedChat,
    cx: &mut TestAppContext,
) -> Entity<crate::thread::AgentThread> {
    lazy_start(f, cx);
    reinstall_history(f, vec![chat], cx);
    cx.update_window(f.handle, |_, window, cx| {
        f.panel
            .update(cx, |panel, cx| panel.adopt_saved_chats(window, cx));
    })
    .unwrap();
    cx.run_until_parked();
    let thread = cx.update(|cx| f.panel.read(cx).chat_documents().next().unwrap().clone());
    f.panel
        .update(cx, |panel, cx| panel.open_thread(thread.entity_id(), cx));
    cx.run_until_parked();
    thread
}

fn empty_chat(f: &Fixture) -> SavedChat {
    let chat = SavedChat::for_identity(
        &f.installation.retention,
        "codex".into(),
        "00000000-0000-4000-8000-00000000006f".into(),
        1,
    );
    let (id, metadata, mut resume, transcript, mut composer, _) = chat.into_parts();
    resume.session_id = Some("never-persisted-provider-session".into());
    resume.workdir = Some(f._directory.path().join("work"));
    composer.draft = Some("literal unsent text".into());
    SavedChat::from_parts(
        &f.installation.retention,
        id,
        metadata,
        resume,
        transcript,
        composer,
    )
    .unwrap()
}

#[gpui_kit::test]
fn draft_only_history_starts_one_fresh_session_without_restoring(cx: &mut TestAppContext) {
    let mut f = fixture(cx);
    let saved = empty_chat(&f);
    let thread = adopt_saved(&mut f, saved, cx);
    assert_eq!(f.commands.sessions.load(Ordering::SeqCst), 0);
    assert!(f.commands.restores.lock().unwrap().is_empty());
    cx.update(|cx| {
        let snapshot = thread.read(cx).snapshot(cx).unwrap();
        assert!(snapshot.resume().session_id.is_none());
        assert!(snapshot.resume().workdir.is_none());
        assert_eq!(
            snapshot.composer().draft.as_deref(),
            Some("literal unsent text")
        );
    });
    thread.update(cx, |thread, cx| {
        thread.send("first actual request".into(), cx)
    });
    cx.run_until_parked();
    assert!(f.commands.restores.lock().unwrap().is_empty());
    assert_eq!(f.commands.sessions.load(Ordering::SeqCst), 1);
    assert_eq!(f.commands.prompts.lock().unwrap().len(), 1);
    cx.update(|cx| assert!(!thread.read(cx).document().pending_history()));
}

#[gpui_kit::test]
fn empty_fork_descriptor_with_a_queued_prompt_does_not_restore(cx: &mut TestAppContext) {
    let mut f = fixture(cx);
    let saved = empty_chat(&f);
    let source = adopt_saved(&mut f, saved, cx);
    let thread = cx
        .update_window(f.handle, |_, window, cx| {
            f.panel.update(cx, |panel, cx| {
                panel.fork_thread(source.entity_id(), window, cx)
            });
            f.panel.read(cx).current().unwrap()
        })
        .unwrap();
    thread.update(cx, |thread, cx| {
        assert!(
            thread
                .pending_restore()
                .as_ref()
                .is_some_and(|restore| restore.fork)
        );
        assert!(thread.submit("waiting request".into(), None, cx).unwrap());
        let id = thread.queued_prompt_ids().next().unwrap();
        thread.send_now(id, cx);
    });
    cx.run_until_parked();
    assert!(f.commands.restores.lock().unwrap().is_empty());
    assert_eq!(f.commands.sessions.load(Ordering::SeqCst), 1);
    let prompts = f.commands.prompts.lock().unwrap();
    assert_eq!(prompts.len(), 1);
    assert!(
        prompts[0]
            .content
            .iter()
            .any(|block| matches!(block, Content::Text(text) if text.text == "waiting request"))
    );
    cx.update(|cx| assert!(!thread.read(cx).document().pending_history()));
}

#[gpui_kit::test]
fn missing_rollout_falls_back_once_and_preserves_history_and_queue(cx: &mut TestAppContext) {
    let mut f = fixture(cx);
    let (id, metadata, resume, mut transcript, mut composer, _) = empty_chat(&f).into_parts();
    transcript.entries = vec![
        Entry::User(vec![Content::Text(Text {
            text: "remember this".into(),
            annotations: None,
            extensions: None,
        })]),
        Entry::Agent("previous answer".into()),
    ];
    composer.queue.push(SavedPrompt {
        id: 1,
        text: "continue conversation".into(),
        images: Vec::new(),
        attachments: Vec::new(),
    });
    let saved = SavedChat::from_parts(
        &f.installation.retention,
        id,
        metadata,
        resume,
        transcript,
        composer,
    )
    .unwrap();
    let thread = adopt_saved(&mut f, saved, cx);
    f.commands
        .restore_errors
        .lock()
        .unwrap()
        .push_back(RestoreFailure::Unavailable(
            "no rollout found for thread id previous",
        ));
    thread.update(cx, |thread, cx| thread.send_now(1, cx));
    cx.run_until_parked();
    assert_eq!(f.commands.restores.lock().unwrap().len(), 1);
    assert_eq!(f.commands.sessions.load(Ordering::SeqCst), 1);
    let prompts = f.commands.prompts.lock().unwrap();
    assert_eq!(prompts.len(), 1);
    assert!(prompts[0].content.iter().any(|block| matches!(block, Content::Text(text) if text.text.contains("User: remember this") && text.text.contains("Assistant: previous answer"))));
    assert!(
        prompts[0].content.iter().any(
            |block| matches!(block, Content::Text(text) if text.text == "continue conversation")
        )
    );
    drop(prompts);
    cx.update(|cx| assert_eq!(thread.read(cx).history_rows().len(), 3));
    complete_active(&f, cx);
    thread.update(cx, |thread, cx| thread.send("next message".into(), cx));
    cx.run_until_parked();
    let prompts = f.commands.prompts.lock().unwrap();
    assert_eq!(prompts.len(), 2);
    assert!(!prompts[1].content.iter().any(|block| matches!(block, Content::Text(text) if text.text.contains("Saved conversation (context"))));
    assert_eq!(f.commands.restores.lock().unwrap().len(), 1);
}

#[gpui_kit::test]
fn a_restored_text_prompt_is_edited_with_its_own_context(cx: &mut TestAppContext) {
    use nocterm_ai::history::SavedAttachment;
    let mut f = fixture(cx);
    let (id, metadata, resume, transcript, mut composer, _) = empty_chat(&f).into_parts();
    composer.attachments = vec![SavedAttachment::Group("defaults".into())];
    composer.queue.push(SavedPrompt {
        id: 1,
        text: "queued text".into(),
        images: Vec::new(),
        attachments: vec![SavedAttachment::Group("prompt context".into())],
    });
    let saved = SavedChat::from_parts(
        &f.installation.retention,
        id,
        metadata,
        resume,
        transcript,
        composer,
    )
    .unwrap();
    let thread = adopt_saved(&mut f, saved, cx);
    cx.update_window(f.handle, |_, window, cx| {
        f.panel
            .update(cx, |panel, cx| panel.edit_queued(1, window, cx))
    })
    .unwrap();
    thread.update(cx, |thread, _| {
        assert!(thread.composer().editing());
        assert_eq!(
            thread.composer().attachments().to_vec(),
            [Attachment::Group("prompt context".into())]
        );
    });
    super::drafts_intersections::type_draft(&f, "edited text", cx);
    cx.update_window(f.handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("agent-send", cx);
    })
    .unwrap();
    cx.run_until_parked();
    thread.update(cx, |thread, _| {
        assert!(!thread.composer().editing());
        assert_eq!(
            thread.composer().queue()[0].text().to_string(),
            "edited text"
        );
        assert_eq!(
            thread.composer().attachments().to_vec(),
            [Attachment::Group("defaults".into())]
        );
    });
}
