//! Selected history membership changes while hidden documents keep their owners.
use super::*;
use nocterm_ai::history::{ContextId, SavedChat};

fn append(
    roster: &mut crate::roster::ChatRoster,
    thread: &Entity<crate::thread::AgentThread>,
    window: &Window,
    cx: &mut App,
) {
    let observation = cx.observe(thread, |_, _| {});
    let slot = roster.track(thread, window.window_handle(), observation, cx);
    roster.append(slot);
    roster.select(thread.entity_id(), cx);
}

#[gpui_kit::test]
fn a_project_chat_starts_the_ordinary_local_agent_after_focus_moves_to_another_project(
    cx: &mut TestAppContext,
) {
    let f = fixture(cx);
    lazy_start(&f, cx);
    let original = nocterm_project_model::ProjectId::from_bytes([1; 16]);
    cx.update_window(f.handle, |_, window, cx| {
        super::seat_binding::project_seat(&f, original, window, cx);
    })
    .unwrap();
    cx.run_until_parked();
    new_chat(&f, cx);
    let thread = cx.update(|cx| f.panel.read(cx).current().unwrap());
    let id = cx.update(|cx| thread.read(cx).chat_id().to_owned());
    cx.update_window(f.handle, |_, window, cx| {
        thread.update(cx, |thread, cx| thread.send("Original A prompt".into(), cx));
        super::seat_binding::project_seat(
            &f,
            nocterm_project_model::ProjectId::from_bytes([2; 16]),
            window,
            cx,
        );
    })
    .unwrap();
    cx.run_until_parked();
    assert_eq!(f.connector.connects.load(Ordering::SeqCst), 1);
    assert_eq!(f.commands.prompts.lock().unwrap().len(), 1);
    cx.update(|cx| {
        let original_scope = ContextId::Project(original);
        let chat = thread.read(cx);
        assert_eq!(chat.document().key().scope(), original_scope);
        assert!(chat.session().is_some());
        assert_eq!(
            f.commands.prompts.lock().unwrap()[0].session,
            *chat.session().as_ref().unwrap()
        );
        assert!(f.panel.read(cx).current().is_none());
        assert_eq!(f.panel.read(cx).chat_documents().count(), 0);
        assert_eq!(
            f.repository
                .transcript_in_scope(original_scope, &id)
                .unwrap()
                .header
                .scope,
            original_scope
        );
        assert!(matches!(
            f.repository.transcript(&id),
            Err(nocterm_ai::storage::AgentStorageError::NotFound(_))
        ));
    });
}
#[gpui_kit::test]
fn switching_contexts_filters_chat_membership_and_retains_original_document_and_draft(
    cx: &mut TestAppContext,
) {
    let f = fixture(cx);
    cx.update_window(f.handle, |_, window, cx| {
        let general = f._directory.deps(
            f.workspace.downgrade(),
            f.installation.installed.clone(),
            cx,
        );
        let mut roster = crate::roster::ChatRoster::new(general.clone());
        let chat = SavedChat::for_identity(
            "codex".into(),
            "00000000-0000-0000-0000-000000000001".into(),
            1,
        );
        let first = general.restore_chat(chat.clone(), cx).unwrap();
        first.update(cx, |thread, cx| {
            thread.set_draft("General literal draft".into(), cx)
        });
        append(&mut roster, &first, window, cx);
        let project = nocterm_project_model::ProjectId::from_bytes([1; 16]);
        let (seat, _surface, _outer) = super::seat_binding::project_seat(&f, project, window, cx);
        let scoped = crate::chat_deps::ChatDeps::new(
            general.runtime().for_scope(ContextId::Project(project)),
            general.agents().clone(),
            general.settings().clone(),
            f._directory.metadata.clone(),
            seat,
            None,
        );
        roster.switch_context(scoped.clone(), cx);
        assert!(roster.is_empty());
        assert!(roster.current().is_none());
        assert!(roster.contains_any(first.entity_id()));
        let mut chat = chat;
        chat.header.scope = scoped.scope();
        let second = scoped.restore_chat(chat, cx).unwrap();
        append(&mut roster, &second, window, cx);
        assert_eq!(
            roster
                .iter()
                .map(|thread| thread.entity_id())
                .collect::<Vec<_>>(),
            [second.entity_id()]
        );
        assert_eq!(roster.host().scope(), scoped.scope());
        roster.switch_context(general, cx);
        assert_eq!(roster.current().unwrap().entity_id(), first.entity_id());
        assert_eq!(
            roster
                .iter()
                .map(|thread| thread.entity_id())
                .collect::<Vec<_>>(),
            [first.entity_id()]
        );
        assert_eq!(first.read(cx).draft_text(), Some("General literal draft"));
        assert!(roster.contains_any(second.entity_id()));
        roster.switch_context(scoped, cx);
        assert_eq!(roster.current().unwrap().entity_id(), second.entity_id());
    })
    .unwrap();
}

#[gpui_kit::test]
fn leaving_a_quiet_archived_context_releases_its_entity_and_original_catalog_reference(
    cx: &mut TestAppContext,
) {
    let f = fixture(cx);
    let project = nocterm_project_model::ProjectId::from_bytes([1; 16]);
    let mut chat = SavedChat::for_identity(
        "codex".into(),
        "00000000-0000-0000-0000-000000000001".into(),
        1,
    );
    chat.header.scope = ContextId::Project(project);
    chat.entries.push(nocterm_ai::thread::Entry::Agent(
        "archived literal answer".into(),
    ));
    f.repository.seed_chat(&chat).unwrap();
    let runtime = f
        .installation
        .installed
        .runtime()
        .for_scope(chat.header.scope);
    cx.update(|cx| runtime.reload_history(cx));
    cx.run_until_parked();
    let (weak, _roster) = cx
        .update_window(f.handle, |_, window, cx| {
            let general = f._directory.deps(
                f.workspace.downgrade(),
                f.installation.installed.clone(),
                cx,
            );
            let (seat, _surface, _outer) =
                super::seat_binding::pending_project_seat(&f, project, window, cx);
            let scoped = crate::chat_deps::ChatDeps::new(
                runtime.clone(),
                general.agents().clone(),
                general.settings().clone(),
                f._directory.metadata.clone(),
                seat,
                None,
            );
            let mut roster = crate::roster::ChatRoster::new(scoped);
            let archived = roster.saved(cx).pop().unwrap();
            let weak = archived.downgrade();
            let observation = cx.observe(&archived, |_, _| {});
            let slot = roster.track(&archived, window.window_handle(), observation, cx);
            roster.append(slot);
            drop(archived);
            roster.switch_context(general, cx);
            assert!(!roster.contains_any(weak.entity_id()));
            (weak, roster)
        })
        .unwrap();
    cx.run_until_parked();
    assert!(weak.upgrade().is_none());
}

#[gpui_kit::test]
fn outer_workspace_selection_filters_panel_history_and_restores_general_literal_input(
    cx: &mut TestAppContext,
) {
    let f = fixture(cx);
    lazy_start(&f, cx);
    new_chat(&f, cx);
    set_input(&f, "General literal input", cx);
    let original = cx.update(|cx| f.panel.read(cx).current().unwrap());
    let (_seat, _surface, outer) = cx
        .update_window(f.handle, |_, window, cx| {
            super::seat_binding::project_seat(
                &f,
                nocterm_project_model::ProjectId::from_bytes([1; 16]),
                window,
                cx,
            )
        })
        .unwrap();
    cx.run_until_parked();
    cx.update(|cx| {
        assert!(f.panel.read(cx).current().is_none());
        assert_eq!(f.panel.read(cx).chat_documents().count(), 0);
        assert_eq!(f.panel.read(cx).composer_text(cx).as_ref(), "");
    });
    new_chat(&f, cx);
    let project = cx.update(|cx| f.panel.read(cx).current().unwrap());
    cx.update(|cx| {
        assert_eq!(
            project.read(cx).document().key().scope(),
            ContextId::Project(nocterm_project_model::ProjectId::from_bytes([1; 16]))
        );
        assert_eq!(f.panel.read(cx).chat_documents().count(), 1);
    });
    cx.update_window(f.handle, |_, window, cx| {
        f.workspace.update(cx, |workspace, cx| {
            workspace.activate_item_by_id(f.terminal, window, cx);
        })
    })
    .unwrap();
    cx.run_until_parked();
    cx.update(|cx| {
        assert_eq!(
            f.panel.read(cx).current().unwrap().entity_id(),
            original.entity_id()
        );
        assert_eq!(
            f.panel.read(cx).composer_text(cx).as_ref(),
            "General literal input"
        );
        assert_eq!(f.panel.read(cx).chat_documents().count(), 1);
    });
    cx.update_window(f.handle, |_, window, cx| {
        f.workspace.update(cx, |workspace, cx| {
            workspace.activate_item_by_id(outer.entity_id(), window, cx);
        })
    })
    .unwrap();
    cx.run_until_parked();
    cx.update(|cx| {
        assert_eq!(
            f.panel.read(cx).current().unwrap().entity_id(),
            project.entity_id()
        )
    });
}

#[gpui_kit::test]
fn accepted_archive_fork_finishes_in_its_hidden_original_context_without_changing_the_visible_chat(
    cx: &mut TestAppContext,
) {
    let f = fixture(cx);
    lazy_start(&f, cx);
    let project = nocterm_project_model::ProjectId::from_bytes([1; 16]);
    let mut saved = SavedChat::for_scope(
        "codex".into(),
        "00000000-0000-0000-0000-000000000001".into(),
        1,
        ContextId::Project(project),
    );
    saved
        .entries
        .push(nocterm_ai::thread::Entry::Agent("original A answer".into()));
    f.repository.seed_chat(&saved).unwrap();
    let (_seat, _surface, outer) = cx
        .update_window(f.handle, |_, window, cx| {
            super::seat_binding::project_seat(&f, project, window, cx)
        })
        .unwrap();
    cx.run_until_parked();
    let source = cx.update(|cx| {
        f.panel
            .read(cx)
            .chat_documents()
            .next()
            .unwrap()
            .entity_id()
    });
    let release = f
        .repository
        .block_next(nocterm_ai::testing::StorageOperation::Load(saved.header.id));
    cx.update_window(f.handle, |_, window, cx| {
        f.panel
            .update(cx, |panel, cx| panel.fork_thread(source, window, cx))
    })
    .unwrap();
    cx.run_until_parked();
    let (_seat_b, _surface_b, _outer_b) = cx
        .update_window(f.handle, |_, window, cx| {
            super::seat_binding::project_seat(
                &f,
                nocterm_project_model::ProjectId::from_bytes([2; 16]),
                window,
                cx,
            )
        })
        .unwrap();
    cx.run_until_parked();
    new_chat(&f, cx);
    set_input(&f, "B literal draft", cx);
    let visible = cx.update(|cx| f.panel.read(cx).current().unwrap().entity_id());
    release.send(Ok(())).unwrap();
    cx.run_until_parked();
    cx.update(|cx| {
        assert_eq!(f.panel.read(cx).current().unwrap().entity_id(), visible);
        assert_eq!(
            f.panel.read(cx).composer_text(cx).as_ref(),
            "B literal draft"
        );
        assert_eq!(f.panel.read(cx).chat_documents().count(), 1);
    });
    cx.update_window(f.handle, |_, window, cx| {
        f.workspace.update(cx, |workspace, cx| {
            workspace.activate_item_by_id(outer.entity_id(), window, cx);
        })
    })
    .unwrap();
    cx.run_until_parked();
    cx.update(|cx| {
        assert_eq!(f.panel.read(cx).chat_documents().count(), 2);
        let fork = f.panel.read(cx).current().unwrap();
        assert_ne!(fork.entity_id(), source);
        assert_eq!(
            fork.read(cx).document().key().scope(),
            ContextId::Project(project)
        );
        assert_eq!(
            serde_json::to_value(fork.read(cx).history_entries()).unwrap(),
            serde_json::to_value(&saved.entries).unwrap()
        );
    });
}

#[gpui_kit::test]
async fn disabling_ai_releases_hidden_context_membership_and_preserves_every_scopes_draft(
    cx: &mut TestAppContext,
) {
    let f = fixture(cx);
    lazy_start(&f, cx);
    new_chat(&f, cx);
    set_input(&f, "General retained draft", cx);
    let general = cx.update(|cx| f.panel.read(cx).current().unwrap());
    let general_id = cx.update(|cx| general.read(cx).chat_id().to_owned());
    let project = nocterm_project_model::ProjectId::from_bytes([1; 16]);
    let (_seat, _surface, _outer) = cx
        .update_window(f.handle, |_, window, cx| {
            super::seat_binding::project_seat(&f, project, window, cx)
        })
        .unwrap();
    cx.run_until_parked();
    new_chat(&f, cx);
    set_input(&f, "Project retained draft", cx);
    let scoped = cx.update(|cx| f.panel.read(cx).current().unwrap());
    let project_id = cx.update(|cx| scoped.read(cx).chat_id().to_owned());
    cx.update(|cx| {
        f.installation
            .ready
            .settings()
            .update::<nocterm_ai::AiSettings>(cx, |settings| settings.enabled = false)
    })
    .await
    .unwrap();
    cx.run_until_parked();
    assert_eq!(
        f.repository
            .transcript(&general_id)
            .unwrap()
            .draft
            .as_deref(),
        Some("General retained draft")
    );
    assert_eq!(
        f.repository
            .transcript_in_scope(ContextId::Project(project), &project_id)
            .unwrap()
            .draft
            .as_deref(),
        Some("Project retained draft")
    );
    // Reenable and return to General: a fresh discovery must claim a fresh entity,
    // rather than resurrecting the hidden retained roster after global disable.
    cx.update(|cx| {
        f.installation
            .ready
            .settings()
            .update::<nocterm_ai::AiSettings>(cx, |settings| settings.enabled = true)
    })
    .await
    .unwrap();
    cx.update_window(f.handle, |_, window, cx| {
        f.workspace.update(cx, |workspace, cx| {
            workspace.activate_item_by_id(f.terminal, window, cx);
        })
    })
    .unwrap();
    cx.run_until_parked();
    cx.update(|cx| {
        assert!(
            f.panel
                .read(cx)
                .chat_documents()
                .all(|thread| thread.entity_id() != general.entity_id())
        );
        assert!(
            f.panel
                .read(cx)
                .chat_documents()
                .any(|thread| thread.read(cx).chat_id() == general_id)
        );
    });
}

#[gpui_kit::test]
fn forked_transcripts_contribute_their_payload_to_installation_residency(cx: &mut TestAppContext) {
    let f = fixture(cx);
    lazy_start(&f, cx);
    cx.update_window(f.handle, |_, window, cx| {
        let general = f._directory.deps(
            f.workspace.downgrade(),
            f.installation.installed.clone(),
            cx,
        );
        let mut roster = crate::roster::ChatRoster::new(general.clone());
        let mut saved = SavedChat::for_identity(
            "codex".into(),
            "00000000-0000-0000-0000-000000000001".into(),
            1,
        );
        saved
            .entries
            .push(nocterm_ai::thread::Entry::Agent("a".repeat(1024 * 1024)));
        let source = general.restore_chat(saved, cx).unwrap();
        append(&mut roster, &source, window, cx);
        let fork = roster.fork(&source, None, cx).unwrap();
        assert!(fork.read(cx).resident_history_bytes() >= 2 * 1024 * 1024);
        assert_eq!(fork.read(cx).history_entries().len(), 1);
    })
    .unwrap();
}
