use super::*;
use crate::history_fixtures::{edited, image, text};
use nocterm_ai::{
    history::{ArchivedDraftPatch, ChatWrite, MAX_FILE_BYTES, SavedAttachment, SavedPrompt},
    model::content::Content,
    thread::Entry,
};
const MAX_SAVED_IMAGE_BYTES: usize = 512 * 1024;
const MAX_SAVED_ENTRIES: usize = 2000;
fn chat(authority: &nocterm_ai::model::retention::RetentionAuthority) -> SavedChat {
    edited(
        SavedChat::for_identity(
            authority,
            "codex".into(),
            uuid::Uuid::new_v4().to_string(),
            1,
        ),
        |metadata, resume, transcript, _| {
            metadata.title = Some("Disk usage".into());
            resume.session_id = Some("session-1".into());
            transcript.entries = vec![
                Entry::User(vec![text("df -h?")]),
                Entry::Thought("checking".into()),
                Entry::Agent("Use `df -h`.".into()),
            ];
        },
    )
}
#[test]
fn a_chat_keeps_its_configuration_on_disk() {
    let authority = nocterm_ai::model::retention::RetentionAuthority::new(
        nocterm_ai::history::MAX_RESIDENT_BYTES,
    )
    .unwrap();
    let dir = tempfile::tempdir().unwrap();
    let saved = edited(chat(&authority), |_, resume, _, _| {
        resume.config.insert(
            "model".into(),
            nocterm_ai::session_config::ConfigValue::Value("opus".into()),
        );
        resume.mode = Some("plan".into());
    });
    save(dir.path(), &saved).unwrap();
    let loaded = load(dir.path(), saved.id(), &authority).unwrap();
    assert_eq!(loaded.resume().config, saved.resume().config);
    assert_eq!(loaded.resume().mode.as_deref(), Some("plan"));
}
#[test]
fn chats_round_trip_newest_first() {
    let authority = nocterm_ai::model::retention::RetentionAuthority::new(
        nocterm_ai::history::MAX_RESIDENT_BYTES,
    )
    .unwrap();
    let dir = tempfile::tempdir().unwrap();
    let older = edited(chat(&authority), |metadata, _, _, _| metadata.updated = 10);
    let newer = edited(chat(&authority), |metadata, _, _, _| metadata.updated = 20);
    save(dir.path(), &older).unwrap();
    save(dir.path(), &newer).unwrap();
    let loaded = load_all(dir.path(), &authority);
    assert_eq!(
        loaded.iter().map(SavedChat::id).collect::<Vec<_>>(),
        [newer.id(), older.id()]
    );
    assert_eq!(loaded[0].transcript().entries.len(), 3);
    assert!(
        matches!(&loaded[0].transcript().entries[2],Entry::Agent(text) if text=="Use `df -h`.")
    );
    assert_eq!(loaded[0].resume().session_id.as_deref(), Some("session-1"));
}
#[test]
fn pinned_chats_come_first_with_their_names() {
    let authority = nocterm_ai::model::retention::RetentionAuthority::new(
        nocterm_ai::history::MAX_RESIDENT_BYTES,
    )
    .unwrap();
    let dir = tempfile::tempdir().unwrap();
    let pinned = edited(chat(&authority), |metadata, _, _, _| {
        metadata.updated = 1;
        metadata.pinned = true;
        metadata.name = Some("Disks".into());
    });
    let newer = edited(chat(&authority), |metadata, _, _, _| metadata.updated = 2);
    save(dir.path(), &pinned).unwrap();
    save(dir.path(), &newer).unwrap();
    let loaded = load_all(dir.path(), &authority);
    assert_eq!(loaded[0].id(), pinned.id());
    assert!(loaded[0].metadata().pinned);
    assert_eq!(loaded[0].metadata().name.as_deref(), Some("Disks"));
    assert!(!loaded[1].metadata().pinned);
}
#[cfg(unix)]
#[test]
fn chats_are_private_to_the_user() {
    let authority = nocterm_ai::model::retention::RetentionAuthority::new(
        nocterm_ai::history::MAX_RESIDENT_BYTES,
    )
    .unwrap();
    use std::os::unix::fs::PermissionsExt as _;
    let root = tempfile::tempdir().unwrap();
    let dir = root.path().join("chats");
    let chat = chat(&authority);
    save(&dir, &chat).unwrap();
    let mode = |path: &Path| std::fs::metadata(path).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode(&dir), 0o700);
    assert_eq!(mode(&dir.join(format!("{}.json", chat.id()))), 0o600);
}
#[test]
fn deleting_removes_the_file_and_ids_cannot_walk_paths() {
    let authority = nocterm_ai::model::retention::RetentionAuthority::new(
        nocterm_ai::history::MAX_RESIDENT_BYTES,
    )
    .unwrap();
    let dir = tempfile::tempdir().unwrap();
    let chat = chat(&authority);
    save(dir.path(), &chat).unwrap();
    delete(dir.path(), chat.id()).unwrap();
    delete(dir.path(), chat.id()).unwrap();
    assert!(load_all(dir.path(), &authority).is_empty());
    let mut invalid = serde_json::to_value(&chat).unwrap();
    invalid["id"] = "../escape".into();
    let evil: SavedChat = crate::history_fixtures::decoded_value(&authority, invalid).unwrap();
    assert!(save(dir.path(), &evil).is_err());
    assert!(delete(dir.path(), "../escape").is_err());
}
#[test]
fn large_images_and_old_entries_are_left_out() {
    let authority = nocterm_ai::model::retention::RetentionAuthority::new(
        nocterm_ai::history::MAX_RESIDENT_BYTES,
    )
    .unwrap();
    let dir = tempfile::tempdir().unwrap();
    let chat = edited(chat(&authority), |_, _, transcript, _| {
        transcript.entries = (0..MAX_SAVED_ENTRIES + 5)
            .map(|ix| Entry::Agent(ix.to_string()))
            .collect();
        transcript.entries.push(Entry::User(vec![image(
            "A".repeat(MAX_SAVED_IMAGE_BYTES + 1),
            "image/png",
        )]));
    });
    save(dir.path(), &chat).unwrap();
    let loaded = load_all(dir.path(), &authority).remove(0);
    assert_eq!(loaded.transcript().entries.len(), MAX_SAVED_ENTRIES);
    assert!(
        matches!(loaded.transcript().entries.last(),Some(Entry::User(blocks)) if matches!(&blocks[0],Content::Text(text) if text.text.contains("not kept")))
    );
}
#[test]
fn foreign_and_broken_files_are_skipped() {
    let authority = nocterm_ai::model::retention::RetentionAuthority::new(
        nocterm_ai::history::MAX_RESIDENT_BYTES,
    )
    .unwrap();
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("broken.json"), "{").unwrap();
    let chat = chat(&authority);
    let bytes = crate::legacy::fixture_bytes(&chat);
    std::fs::write(
        dir.path().join(format!("{}.json", uuid::Uuid::new_v4())),
        bytes,
    )
    .unwrap();
    assert!(load_all(dir.path(), &authority).is_empty());
}
#[test]
fn pending_images_and_attachment_snapshots_round_trip_without_truncation() {
    let authority = nocterm_ai::model::retention::RetentionAuthority::new(
        nocterm_ai::history::MAX_RESIDENT_BYTES,
    )
    .unwrap();
    let dir = tempfile::tempdir().unwrap();
    let data = "A".repeat(MAX_SAVED_IMAGE_BYTES + 1);
    let chat = edited(chat(&authority), |_, _, _, composer| {
        composer.attachments = vec![
            SavedAttachment::Connection("server".into()),
            SavedAttachment::Group("production".into()),
        ];
        composer.queue = vec![SavedPrompt {
            id: 42,
            text: "queued image".into(),
            images: vec![image(data.clone(), "image/png")],
            attachments: composer.attachments.clone(),
        }];
    });
    save(dir.path(), &chat).unwrap();
    let saved = load_all(dir.path(), &authority).remove(0);
    assert_eq!(saved.composer().queue[0].text, "queued image");
    assert_eq!(saved.composer().attachments, chat.composer().attachments);
    assert_eq!(
        saved.composer().queue[0].attachments,
        chat.composer().attachments
    );
    assert!(
        matches!(&saved.composer().queue[0].images[0],Content::Media(image) if image.encoded==data)
    );
}
#[test]
fn oversized_pending_messages_are_rejected_and_old_chats_still_load() {
    let authority = nocterm_ai::model::retention::RetentionAuthority::new(
        nocterm_ai::history::MAX_RESIDENT_BYTES,
    )
    .unwrap();
    let dir = tempfile::tempdir().unwrap();
    let chat = chat(&authority);
    let mut old: serde_json::Value =
        serde_json::from_slice(&crate::legacy::fixture_bytes(&chat)).unwrap();
    old.as_object_mut().unwrap().remove("queue");
    old.as_object_mut().unwrap().remove("attachments");
    std::fs::write(
        dir.path().join(format!("{}.json", chat.id())),
        serde_json::to_vec(&old).unwrap(),
    )
    .unwrap();
    let old = load(dir.path(), chat.id(), &authority).unwrap();
    assert!(old.composer().queue.is_empty());
    assert!(old.composer().attachments.is_empty());
    let chat = edited(chat, |_, _, _, composer| {
        composer.queue = vec![SavedPrompt {
            id: 1,
            text: "x".repeat(MAX_FILE_BYTES as usize),
            images: Vec::new(),
            attachments: Vec::new(),
        }]
    });
    assert!(history::prepare(chat).is_err());
}
#[test]
fn archived_draft_publication_merges_only_draft_and_updated_without_hydration() {
    let authority = nocterm_ai::model::retention::RetentionAuthority::new(
        nocterm_ai::history::MAX_RESIDENT_BYTES,
    )
    .unwrap();
    let directory = tempfile::tempdir().unwrap();
    let original = edited(chat(&authority), |metadata, _, _, composer| {
        metadata.name = Some("keep the name".into());
        composer.queue.push(SavedPrompt {
            id: 9,
            text: "keep queued text".into(),
            images: Vec::new(),
            attachments: Vec::new(),
        });
    });
    save(directory.path(), &original).unwrap();
    let draft = "literal quote\"\n界";
    let patch = ChatWrite::ArchiveDraft(ArchivedDraftPatch {
        id: original.id().to_owned(),
        agent_id: "other-agent".into(),
        text: Some(draft.into()),
        updated: original.metadata().updated + 1,
    });
    save_write(directory.path(), &patch, &authority).unwrap();
    let loaded = load(directory.path(), original.id(), &authority).unwrap();
    assert_eq!(
        serde_json::to_value(&loaded.transcript().entries).unwrap(),
        serde_json::to_value(&original.transcript().entries).unwrap()
    );
    assert_eq!(
        serde_json::to_value(&loaded.composer().queue).unwrap(),
        serde_json::to_value(&original.composer().queue).unwrap()
    );
    assert_eq!(loaded.metadata().name, original.metadata().name);
    assert_eq!(loaded.metadata().agent_id, original.metadata().agent_id);
    assert_eq!(loaded.composer().draft.as_deref(), Some(draft));
    assert_eq!(loaded.metadata().updated, original.metadata().updated + 1);
}
#[test]
fn best_effort_cache_failure_does_not_revoke_a_published_transcript() {
    let authority = nocterm_ai::model::retention::RetentionAuthority::new(
        nocterm_ai::history::MAX_RESIDENT_BYTES,
    )
    .unwrap();
    let directory = tempfile::tempdir().unwrap();
    let chat = chat(&authority);
    std::fs::create_dir(directory.path().join(format!("{}.meta", chat.id()))).unwrap();
    save(directory.path(), &chat).unwrap();
    assert_eq!(
        serde_json::to_value(
            &load(directory.path(), chat.id(), &authority)
                .unwrap()
                .transcript()
                .entries
        )
        .unwrap(),
        serde_json::to_value(&chat.transcript().entries).unwrap()
    );
}

#[test]
fn public_repository_keeps_untouched_legacy_bytes_and_changes_format_after_real_draft_edit() {
    let authority = nocterm_ai::model::retention::RetentionAuthority::new(
        nocterm_ai::history::MAX_RESIDENT_BYTES,
    )
    .unwrap();
    use futures::executor::block_on;
    use nocterm_ai::{
        history::{ArchivedDraftPatch, ChatWrite, HistoryOrigin},
        storage::ChatRepository,
    };
    let dir = tempfile::tempdir().unwrap();
    let id = "f2f8f6ac-eae5-4dd7-b4bc-3713da3ed73c";
    // The original codec independently limits timestamps to2000; their length
    // need not match the one transcript record, and values stay literal.
    let times = (0..2000).map(|index| 4 + index * 2).collect::<Vec<u64>>();
    let encoded_times = serde_json::to_string(&times).unwrap();
    let original = format!(
        r#"{{
      "updated":17, "id":"{id}", "agent_id":"provider", "version":1,
      "draft":"literal unsent", "session_id":"original-session",
      "entries":[{{"kind":"agent","value":"historical answer"}}],
      "attachments":[{{"kind":"group","id":"old group"}}],
      "times":{encoded_times}
    }}"#
    )
    .into_bytes();
    let path = dir.path().join(format!("{id}.json"));
    std::fs::write(&path, &original).unwrap();
    let repository = FileChatRepository::with_retention(dir.path().to_owned(), authority.clone());
    let loaded = block_on(repository.load(id.into())).unwrap();
    assert_eq!(loaded.origin(), HistoryOrigin::LegacyV1);
    assert_eq!(loaded.transcript().times, times);
    assert_eq!(
        loaded.resume().session_id.as_deref(),
        Some("original-session")
    );
    assert_eq!(std::fs::read(&path).unwrap(), original);
    block_on(repository.catalog()).unwrap();
    assert_eq!(std::fs::read(&path).unwrap(), original);
    let unchanged = ChatWrite::ArchiveDraft(ArchivedDraftPatch {
        id: id.into(),
        agent_id: "provider".into(),
        text: Some("literal unsent".into()),
        updated: 99,
    });
    block_on(repository.save(Arc::new(unchanged))).unwrap();
    assert_eq!(std::fs::read(&path).unwrap(), original);
    let edited = ChatWrite::ArchiveDraft(ArchivedDraftPatch {
        id: id.into(),
        agent_id: "provider".into(),
        text: Some("changed draft".into()),
        updated: 100,
    });
    block_on(repository.save(Arc::new(edited))).unwrap();
    let physical: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    assert_eq!(physical["format"], "nocterm.chat");
    assert_eq!(physical["version"], 2);
    let restarted = block_on(repository.load(id.into())).unwrap();
    assert_eq!(restarted.origin(), HistoryOrigin::Version2);
    assert_eq!(restarted.transcript().times, times);
    assert_eq!(restarted.composer().draft.as_deref(), Some("changed draft"));
    assert_eq!(
        restarted.resume().session_id.as_deref(),
        Some("original-session")
    );
    assert_eq!(
        restarted.composer().attachments,
        vec![SavedAttachment::Group("old group".into())]
    );
    assert!(
        matches!(&restarted.transcript().entries[0], Entry::Agent(text) if text == "historical answer")
    );
    // Actual filesystem API coverage. Runtime clean capture/restart admission
    // and live queued dispatch require separate service/feature wiring tests.
}

#[test]
fn public_codec_keeps_supplied_tool_null_in_v2_and_refuses_lossy_legacy_fallback() {
    let authority = nocterm_ai::model::retention::RetentionAuthority::new(
        nocterm_ai::history::MAX_RESIDENT_BYTES,
    )
    .unwrap();
    use futures::executor::block_on;
    use nocterm_ai::history::{ChatWrite, HistoryBudget, HistoryCodecError, SharedChat};
    for near_file_limit in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        let id = "f2f8f6ac-eae5-4dd7-b4bc-3713da3ed73c";
        let mut fixture = serde_json::json!({
            "version":1,"id":id,"agent_id":"provider","updated":1,
            "entries":[{"kind":"tool","value":{"toolCallId":"original-id","title":"literal","rawOutput":""}}]
        });
        if near_file_limit {
            let framing = serde_json::to_vec(&fixture).unwrap().len();
            // Escapes make the real encoded bound tight while genuine retained
            // string bytes remain comfortably below the independent64MiB gate.
            let controls = (MAX_FILE_BYTES as usize - 256 - framing) / 6;
            fixture["entries"][0]["value"]["rawOutput"] =
                serde_json::Value::String("\0".repeat(controls));
        }
        let bytes = serde_json::to_vec(&fixture).unwrap();
        assert!(bytes.len() <= MAX_FILE_BYTES as usize);
        let path = dir.path().join(format!("{id}.json"));
        std::fs::write(&path, &bytes).unwrap();
        let repository =
            FileChatRepository::with_retention(dir.path().to_owned(), authority.clone());
        let imported = block_on(repository.load(id.into())).unwrap();
        let (id, metadata, resume, mut transcript, composer, prepared) = imported.into_parts();
        let budget = HistoryBudget::restored(&authority, prepared.as_ref());
        let Entry::Tool(tool) = &mut transcript.entries[0] else {
            panic!("tool row")
        };
        tool.arguments = Some(
            nocterm_ai::model::json::OwnedJson::from_untracked(&authority, serde_json::Value::Null)
                .unwrap(),
        );
        let changed = SavedChat::from_parts(
            &authority,
            id.clone(),
            metadata,
            resume,
            transcript,
            composer,
        )
        .unwrap();
        let result = budget.prepare_snapshot(changed);
        if near_file_limit {
            assert!(matches!(
                result,
                Err(HistoryCodecError::Compatibility(
                    nocterm_ai::history::CompatibilityReason::ToolField
                ))
            ));
            assert_eq!(std::fs::read(&path).unwrap(), bytes);
        } else {
            let shared = SharedChat::from_saved(result.unwrap()).unwrap();
            block_on(repository.save(Arc::new(ChatWrite::Snapshot(Box::new(shared))))).unwrap();
            let restarted = block_on(repository.load(id)).unwrap();
            let Entry::Tool(tool) = &restarted.transcript().entries[0] else {
                panic!("tool row")
            };
            assert_eq!(
                tool.arguments.as_ref().map(|value| value.as_value()),
                Some(&serde_json::Value::Null)
            );
        }
    }
    // Actual public format costs decide fallback eligibility; no fabricated
    // over-limit scalar or private model/view field bypass is used here.
}

#[test]
fn public_repository_streams_exact_selected_version_two_projection() {
    let authority = nocterm_ai::model::retention::RetentionAuthority::new(
        nocterm_ai::history::MAX_RESIDENT_BYTES,
    )
    .unwrap();
    use futures::executor::block_on;
    use nocterm_ai::history::{self, ChatWrite, SharedChat};
    let directory = tempfile::tempdir().unwrap();
    let original = SavedChat::for_identity(
        &authority,
        "provider".into(),
        "becf172b-a7ef-4207-9f3f-e17c13e4ae29".into(),
        3,
    );
    let (id, metadata, resume, transcript, mut composer, _) = original.into_parts();
    composer.draft = Some("literal quote\"\n界\\tail".into());
    let chat = SavedChat::from_parts(
        &authority,
        id.clone(),
        metadata,
        resume,
        transcript,
        composer,
    )
    .unwrap();
    let shared = SharedChat::from_saved(chat).unwrap();
    let expected = history::encode_shared(&shared).unwrap();
    let decision = shared.decision().unwrap();
    let repository =
        FileChatRepository::with_retention(directory.path().to_owned(), authority.clone());
    block_on(repository.save(Arc::new(ChatWrite::Snapshot(Box::new(shared))))).unwrap();
    let physical = std::fs::read(directory.path().join(format!("{id}.json"))).unwrap();
    assert_eq!(physical, expected);
    assert_eq!(physical.len(), decision.size.encoded_bytes);
    assert_eq!(
        history::structural_bytes(&physical).unwrap(),
        decision.size.structural_bytes
    );
    let restarted = block_on(repository.load(id)).unwrap();
    assert_eq!(
        restarted.composer().draft.as_deref(),
        Some("literal quote\"\n界\\tail")
    );
}

#[test]
fn public_repository_restores_literal_tool_descriptions_without_execution_authority() {
    let authority = nocterm_ai::model::retention::RetentionAuthority::new(
        nocterm_ai::history::MAX_RESIDENT_BYTES,
    )
    .unwrap();
    use futures::executor::block_on;
    use nocterm_ai::{
        history::{ArchivedDraftPatch, ChatWrite},
        tool_display,
    };
    for (title, arguments, rejected_unknown) in [
        (
            "mcp.nocterm-3.typo",
            serde_json::json!({"wrong":"original"}),
            true,
        ),
        (
            "mcp_nocterm_exec_command",
            serde_json::json!({"terminal_id":"t1","program":"sh"}),
            false,
        ),
    ] {
        let directory = tempfile::tempdir().unwrap();
        let id = "7c196d12-c96a-4e71-8568-f41b473af224";
        let original = serde_json::to_vec(&serde_json::json!({
            "version":1,"id":id,"agent_id":"provider","updated":7,
            "entries":[{"kind":"tool","value":{"toolCallId":" 001 ","title":title,"rawInput":arguments}}]
        })).unwrap();
        let path = directory.path().join(format!("{id}.json"));
        std::fs::write(&path, &original).unwrap();
        let repository =
            FileChatRepository::with_retention(directory.path().to_owned(), authority.clone());
        let loaded = block_on(repository.load(id.into())).unwrap();
        let assert_row = |chat: &SavedChat| {
            let Entry::Tool(call) = &chat.transcript().entries[0] else {
                panic!("tool row")
            };
            assert_eq!(call.id.as_str(), " 001 ");
            assert_eq!(call.title, title);
            assert_eq!(
                call.arguments.as_ref().map(|value| value.as_value()),
                Some(&arguments)
            );
            assert!(call.claims.consistent_identity().is_none());
            assert!(tool_display::envelope(call, "nocterm-3").is_none());
            if rejected_unknown {
                assert_eq!(
                    tool_display::raw_envelope(call, "nocterm-3"),
                    Some(("typo".into(), arguments.clone()))
                );
            } else {
                assert!(tool_display::raw_envelope(call, "nocterm-3").is_none());
                assert!(tool_display::requested_call(call).is_some());
                assert!(tool_display::header(call).starts_with("Nocterm · Execute program"));
            }
        };
        assert_row(&loaded);
        assert_eq!(std::fs::read(&path).unwrap(), original);
        block_on(
            repository.save(Arc::new(ChatWrite::ArchiveDraft(ArchivedDraftPatch {
                id: id.into(),
                agent_id: "provider".into(),
                text: Some("real draft edit".into()),
                updated: 8,
            }))),
        )
        .unwrap();
        let physical: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        assert_eq!(physical["version"], 2);
        assert!(
            physical["document"]["transcript"]["entries"][0]["value"]
                .get("claims")
                .is_none()
        );
        let restarted = block_on(repository.load(id.into())).unwrap();
        assert_row(&restarted);
    }
}

/// The original SDK's accepted embedded resource as the application content row.
fn original_resource_content(
    authority: &nocterm_ai::model::retention::RetentionAuthority,
    block: &str,
) -> Result<nocterm_ai::model::content::Content, serde_json::Error> {
    use agent_client_protocol_schema::v1 as original;
    serde_json::from_str::<original::ContentBlock>(block).map(|block| {
        use nocterm_ai::model::content::{
            Content, EmbeddedResource, ResourceBlob, ResourceBody, ResourceText,
        };
        use nocterm_ai::model::extensions::Extensions;
        let original::ContentBlock::Resource(resource) = block else {
            unreachable!()
        };
        let body = match resource.resource {
            original::EmbeddedResourceResource::TextResourceContents(text) => {
                ResourceBody::Text(ResourceText {
                    uri: text.uri,
                    text: text.text,
                    mime: text.mime_type,
                    extensions: text
                        .meta
                        .map(|meta| Extensions::checked(authority, meta))
                        .transpose()
                        .unwrap(),
                })
            }
            original::EmbeddedResourceResource::BlobResourceContents(blob) => {
                ResourceBody::Encoded(ResourceBlob {
                    uri: blob.uri,
                    encoded: blob.blob,
                    mime: blob.mime_type,
                    extensions: blob
                        .meta
                        .map(|meta| Extensions::checked(authority, meta))
                        .transpose()
                        .unwrap(),
                })
            }
            _ => unreachable!(),
        };
        Content::EmbeddedResource(EmbeddedResource {
            body,
            annotations: None,
            extensions: None,
        })
    })
}
/// The same resource as tool output follows the original lenient tool grammar.
fn assert_tool_output_keeps_original_grammar(
    repository: &FileChatRepository,
    directory: &std::path::Path,
    block: &str,
    resource: &str,
    expected_tool_content: Option<nocterm_ai::model::content::Content>,
) {
    use agent_client_protocol_schema::v1 as original;
    use futures::executor::block_on;
    let call = format!(
        r#"{{"toolCallId":"original","title":"resource output","content":[{{"type":"content","content":{block}}},{{"type":"unrecognized"}}]}}"#
    );
    let expected_call = serde_json::from_str::<original::ToolCall>(&call);
    let tool_id = "3a9ef0d2-3c03-405b-823b-7b302ce12fc8";
    let bytes = format!(r#"{{"version":1,"id":"{tool_id}","agent_id":"provider","updated":1,"entries":[{{"kind":"tool","value":{call}}}]}}"#).into_bytes();
    let path = directory.join(format!("{tool_id}.json"));
    std::fs::write(&path, &bytes).unwrap();
    let actual = block_on(repository.load(tool_id.into()));
    if nocterm_ai::history::structural_bytes(&bytes).is_err() {
        // Original document preflight precedes lenient tool-field decoding.
        assert!(actual.is_err());
        assert_eq!(std::fs::read(path).unwrap(), bytes);
        return;
    }
    match expected_call {
        Ok(expected_call) => {
            let loaded =
                actual.unwrap_or_else(|error| panic!("accepted tool output {resource}: {error}"));
            let Entry::Tool(tool) = &loaded.transcript().entries[0] else {
                panic!("tool row")
            };
            assert_eq!(tool.id.as_str(), "original");
            assert_eq!(tool.output.len(), expected_call.content.len());
            if let Some(expected) = expected_tool_content {
                let nocterm_ai::model::tools::ToolOutput::Content {
                    content,
                    extensions,
                } = &tool.output[0]
                else {
                    panic!("content output")
                };
                assert_eq!(content.as_ref(), &expected);
                assert!(extensions.is_none());
            } else {
                assert!(tool.output.is_empty());
            }
        }
        Err(_) => assert!(actual.is_err(), "original tool grammar rejected {resource}"),
    }
    assert_eq!(std::fs::read(path).unwrap(), bytes);
}
#[test]
fn public_repository_keeps_original_embedded_resource_branch_grammar() {
    let authority = nocterm_ai::model::retention::RetentionAuthority::new(
        nocterm_ai::history::MAX_RESIDENT_BYTES,
    )
    .unwrap();
    use futures::executor::block_on;
    use nocterm_ai::history::{ArchivedDraftPatch, ChatWrite};
    // Literal JSON deliberately keeps duplicate fields; Value would collapse them.
    let resources = [
        r#"{"uri":"x","text":"literal"}"#,
        r#"{"uri":"x","blob":"YWJj"}"#,
        r#"{"uri":"x","text":"text first","blob":"ignored"}"#,
        r#"{"uri":"x","text":null,"blob":"YWJj"}"#,
        r#"{"uri":"x","text":{},"blob":"YWJj"}"#,
        r#"{"uri":"x","text":"a","text":"b","blob":"YWJj"}"#,
        r#"{"uri":"x","text":"a","blob":"a","blob":"b"}"#,
        r#"{"uri":"x","uri":"y","text":"a","blob":"YWJj"}"#,
        r#"{"uri":"x","text":"a","mimeType":false,"_meta":null}"#,
        r#"{"uri":"x","text":"a","mimeType":[],"_meta":{"nested":{"keep":null},"duplicate":1,"duplicate":2}}"#,
        r#"{"uri":"x","text":"a","mimeType":null,"mimeType":"text/plain","blob":"YWJj"}"#,
        r#"{"uri":"x","text":"a","_meta":null,"_meta":{},"blob":"YWJj"}"#,
        r#"{"uri":"x","text":"\uD83D\uDE00\n\"\\tail","future":{"literal":[null,1.5]}}"#,
        r#"{"uri":"x","text":"\uD83D"}"#,
        r#"[null,"text","x"]"#,
        r#"["YWJj",null,"x",{"keep":null}]"#,
        r#"[false,"text","x",null]"#,
        r#"["YWJj",false,"x"]"#,
        r#"[null,"text","x",null,"extra"]"#,
        r#"{"text":"missing uri"}"#,
    ];
    for resource in resources {
        let block = format!(r#"{{"type":"resource","resource":{resource}}}"#);
        let expected = original_resource_content(&authority, &block);
        let directory = tempfile::tempdir().unwrap();
        let id = "28627346-8a33-4b17-8a28-d4d59b360dcb";
        let bytes = format!(r#"{{"version":1,"id":"{id}","agent_id":"provider","updated":1,"entries":[{{"kind":"content","value":{block}}}]}}"#).into_bytes();
        let path = directory.path().join(format!("{id}.json"));
        std::fs::write(&path, &bytes).unwrap();
        let repository =
            FileChatRepository::with_retention(directory.path().to_owned(), authority.clone());
        let actual = block_on(repository.load(id.into()));
        let expected_tool_content = expected.as_ref().ok().cloned();
        match expected {
            Ok(expected) => {
                let loaded =
                    actual.unwrap_or_else(|error| panic!("accepted resource {resource}: {error}"));
                assert_eq!(loaded.transcript().entries, vec![Entry::Content(expected)]);
                block_on(
                    repository.save(Arc::new(ChatWrite::ArchiveDraft(ArchivedDraftPatch {
                        id: id.into(),
                        agent_id: "provider".into(),
                        text: None,
                        updated: 1,
                    }))),
                )
                .unwrap();
            }
            _ => assert!(actual.is_err(), "original rejected resource {resource}"),
        }
        assert_eq!(std::fs::read(path).unwrap(), bytes);
        assert_tool_output_keeps_original_grammar(
            &repository,
            directory.path(),
            &block,
            resource,
            expected_tool_content,
        );
    }
}

#[path = "history_tests/large_resources.rs"]
mod large_resources;
#[path = "history_tests/raw_values.rs"]
mod raw_values;
