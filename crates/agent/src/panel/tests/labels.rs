use super::*;
use gpui_kit::px;
use nocterm_ai::thread::Entry;

use crate::panel::widgets::single_line_label;

#[test]
fn labels_collapse_unicode_whitespace_without_changing_words() {
    for (raw, expected) in [
        ("смотри в чем соль\nтут", "смотри в чем соль тут"),
        (
            "  first\r\n\tsecond\u{a0}\u{2003}third\u{2028}fourth  ",
            "first second third fourth",
        ),
        ("\n\t\u{3000}", ""),
        ("", ""),
        ("run('two  spaces')", "run('two spaces')"),
        ("Already single-line 🚀", "Already single-line 🚀"),
    ] {
        assert_eq!(single_line_label(raw), expected);
    }
}

fn settle(f: &Fixture, cx: &mut TestAppContext) {
    for _ in 0..3 {
        cx.update_window(f.handle, |_, window, cx| window.render_frame(cx))
            .unwrap();
        cx.run_until_parked();
    }
}

fn chat_label_heights(f: &Fixture, raw: &str, cx: &mut TestAppContext) -> [gpui_kit::Pixels; 2] {
    settle(f, cx);
    cx.update_window(f.handle, |_, window, cx| {
        let id = f.panel.read(cx).current().unwrap().entity_id().as_u64();
        let title = window.find("agent-chat-title").bounds();
        let header = window.find("agent-header").bounds();
        let row = window.find(("history-thread", id));
        assert_eq!(row.label(), Some(raw), "accessibility keeps the full title");
        assert!(title.origin.y >= header.origin.y && title.bottom() <= header.bottom());
        assert!(title.right() <= header.right());
        assert!(row.bounds().right() <= window.find("agent-history-list").bounds().right());
        [title.size.height, row.bounds().size.height]
    })
    .unwrap()
}

#[gpui_kit::test]
fn chat_header_and_history_keep_multiline_titles_on_one_line(cx: &mut TestAppContext) {
    let f = fixture_with_width(cx, 18.);
    new_chat(&f, cx);
    cx.update(|cx| cx.set_reduce_motion(true));
    let prompt = "смотри в чем соль\nтут";
    let session = exchange(&f, prompt, "answer", cx);
    let thread = cx.update(|cx| f.panel.read(cx).current().unwrap());
    f.panel.update(cx, |panel, cx| {
        panel.history = true;
        cx.notify();
    });
    // Compare identical words: explicit line breaks must not increase either label's height.
    thread.update(cx, |thread, cx| {
        thread.rename(Some(single_line_label(prompt)), cx)
    });
    let baseline = chat_label_heights(&f, &single_line_label(prompt), cx);
    thread.update(cx, |thread, cx| thread.rename(None, cx));
    assert_eq!(chat_label_heights(&f, prompt, cx), baseline);
    cx.update(|cx| {
        let thread = thread.read(cx);
        assert_eq!(thread.title(), prompt);
        assert!(matches!(&thread.state.entries[0], Entry::User(parts)
            if matches!(&parts[0], acp::ContentBlock::Text(text) if text.text == prompt)));
    });

    let agent_title = "Agent\r\n  title\u{2028}with\tspaces";
    f.events
        .try_send(AgentEvent::Session(acp::SessionNotification::new(
            session,
            acp::SessionUpdate::SessionInfoUpdate(acp::SessionInfoUpdate::new().title(agent_title)),
        )))
        .unwrap();
    cx.run_until_parked();
    assert_eq!(chat_label_heights(&f, agent_title, cx), baseline);

    let renamed = "User\n\tname\u{2003}with spaces";
    thread.update(cx, |thread, cx| thread.rename(Some(renamed.into()), cx));
    assert_eq!(chat_label_heights(&f, renamed, cx), baseline);
    let long = "A lengthy chat title with many words ".repeat(20);
    let long = format!("{}\nsecond line", long.trim());
    thread.update(cx, |thread, cx| thread.rename(Some(long.clone()), cx));
    assert_eq!(chat_label_heights(&f, &long, cx), baseline);
    cx.update(|cx| {
        let thread = thread.read(cx);
        assert_eq!(thread.name.as_deref(), Some(long.as_str()));
        assert_eq!(thread.state.title.as_deref(), Some(agent_title));
    });
    cx.run_until_parked();
    let saved = nocterm_ai::history::load_all(&f._directory.path().join("chats"));
    assert_eq!(saved[0].name.as_deref(), Some(long.as_str()));
    assert_eq!(saved[0].title.as_deref(), Some(agent_title));
    assert!(matches!(&saved[0].entries[0], Entry::User(parts)
        if matches!(&parts[0], acp::ContentBlock::Text(text) if text.text == prompt)));

    // The existing single-line rename editor still receives the original title,
    // applying its own newline filtering. Cancelling it keeps the stored title.
    cx.update_window(f.handle, |_, window, cx| {
        f.panel.update(cx, |panel, cx| {
            panel.start_rename(thread.entity_id(), window, cx);
            assert_eq!(
                panel
                    .renaming
                    .as_ref()
                    .unwrap()
                    .input
                    .read(cx)
                    .value()
                    .as_ref(),
                long.replace('\n', "")
            );
            panel.finish_rename(false, window, cx);
            panel
                .history_search
                .update(cx, |search, cx| search.set_value("second line", window, cx));
        });
        window.render_frame(cx);
        assert!(
            window
                .try_find(("history-thread", thread.entity_id().as_u64()))
                .is_some()
        );
        assert_eq!(thread.read(cx).name.as_deref(), Some(long.as_str()));
    })
    .unwrap();
}

#[gpui_kit::test]
fn completed_and_live_heredoc_tool_headers_keep_one_line_and_raw_command(cx: &mut TestAppContext) {
    let f = fixture_with_width(cx, 18.);
    new_chat(&f, cx);
    let thread = cx.update(|cx| f.panel.read(cx).current().unwrap());
    let script = "python3 - <<'PY'\nfrom pathlib import Path\nPath('/tmp/probe.txt').write_text('nocterm-ui-test-artifacts')\nPY";
    // Exercise both animated glyphs and reduced-motion live rendering.
    for reduced_motion in [false, true] {
        cx.update(|cx| cx.set_reduce_motion(reduced_motion));
        for (status, live) in [
            (acp::ToolCallStatus::Completed, false),
            (acp::ToolCallStatus::Failed, false),
            (acp::ToolCallStatus::Pending, true),
            (acp::ToolCallStatus::InProgress, true),
        ] {
            let mut heights = Vec::new();
            for title in [single_line_label(script), script.into()] {
                thread.update(cx, |thread, cx| {
                    let mut call = acp::ToolCall::new("script", title.clone());
                    call.status = status;
                    call.raw_input = Some(serde_json::json!({"command": script}));
                    call.content = vec![
                        serde_json::from_value(serde_json::json!({
                            "type": "content",
                            "content": {"type": "text", "text": script},
                        }))
                        .unwrap(),
                    ];
                    thread.state.entries = vec![Entry::Tool(call)];
                    thread.lifecycle.force_phase(if live {
                        nocterm_ai::session::SessionPhase::Prompting
                    } else {
                        nocterm_ai::session::SessionPhase::Ready
                    });
                    thread.mark_dirty(0);
                    cx.notify();
                });
                settle(&f, cx);
                heights.push(
                    cx.update_window(f.handle, |_, window, cx| {
                        let header = window.find(("tool-call", 0usize));
                        assert_eq!(
                            header.label(),
                            Some(format!("{title} · {status:?}").as_str())
                        );
                        let entry = window.find(("agent-entry", 0usize)).bounds();
                        assert!(header.bounds().right() <= entry.right() + px(1.));
                        let Entry::Tool(call) = &thread.read(cx).state.entries[0] else {
                            panic!("expected tool call");
                        };
                        assert_eq!(call.title, title);
                        assert_eq!(call.raw_input.as_ref().unwrap()["command"], script);
                        header.bounds().size.height
                    })
                    .unwrap(),
                );
            }
            assert_eq!(
                heights[0], heights[1],
                "multiline tool header grew: live={live}, reduced_motion={reduced_motion}"
            );
            assert!(heights[1] < px(40.));
            cx.update_window(f.handle, |_, window, cx| {
                window.click(("tool-call", 0usize), cx);
                assert!(window.try_find(("tool-output", 0usize)).is_some());
                let Entry::Tool(call) = &thread.read(cx).state.entries[0] else {
                    panic!("expected tool call");
                };
                let acp::ToolCallContent::Content(output) = &call.content[0] else {
                    panic!("expected text output");
                };
                assert!(matches!(&output.content, acp::ContentBlock::Text(text)
                    if text.text == script));
                window.click(("tool-call", 0usize), cx);
                assert!(window.try_find(("tool-output", 0usize)).is_none());
            })
            .unwrap();
        }
    }
}
