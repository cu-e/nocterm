use super::*;
use nocterm_ai::model::content::Content;
use nocterm_ai::model::controls::{ConfigChoice, ConfigOption};
use nocterm_ai::model::session::{PromptOutcome, SessionNotice, SessionUpdate, StopReason};
use nocterm_ai::model::usage::{ContextUsage, TokenTotals};

pub(super) fn captured_panel(
    f: &Fixture,
    cx: &mut TestAppContext,
) -> (
    Arc<nocterm_cloud_model::projection::testing::ScriptedConversationSource>,
    nocterm_cloud_model::projection::testing::ProjectionFixture,
    crate::ConversationPanel,
) {
    use nocterm_cloud_model::projection::testing::{ProjectionFixture, ScriptedConversationSource};
    let projection = ProjectionFixture::new();
    let source = Arc::new(ScriptedConversationSource::new(
        projection.binding().clone(),
    ));
    let facade = cx.update(|cx| {
        f.installation
            .installed
            .cloud_chats()
            .open(source.clone(), cx)
            .unwrap()
    });
    let selector = crate::ConversationPanel::new(f.panel.downgrade(), f.handle);
    cx.update_window(f.handle, |_, window, cx| {
        selector
            .open(facade, "Cloud task".into(), window, cx)
            .unwrap();
    })
    .unwrap();
    (source, projection, selector)
}

#[gpui_kit::test]
fn captured_conversation_uses_existing_rows_and_exact_approval_then_stop(cx: &mut TestAppContext) {
    use nocterm_cloud_model::{
        identity::{CommandId, Timestamp},
        projection::{
            ConversationIntent,
            testing::{approval_events, prompt_event, run_binding},
        },
    };
    let f = fixture(cx);
    new_chat(&f, cx);
    exchange(&f, "Local question", "Local answer", cx);
    cx.update_window(f.handle, |_, window, cx| {
        window.render_frame(cx);
        window.hover(("agent-entry", 0usize), cx);
        window.click(("copy-message", 0usize), cx);
        assert_eq!(
            cx.read_from_clipboard().unwrap().text().as_deref(),
            Some("Local question")
        );
    })
    .unwrap();
    let (source, mut projection, _selector) = captured_panel(&f, cx);
    let run = run_binding(1);
    let (intent, mut events) = approval_events(&run, 7, Timestamp::from_millis(20_000));
    events.insert(
        0,
        prompt_event(&run, "Cloud question", CommandId::from_bytes([22; 16])),
    );
    source.queue(projection.page(events)).unwrap();
    cx.run_until_parked();
    cx.update_window(f.handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(f.panel.read(cx).current().is_none());
        window.hover(("agent-entry", 0usize), cx);
        window.click(("copy-message", 0usize), cx);
        assert_eq!(
            cx.read_from_clipboard().unwrap().text().as_deref(),
            Some("Cloud question")
        );
        assert!(window.try_find(("fork-message", 0usize)).is_none());
        assert!(window.try_find("agent-attach-context").is_none());
        let approvals = window.find("agent-approvals").bounds();
        assert!(approvals.bottom() <= window.find("agent-composer").bounds().origin.y);
        window.click(("tool-Allow once", 0usize), cx);
    })
    .unwrap();
    cx.run_until_parked();
    let id = source.effects().staged[0];
    let answer = source.staged(id).unwrap();
    let ConversationIntent::AnswerApproval(request) = &answer.action else {
        panic!("expected actual approval answer");
    };
    assert_eq!(request.answer().intent(), intent.id());
    assert_eq!(request.answer().authority(), intent.authority());
    assert_eq!(request.answer().arguments(), intent.arguments());
    cx.update_window(f.handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("agent-send", cx);
    })
    .unwrap();
    cx.run_until_parked();
    let effects = source.effects();
    assert_eq!(effects.delivered.len(), 2);
    assert_eq!(
        source.staged(effects.delivered[1]).unwrap().action,
        nocterm_ai::conversation::ConversationAction::Stop
    );
    assert_eq!(
        source.staged(effects.delivered[1]).unwrap().coordinates.run,
        Some(run)
    );
}

#[gpui_kit::test]
fn captured_conversation_preserves_edited_draft_and_delivery_after_detach(cx: &mut TestAppContext) {
    use nocterm_ai::conversation::ConversationAction;
    let f = fixture(cx);
    new_chat(&f, cx);
    let local = cx.update(|cx| f.panel.read(cx).current().unwrap());
    let (source, _projection, selector) = captured_panel(&f, cx);
    let delivery = source.pause_delivery();
    cx.update_window(f.handle, |_, window, cx| {
        window.render_frame(cx);
        let focus = f.panel.read(cx).composer_focus(cx);
        replace_composer_value(focus, "original prompt", window, cx);
        window.render_frame(cx);
        window.click("agent-send", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(f.handle, |_, window, cx| {
        let focus = f.panel.read(cx).composer_focus(cx);
        replace_composer_value(focus, "new unsent draft", window, cx);
    })
    .unwrap();
    delivery.release();
    cx.run_until_parked();
    cx.update(|cx| {
        assert_eq!(
            f.panel.read(cx).composer_text(cx).as_ref(),
            "new unsent draft"
        )
    });
    let delivery = source.pause_delivery();
    cx.update_window(f.handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("agent-send", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update(|cx| selector.close(cx).unwrap());
    delivery.release();
    cx.run_until_parked();
    let effects = source.effects();
    assert_eq!(effects.staged.len(), 2);
    assert_eq!(effects.delivered, effects.staged);
    for (id, text) in effects
        .staged
        .iter()
        .zip(["original prompt", "new unsent draft"])
    {
        assert_eq!(
            source.staged(*id).unwrap().action,
            ConversationAction::Prompt(text.into())
        );
    }
    cx.update(|cx| {
        assert_eq!(
            f.panel.read(cx).current().unwrap().entity_id(),
            local.entity_id()
        )
    });
}

mod cloud_delivery;
mod context_usage;
mod inference_allow;

#[gpui_kit::test]
fn captured_conversation_retries_one_original_prompt_after_pending_and_lost_ack(
    cx: &mut TestAppContext,
) {
    let f = fixture(cx);
    let (source, mut projection, _selector) = captured_panel(&f, cx);
    let delivery = source.pause_delivery();
    source.lose_next_ack();
    cx.update_window(f.handle, |_, window, cx| {
        window.render_frame(cx);
        let focus = f.panel.read(cx).composer_focus(cx);
        replace_composer_value(focus, "same original input", window, cx);
        window.click("agent-send", cx);
    })
    .unwrap();
    cx.run_until_parked();
    let id = source.effects().staged[0];
    let original = source.staged(id).unwrap();
    cx.update_window(f.handle, |_, window, cx| window.click("agent-send", cx))
        .unwrap();
    cx.run_until_parked();
    assert_eq!(
        source.effects().staged,
        [id],
        "pending repeat must not prepare a new command"
    );
    delivery.release();
    cx.run_until_parked();
    cx.update(|cx| {
        assert_eq!(
            f.panel.read(cx).composer_text(cx).as_ref(),
            "same original input"
        )
    });
    cx.update_window(f.handle, |_, window, cx| window.click("agent-send", cx))
        .unwrap();
    cx.run_until_parked();
    let effects = source.effects();
    assert_eq!(
        effects.staged,
        [id],
        "lost ACK retry must keep its original durable M"
    );
    assert_eq!(effects.queried, [id]);
    assert!(effects.delivered.iter().all(|delivered| delivered == &id));
    let retried = source.staged(id).unwrap();
    assert_eq!(retried.action, original.action);
    assert_eq!(retried.coordinates, original.coordinates);
    assert_eq!(retried.preparation, original.preparation);
    assert_late_admission_finishes_original_input(&f, &source, &mut projection, cx);
}

fn assert_late_admission_finishes_original_input(
    f: &Fixture,
    source: &nocterm_cloud_model::projection::testing::ScriptedConversationSource,
    projection: &mut nocterm_cloud_model::projection::testing::ProjectionFixture,
    cx: &mut TestAppContext,
) {
    use nocterm_cloud_model::projection::testing::{prompt_event, run_binding};
    source.lose_next_ack();
    cx.update_window(f.handle, |_, window, cx| {
        let focus = f.panel.read(cx).composer_focus(cx);
        replace_composer_value(focus, "accepted original", window, cx);
        window.click("agent-send", cx);
    })
    .unwrap();
    cx.run_until_parked();
    let before = source.effects().staged;
    let id = *before.last().unwrap();
    source
        .queue(projection.page(vec![prompt_event(&run_binding(1), "accepted original", id)]))
        .unwrap();
    cx.run_until_parked();
    // The accepted prompt's run is allowed once, on its own.
    let allowed = source.effects().staged;
    assert_eq!(allowed[..before.len()], before);
    assert_eq!(allowed.len(), before.len() + 1);
    assert!(matches!(
        source.staged(allowed[before.len()]).unwrap().action,
        nocterm_cloud_model::projection::ConversationIntent::GrantInference
    ));
    cx.update_window(f.handle, |_, window, cx| window.click("agent-send", cx))
        .unwrap();
    cx.run_until_parked();
    assert_eq!(source.effects().staged, allowed);
    cx.update(|cx| assert!(f.panel.read(cx).composer_text(cx).is_empty()));
}

fn assert_local_rendered_stop_ignores_replacement_turn(f: &Fixture, cx: &mut TestAppContext) {
    new_chat(f, cx);
    let local = cx.update(|cx| f.panel.read(cx).current().unwrap());
    local.update(cx, |thread, cx| thread.send("Local A".into(), cx));
    cx.run_until_parked();
    let original = cx.update(|cx| {
        use nocterm_agent_chat::conversation::{ConversationSource, LocalConversation};
        LocalConversation::new(&local).capture_actions(cx).unwrap()
    });
    let rendered = cx
        .update_window(f.handle, |_, window, cx| {
            window.render_frame(cx);
            f.panel.read(cx).capture_rendered_stop(cx).unwrap()
        })
        .unwrap();
    complete_active(f, cx);
    local.update(cx, |thread, cx| thread.send("Local B".into(), cx));
    cx.run_until_parked();
    let cancels = f.commands.cancels.load(Ordering::SeqCst);
    let stale =
        cx.update(|cx| original.perform(nocterm_ai::conversation::ConversationAction::Stop, cx));
    assert!(cx.foreground_executor().block_test(stale).is_err());
    f.panel.update(cx, |panel, cx| rendered.activate(panel, cx));
    cx.run_until_parked();
    assert_eq!(f.commands.cancels.load(Ordering::SeqCst), cancels);
    cx.update(|cx| assert!(local.read(cx).session_generating()));
    cx.update_window(f.handle, |_, window, cx| window.click("agent-send", cx))
        .unwrap();
    cx.run_until_parked();
    assert_eq!(f.commands.cancels.load(Ordering::SeqCst), cancels + 1);
    complete_active(f, cx);
}

#[gpui_kit::test]
fn captured_conversation_obsolete_rendered_stop_never_stops_the_replacement_run(
    cx: &mut TestAppContext,
) {
    use nocterm_cloud_model::{
        coordinator::{CoordinatorEvent, RunStatus},
        identity::CommandId,
        projection::testing::{prompt_event, run_binding},
    };
    let f = fixture(cx);
    assert_local_rendered_stop_ignores_replacement_turn(&f, cx);
    let (source, mut projection, _selector) = captured_panel(&f, cx);
    let a = run_binding(1);
    source
        .queue(projection.page(vec![prompt_event(
            &a,
            "Cloud A",
            CommandId::from_bytes([44; 16]),
        )]))
        .unwrap();
    cx.run_until_parked();
    let rendered = cx
        .update_window(f.handle, |_, window, cx| {
            window.render_frame(cx);
            f.panel.read(cx).capture_rendered_stop(cx).unwrap()
        })
        .unwrap();
    let b = run_binding(2);
    source
        .queue(projection.page(vec![
            CoordinatorEvent::Status {
                run: a,
                status: RunStatus::Completed,
            },
            prompt_event(&b, "Cloud B", CommandId::from_bytes([45; 16])),
        ]))
        .unwrap();
    cx.run_until_parked();
    f.panel.update(cx, |panel, cx| rendered.activate(panel, cx));
    cx.run_until_parked();
    assert!(
        source.effects().staged.is_empty(),
        "the old A callback must never prepare a B Stop"
    );
    cx.update_window(f.handle, |_, window, cx| window.click("agent-send", cx))
        .unwrap();
    cx.run_until_parked();
    let effects = source.effects();
    assert_eq!(
        effects.staged.len(),
        1,
        "fresh rendered B Stop remains usable"
    );
    assert_eq!(
        source.staged(effects.staged[0]).unwrap().coordinates.run,
        Some(b)
    );
}

#[gpui_kit::test]
fn attach_menu_lists_servers_under_their_folders(cx: &mut TestAppContext) {
    let f = fixture(cx);
    let server = |id: &str, group: Option<&str>| nocterm_workspace::ConnectionSummary {
        id: id.to_owned().into(),
        name: id.to_owned().into(),
        group: group.map(|group| group.to_owned().into()),
        description: Default::default(),
        target: serde_json::from_value(
            serde_json::json!({"host":"example.test", "port":22, "user":"user"}),
        )
        .unwrap(),
        icon: None,
        flag: None,
    };
    let directory = Rc::new(Directory(std::cell::RefCell::new(vec![
        server("proxmox", Some("homelab")),
        server("backup", None),
        server("ai-server", Some("homelab")),
    ])));
    cx.update(|cx| {
        f.workspace.update(cx, |workspace, _| {
            workspace.set_connection_directory(directory.clone())
        })
    });
    new_chat(&f, cx);
    cx.update_window(f.handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("agent-attach-context", cx);
        window.render_frame(cx);
        let bounds = |id: &str| window.find(id.to_owned()).bounds();
        let backup = bounds("attach-connection-backup");
        let folder = bounds("attach-group-homelab");
        let ai = bounds("attach-connection-ai-server");
        let proxmox = bounds("attach-connection-proxmox");
        assert!(backup.origin.y < folder.origin.y);
        assert!(folder.origin.y < ai.origin.y && ai.origin.y < proxmox.origin.y);
        // Servers in a folder sit indented under it.
        assert!(ai.origin.x > folder.origin.x + gpui_kit::px(8.));
        assert_eq!(proxmox.origin.x, ai.origin.x);
        assert_eq!(backup.origin.x, folder.origin.x);
        window.click("attach-group-homelab", cx);
        let thread = f.panel.read(cx).current().unwrap();
        assert!(
            thread
                .read(cx)
                .composer()
                .attachments()
                .contains(&Attachment::Group("homelab".into()))
        );
    })
    .unwrap();
}

#[test]
fn implicit_markdown_images_are_disabled_and_config_labels_show_selection() {
    assert_eq!(
        crate::panel::widgets::safe_markdown("![secret](file:///hidden)<img src='x'>"),
        "[secret](file:///hidden)&lt;img src='x'>"
    );
    let option = ConfigOption::select(
        "model",
        "Model",
        "one",
        vec![ConfigChoice::new("one", "First")],
    );
    assert_eq!(crate::thread::config_label(&option), "First");
}
#[gpui_kit::test]
fn image_preparation_runs_once_and_renders_a_cached_small_static_preview(cx: &mut TestAppContext) {
    let codec = image_codec();
    let f = fixture_with_width(
        cx,
        26.,
        workspace_resolver(),
        crate::ImageDeps::new(codec.clone(), codec.clone()),
    );
    f.connector.support_images();
    new_chat(&f, cx);
    let data = image_bytes();
    let expected = Arc::new(gpui_kit::Image::from_bytes(
        gpui_kit::ImageFormat::Png,
        data,
    ));
    cx.update(|cx| assert!(!expected.is_asset_cached(cx)));
    let thread = cx.update(|cx| f.panel.read(cx).current().unwrap());
    thread.update(cx, |thread, cx| {
        thread
            .accept_composer_images(vec![prompt_image()], cx)
            .unwrap();
    });
    cx.update_window(f.handle, |_, window, cx| window.render_frame(cx))
        .unwrap();
    cx.run_until_parked();
    assert_eq!(codec.requests().len(), 1);
    // Draw the real composer strip, then deliver its image asset completion.
    cx.update_window(f.handle, |_, window, cx| window.render_frame(cx))
        .unwrap();
    cx.run_until_parked();
    let rendered = cx
        .update_window(f.handle, |_, window, cx| {
            window.draw(cx).clear(cx);
            assert!(expected.is_asset_cached(cx));
            expected
                .clone()
                .get_render_image(window, cx)
                .expect("successful preview rendering must load its supplied PNG")
        })
        .unwrap();
    assert_eq!(rendered.frame_count(), 1);
    assert_eq!(rendered.size(0), gpui_kit::size(2u32.into(), 2u32.into()));
    assert_eq!(rendered.as_bytes(0).unwrap().len(), 2 * 2 * 4);
    cx.update_window(f.handle, |_, window, cx| {
        window.draw(cx).clear(cx);
        assert!(window.has_image_atlas_entry(&rendered));
        let reused = expected.clone().get_render_image(window, cx).unwrap();
        assert!(Arc::ptr_eq(&rendered, &reused));
    })
    .unwrap();
    assert_eq!(codec.requests().len(), 1);
}

#[gpui_kit::test]
fn agent_popup_is_anchored_to_header_and_dismisses(cx: &mut TestAppContext) {
    let f = fixture(cx);
    cx.update_window(f.handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("agent-picker").is_none());
        let trigger = window.find("agent-new-thread").bounds();
        window.click("agent-new-thread", cx);
        let picker = window.find("agent-picker").bounds();
        assert!(picker.origin.y >= trigger.bottom());
        for agent in ["codex", "claude", "hermes"] {
            let row = window
                .find(gpui_kit::SharedString::from(format!("new-agent-{agent}")))
                .bounds();
            assert_eq!(row.origin.x, picker.origin.x);
            assert_eq!(row.size.width, picker.size.width);
        }
        window.press("escape", cx);
        window.render_frame(cx);
        assert!(window.try_find("agent-picker").is_none());
    })
    .unwrap();
}

#[gpui_kit::test]
fn composer_popups_select_config_modes_and_context(cx: &mut TestAppContext) {
    let f = fixture_with_width(cx, 18., workspace_resolver(), image_deps());
    f.commands.config.set_options(vec![ConfigOption::select(
        "model",
        "Model",
        "one",
        vec![
            ConfigChoice::new(
                "one",
                "Very long selected model name that must fit the narrow panel",
            ),
            ConfigChoice::new("two", "Second"),
        ],
    )]);
    f.commands.config.set_modes(ask_and_code());
    new_chat(&f, cx);
    cx.update_window(f.handle, |_, window, cx| {
        window.render_frame(cx);
        let composer = window.find("agent-composer").bounds();
        let send = window.find("agent-send").bounds();
        assert!(send.right() <= composer.right());
        assert!(send.origin.x >= composer.origin.x);
        let selector = window.find("config-picker-model").bounds();
        assert!(selector.right() <= send.origin.x);
        assert_eq!(window.find("mode-picker").label(), Some("Ask"));
        window.click("config-picker-model", cx);
        let picker = window.find("agent-picker").bounds();
        assert!(picker.bottom() <= selector.origin.y);
        let row = window.find("config-model-two").bounds();
        assert_eq!(row.origin.x, picker.origin.x);
        assert_eq!(row.size.width, picker.size.width);
        window.click("config-model-two", cx);
        assert!(window.try_find("agent-picker").is_none());
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(f.handle, |_, window, cx| {
        window.click("mode-picker", cx);
        window.click("mode-code", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(f.handle, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(window.find("mode-picker").label(), Some("Code"));
        window.click("agent-attach-context", cx);
        let attachment = gpui_kit::SharedString::from(format!("attach-{:?}", f.terminal));
        window.click(attachment, cx);
        assert!(
            f.panel
                .read(cx)
                .current()
                .unwrap()
                .read(cx)
                .composer()
                .attachments()
                .is_empty()
        );
        window.press("escape", cx);
        window.render_frame(cx);
        assert!(window.try_find("agent-picker").is_none());
        window.click("agent-maximize", cx);
        window.render_frame(cx);
        let composer = window.find("agent-composer").bounds();
        let send = window.find("agent-send").bounds();
        assert!(send.right() <= composer.right());
        assert!(send.origin.x >= composer.origin.x);
    })
    .unwrap();
}

#[test]
fn chat_title_and_usage_handle_empty_unicode_unknown_and_overflow() {
    use nocterm_ai::thread::{Entry, ThreadState};
    let mut state = ThreadState {
        title: Some("Provider title".into()),
        ..Default::default()
    };
    let title = crate::thread::chat_title;
    assert_eq!(title(None, &state), "New Agent");
    assert_eq!(title(Some("Mine"), &state), "Mine");
    state.entries.push(Entry::User(vec![Content::Text(
        nocterm_ai::model::content::Text::new("Привет 🌍"),
    )]));
    assert_eq!(title(None, &state), "Provider title");
    assert_eq!(title(Some("  "), &state), "Provider title");
    state.title = None;
    assert_eq!(title(None, &state), "Привет 🌍");
    assert_eq!(crate::panel::usage::usage_fraction(None), 0.);
    for (used, size, expected) in [
        (50, 100, 0.5),
        (120, 100, 1.),
        (100, 0, 0.),
        (85, 100, 0.85),
    ] {
        let usage = ContextUsage::new(used, size);
        assert_eq!(crate::panel::usage::usage_fraction(Some(&usage)), expected);
    }
}

fn ask_and_code() -> nocterm_ai::model::controls::Modes {
    use nocterm_ai::model::controls::{ModeChoice, Modes};
    Modes::new(
        "ask",
        vec![
            ModeChoice::new("ask", "Ask"),
            ModeChoice::new("code", "Code"),
        ],
    )
}

#[test]
fn transcript_activity_tracks_only_live_reasoning_and_unfinished_tools() {
    use nocterm_ai::thread::Entry;
    let tool = |status| {
        Entry::Tool(serde_json::from_value(serde_json::json!({"toolCallId":"call","title":"Read terminal","status":status,"kind":"read"})).unwrap())
    };
    let mut entries = vec![
        Entry::Thought("Earlier thought".into()),
        Entry::Agent("Answer".into()),
        Entry::Thought("Live thought".into()),
    ];
    assert!(!crate::panel::widgets::entry_is_live(&entries, 0, true));
    assert!(crate::panel::widgets::entry_is_live(&entries, 2, true));
    assert!(!crate::panel::widgets::entry_is_live(&entries, 2, false));
    entries.push(tool("in_progress"));
    assert!(!crate::panel::widgets::entry_is_live(&entries, 2, true));
    assert!(crate::panel::widgets::entry_is_live(&entries, 3, true));
    for status in ["completed", "failed"] {
        entries[3] = tool(status);
        assert!(!crate::panel::widgets::entry_is_live(&entries, 3, true));
    }
    entries[3] = tool("pending");
    assert!(crate::panel::widgets::entry_is_live(&entries, 3, true));
    assert!(!crate::panel::widgets::entry_is_live(&entries, 3, false));
}

#[gpui_kit::test]
fn compact_chips_and_live_reasoning_respect_narrow_layout_and_reduced_motion(
    cx: &mut TestAppContext,
) {
    let f = fixture_with_width(cx, 18., workspace_resolver(), image_deps());
    new_chat(&f, cx);
    let thread = cx.update(|cx| f.panel.read(cx).current().unwrap());
    thread.update(cx, |thread, cx| thread.send("Question".into(), cx));
    cx.run_until_parked();
    thread.update(cx, |thread, cx| {
        report_control(
            thread,
            SessionUpdate::AgentThoughtChunk(Content::Text(nocterm_ai::model::content::Text::new(
                "Inspecting the current terminal carefully before running commands",
            ))),
            cx,
        )
    });
    cx.run_until_parked();
    cx.update_window(f.handle, |_, window, cx| {
        window.render_frame(cx);
        let chips = window.find("agent-attachment-chips").bounds();
        let chip = window.find("chip-Terminal").bounds();
        assert!(chip.size.width < chips.size.width);
        assert_eq!(chip.origin.x, chips.origin.x);
        let panel = window.find("agent-panel").bounds();
        let bubble = window.find(("agent-user-bubble", 0usize)).bounds();
        assert!(bubble.right() <= panel.right());
        let user_row = window.find(("agent-entry", 0usize)).bounds();
        assert_eq!(bubble.right(), user_row.right() - gpui_kit::px(12.));
        assert!(
            bubble.size.width < user_row.size.width * 0.5,
            "short bubble should fit its content"
        );
        let thought = window.find(("thought", 1usize)).bounds();
        assert!(
            thought.right() <= panel.right(),
            "thought {thought:?} outside panel {panel:?}; entry {:?}",
            window.find(("agent-entry", 1usize)).bounds()
        );
        assert!(
            window.simulate_next_frame(cx) > 0,
            "live reasoning must animate"
        );
        cx.set_reduce_motion(true);
        window.render_frame(cx);
        assert_eq!(
            window.simulate_next_frame(cx),
            0,
            "reduced motion must stop animation"
        );
    })
    .unwrap();
    cx.update(|cx| cx.set_reduce_motion(false));
    complete_active(&f, cx);
    cx.update_window(f.handle, |_, window, cx| {
        window.render_frame(cx);
        // Deliver final publication/layout callbacks before checking sustained animation.
        window.simulate_next_frame(cx);
        window.render_frame(cx);
        assert_eq!(
            window.simulate_next_frame(cx),
            0,
            "completed reasoning must be static"
        );
        window.click("chip-Terminal", cx);
        assert!(
            f.panel
                .read(cx)
                .current()
                .unwrap()
                .read(cx)
                .composer()
                .attachments()
                .is_empty()
        );
    })
    .unwrap();
}

#[gpui_kit::test]
fn expanded_long_mcp_command_json_and_reasoning_stay_within_narrow_panel(cx: &mut TestAppContext) {
    let f = fixture_with_width(cx, 18., workspace_resolver(), image_deps());
    new_chat(&f, cx);
    let long = "very_long_unbroken_argument_".repeat(300);
    cx.update(|cx| {
        f.panel.read(cx).current().unwrap().update(cx, |thread, cx| {
            restore_entries(&f,thread,vec![nocterm_ai::thread::Entry::Tool(serde_json::from_value(serde_json::json!({"toolCallId":"mcp","title":format!("terminal.run_command {long}"),"status":"completed","kind":"execute","content":[{"type":"content","content":{"type":"text","text":long}}]})).unwrap())],cx);
            cx.notify();
        });
    });
    cx.update_window(f.handle, |_, window, cx| {
        window.render_frame(cx);
        window.click(("tool-call", 0usize), cx);
        let panel = window.find("agent-panel").bounds();
        for id in ["agent-entry", "tool-call", "tool-output"] {
            let bounds = window.find((id, 0usize)).bounds();
            assert!(
                bounds.right() <= panel.right(),
                "{id}: {bounds:?}, panel {panel:?}"
            );
            assert!(bounds.origin.x >= panel.origin.x);
        }
        // Wrapped, the output is tall enough to push the header out of view.
        window.scroll(
            "agent-transcript-area",
            gpui_kit::ScrollDelta::Pixels(gpui_kit::point(
                gpui_kit::px(0.),
                gpui_kit::px(1_000_000.),
            )),
            cx,
        );
        window.render_frame(cx);
        window.click(("tool-call", 0usize), cx);
        window.render_frame(cx);
        assert!(window.try_find(("tool-output", 0usize)).is_none());
    })
    .unwrap();
    cx.update(|cx| {
        f.panel
            .read(cx)
            .current()
            .unwrap()
            .update(cx, |thread, cx| {
                restore_entries(
                    &f,
                    thread,
                    vec![nocterm_ai::thread::Entry::Thought(format!(
                        "Inspecting command `{long}`\n\n```json\n{{\"command\":\"{long}\"}}\n```"
                    ))],
                    cx,
                );
                cx.notify();
            });
    });
    cx.update_window(f.handle, |_, window, cx| {
        window.render_frame(cx);
        window.click(("thought", 0usize), cx);
        let panel = window.find("agent-panel").bounds();
        for id in ["agent-entry", "thought", "thought-output"] {
            let bounds = window.find((id, 0usize)).bounds();
            assert!(
                bounds.right() <= panel.right(),
                "{id}: {bounds:?}, panel {panel:?}"
            );
        }
    })
    .unwrap();
}
