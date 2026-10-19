use super::*;
use nocterm_ai::model::{
    controls::{ConfigChoice, ConfigKind, ModeChoice, SelectChoices},
    ids::{ConfigChoiceId, ConfigId, ModeId},
    literal::{LiteralIdentity, TextRange},
    tools::{ToolKind, ToolOutput, ToolStatus},
};
pub(super) fn select(
    id: &str,
    name: &str,
    current: &str,
    choices: Vec<(&str, &str)>,
) -> ConfigOption {
    ConfigOption {
        id: ConfigId::new(id),
        name: name.into(),
        description: None,
        category: None,
        kind: ConfigKind::Select {
            current: ConfigChoiceId::new(current),
            choices: SelectChoices::Flat(
                choices
                    .into_iter()
                    .map(|(id, name)| ConfigChoice {
                        id: ConfigChoiceId::new(id),
                        name: name.into(),
                        description: None,
                        extensions: None,
                    })
                    .collect(),
            ),
        },
        extensions: None,
    }
}

fn finished_with_tokens() -> PromptOutcome {
    PromptOutcome {
        ending: EndOfTurn::Finished,
        tokens: Some(nocterm_ai::model::usage::TokenTotals {
            total: 30_000,
            input: 25_000,
            output: 5_000,
            thought: None,
            cache_read: Some(20_000),
            cache_write: None,
        }),
    }
}

#[gpui_kit::test]
fn usage_card_shows_context_tokens_and_plan_limits(cx: &mut TestAppContext) {
    let source = Arc::new(nocterm_ai::testing::ScriptedUsageSource::default());
    source.push(Ok(Some(vec![
        nocterm_ai::usage::RateLimit {
            window: nocterm_ai::usage::Window::Weekly,
            used: Some(0.38),
            resets_at: Some(1_791_588_105),
            reached: false,
        },
        nocterm_ai::usage::RateLimit {
            window: nocterm_ai::usage::Window::FiveHour,
            used: Some(0.12),
            resets_at: Some(1_791_500_000),
            reached: false,
        },
    ])));
    let f = fixture_with_width(cx, 26., source, workspace_resolver(), image_deps());
    new_chat(&f, cx);
    let session = cx.update(|cx| {
        let thread = f.panel.read(cx).current().unwrap();
        thread.update(cx, |thread, cx| thread.send("hello".into(), cx));
        thread.read(cx).session().clone().unwrap()
    });
    cx.run_until_parked();
    f.events
        .try_send(AgentEvent::Session(SessionNotice {
            session,
            update: SessionUpdate::Usage(nocterm_ai::model::notice::UsageNotice {
                context: nocterm_ai::model::usage::ContextUsage {
                    used: 25_900,
                    size: 256_000,
                    cost: None,
                },
                rate_limits: vec![],
            }),
        }))
        .unwrap();
    cx.run_until_parked();
    let finish = f.commands.pending.lock().unwrap().take().unwrap();
    finish.send(finished_with_tokens()).unwrap();
    cx.run_until_parked();
    cx.update(|cx| {
        let runtime = f.installation.installed.runtime();
        let limits: Vec<_> = runtime
            .limits("codex", cx)
            .unwrap()
            .iter()
            .cloned()
            .collect();
        assert_eq!(limits.len(), 2);
        assert_eq!(limits[0].window, nocterm_ai::usage::Window::FiveHour);
        let thread = f.panel.read(cx).current().unwrap();
        assert_eq!(
            thread
                .read(cx)
                .transcript()
                .unwrap()
                .tokens()
                .unwrap()
                .total,
            30_000
        );
    });
    cx.update_window(f.handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("agent-context-usage", cx);
        window.render_frame(cx);
        let card = window.find("agent-usage").bounds();
        let composer = window.find("agent-composer").bounds();
        // Over the chat, above the composer.
        assert!(card.bottom() <= composer.origin.y + gpui_kit::px(1.));
        window.click("agent-usage-close", cx);
        window.render_frame(cx);
        assert!(window.try_find("agent-usage").is_none());
    })
    .unwrap();
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
    let option = select("model", "Model", "one", vec![("one", "First")]);
    assert_eq!(crate::thread::config_label(&option), "First");
}
#[gpui_kit::test]
fn image_preparation_runs_once_and_renders_a_cached_small_static_preview(cx: &mut TestAppContext) {
    let codec = image_codec();
    let f = fixture_with_width(
        cx,
        26.,
        usage_source(),
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
        attach_image(thread, image_bytes(), codec.clone(), cx);
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

fn mode_choice(id: &str, name: &str) -> ModeChoice {
    ModeChoice {
        id: ModeId::new(id),
        name: name.into(),
        description: None,
        extensions: None,
    }
}

#[gpui_kit::test]
fn composer_popups_select_config_modes_and_context(cx: &mut TestAppContext) {
    let f = fixture_with_width(cx, 18., usage_source(), workspace_resolver(), image_deps());
    f.commands.config.set_options(vec![select(
        "model",
        "Model",
        "one",
        vec![
            (
                "one",
                "Very long selected model name that must fit the narrow panel",
            ),
            ("two", "Second"),
        ],
    )]);
    f.commands.config.set_modes(Modes {
        current: ModeId::new("ask"),
        choices: vec![mode_choice("ask", "Ask"), mode_choice("code", "Code")],
        extensions: None,
    });
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

#[gpui_kit::test]
fn chat_title_and_usage_handle_empty_unicode_unknown_and_overflow(cx: &mut TestAppContext) {
    let f = fixture(cx);
    new_chat(&f, cx);
    let thread = cx.update(|cx| f.panel.read(cx).current().unwrap());
    cx.update(|cx| assert_eq!(thread.read(cx).title(), "New Agent"));
    thread.update(cx, |thread, cx| thread.send("Привет 🌍".into(), cx));
    cx.run_until_parked();
    cx.update(|cx| assert_eq!(thread.read(cx).title(), "Привет 🌍"));
    thread.update(cx, |thread, cx| thread.rename(Some("Mine".into()), cx));
    cx.update(|cx| assert_eq!(thread.read(cx).title(), "Mine"));
    thread.update(cx, |thread, cx| thread.rename(Some("  ".into()), cx));
    cx.update(|cx| assert_eq!(thread.read(cx).title(), "Привет 🌍"));
    assert_eq!(crate::panel::usage::usage_fraction(None), 0.);
    for (used, size, expected) in [
        (50, 100, 0.5),
        (120, 100, 1.),
        (100, 0, 0.),
        (85, 100, 0.85),
    ] {
        let usage = nocterm_ai::model::usage::ContextUsage {
            used,
            size,
            cost: None,
        };
        assert_eq!(crate::panel::usage::usage_fraction(Some(&usage)), expected);
    }
}

#[test]
fn transcript_activity_tracks_only_live_reasoning_and_unfinished_tools() {
    use nocterm_ai::thread::Entry;
    let tool = |status: ToolStatus| {
        let mut call = super::provider_fixtures::provider_tool("call", "Read terminal", status);
        call.kind = ToolKind::Read;
        Entry::Tool(call)
    };
    let authority = nocterm_ai::model::retention::RetentionAuthority::new(usize::MAX).unwrap();
    let live = |entries: &[Entry], index, generating| {
        let rows =
            nocterm_ai::testing::ManagedRows::with_authority(&authority, entries.to_vec()).unwrap();
        crate::panel::widgets::entry_is_live(rows.rows(), index, generating)
    };
    let mut entries = vec![
        Entry::Thought("Earlier thought".into()),
        Entry::Agent("Answer".into()),
        Entry::Thought("Live thought".into()),
    ];
    assert!(!live(&entries, 0, true));
    assert!(live(&entries, 2, true));
    assert!(!live(&entries, 2, false));
    entries.push(tool(ToolStatus::Running));
    assert!(!live(&entries, 2, true));
    assert!(live(&entries, 3, true));
    for status in [ToolStatus::Succeeded, ToolStatus::Failed] {
        entries[3] = tool(status);
        assert!(!live(&entries, 3, true));
    }
    entries[3] = tool(ToolStatus::Pending);
    assert!(live(&entries, 3, true));
    assert!(!live(&entries, 3, false));
}

#[gpui_kit::test]
fn compact_chips_and_live_reasoning_respect_narrow_layout_and_reduced_motion(
    cx: &mut TestAppContext,
) {
    let f = fixture_with_width(cx, 18., usage_source(), workspace_resolver(), image_deps());
    new_chat(&f, cx);
    let thread = cx.update(|cx| f.panel.read(cx).current().unwrap());
    thread.update(cx, |thread, cx| thread.send("Question".into(), cx));
    cx.run_until_parked();
    thread.update(cx, |thread, cx| {
        report_control(
            &f,
            thread,
            SessionUpdate::ThoughtChunk(Content::Text(Text {
                text: "Inspecting the current terminal carefully before running commands".into(),
                annotations: None,
                extensions: None,
            })),
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

/// An executed MCP command whose title and output are one long unbroken word.
fn long_mcp_call(long: &str) -> nocterm_ai::model::tools::ToolRecord {
    let mut call = super::provider_fixtures::provider_tool(
        "mcp",
        format!("terminal.run_command {long}"),
        ToolStatus::Succeeded,
    );
    call.kind = ToolKind::Execute;
    call.literal.title = LiteralIdentity::Historical {
        tool: TextRange::supplied(&call.title, &call.title[9..20]).unwrap(),
    };
    call.output = vec![ToolOutput::Content {
        content: Box::new(Content::Text(Text {
            text: long.to_owned(),
            annotations: None,
            extensions: None,
        })),
        extensions: None,
    }];
    call
}

#[gpui_kit::test]
fn expanded_long_mcp_command_json_and_reasoning_stay_within_narrow_panel(cx: &mut TestAppContext) {
    let f = fixture_with_width(cx, 18., usage_source(), workspace_resolver(), image_deps());
    new_chat(&f, cx);
    let long = "very_long_unbroken_argument_".repeat(300);
    cx.update(|cx| {
        f.panel
            .read(cx)
            .current()
            .unwrap()
            .update(cx, |thread, cx| {
                let call = long_mcp_call(&long);
                restore_entries(&f, thread, vec![nocterm_ai::thread::Entry::Tool(call)], cx);
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
        window.click(("thought", 1usize), cx);
        let panel = window.find("agent-panel").bounds();
        for id in ["agent-entry", "thought", "thought-output"] {
            let bounds = window.find((id, 1usize)).bounds();
            assert!(
                bounds.right() <= panel.right(),
                "{id}: {bounds:?}, panel {panel:?}"
            );
        }
    })
    .unwrap();
}

#[gpui_kit::test]
fn prompt_completion_refreshes_its_original_usage_source_after_another_installation(
    cx: &mut TestAppContext,
) {
    let source = usage_source();
    source.push(Ok(Some(vec![nocterm_ai::usage::RateLimit {
        window: nocterm_ai::usage::Window::Weekly,
        used: Some(0.38),
        resets_at: None,
        reached: false,
    }])));
    let first = fixture_with_width(cx, 26., source.clone(), workspace_resolver(), image_deps());
    new_chat(&first, cx);
    let thread = cx.update(|cx| first.panel.read(cx).current().unwrap());
    thread.update(cx, |thread, cx| thread.send("original prompt".into(), cx));
    cx.run_until_parked();
    let finish = first.commands.pending.lock().unwrap().take().unwrap();
    let other_source = usage_source();
    let second = fixture_with_width(
        cx,
        26.,
        other_source.clone(),
        workspace_resolver(),
        image_deps(),
    );
    finish
        .send(PromptOutcome {
            ending: EndOfTurn::Finished,
            tokens: None,
        })
        .unwrap();
    cx.run_until_parked();
    assert_eq!(source.requests().len(), 1);
    assert!(other_source.requests().is_empty());
    cx.update(|cx| {
        assert!(
            first
                .installation
                .installed
                .runtime()
                .limits("codex", cx)
                .is_some()
        );
        assert!(
            second
                .installation
                .installed
                .runtime()
                .limits("codex", cx)
                .is_none()
        );
    });
}
