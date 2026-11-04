use super::*;
use nocterm_ai::model::content::Content;
use nocterm_ai::model::controls::{ConfigChoice, ConfigOption};
use nocterm_ai::model::session::{PromptOutcome, SessionNotice, SessionUpdate, StopReason};
use nocterm_ai::model::usage::{ContextUsage, TokenTotals};

mod cloud_delivery;
pub(super) use cloud_delivery::captured_panel;
mod context_usage;
mod inference_allow;

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
