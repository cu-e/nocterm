//! Chat controls use the existing virtual scroller instead of a second viewport.
use super::*;
use gpui_kit::{ListOffset, point, px};
use nocterm_ai::thread::Entry;

pub(super) fn content(text: impl Into<String>) -> Content {
    Content::Text(Text {
        text: text.into(),
        annotations: None,
        extensions: None,
    })
}
fn tool(id: &str, title: &str) -> nocterm_ai::model::tools::ToolRecord {
    use nocterm_ai::model::{
        ids::ToolId,
        tools::{ToolKind, ToolRecord, ToolStatus},
    };
    ToolRecord {
        id: ToolId::new(id),
        title: title.into(),
        name: None,
        kind: ToolKind::Other,
        status: ToolStatus::Pending,
        output: Vec::new(),
        locations: Vec::new(),
        arguments: None,
        result: None,
        extensions: None,
        claims: Default::default(),
        literal: Default::default(),
        client_display: None,
    }
}

fn transcript(f: &Fixture, cx: &mut TestAppContext) -> Entity<crate::thread::AgentThread> {
    conversation(f, true, cx)
}
/// Nine rows: user, thought, tool, agent, thought, user, thought, tall agent, tool. Without a
/// first question, a leading tool stands in for the first user row.
fn conversation(
    f: &Fixture,
    first_question: bool,
    cx: &mut TestAppContext,
) -> Entity<crate::thread::AgentThread> {
    new_chat(f, cx);
    let thread = cx.update(|cx| f.panel.read(cx).current().unwrap());
    let session = cx.update(|cx| thread.read(cx).session().clone().unwrap());
    if first_question {
        thread.update(cx, |thread, cx| thread.send("First question".into(), cx));
        cx.run_until_parked();
    } else {
        deliver(
            f,
            &session,
            SessionUpdate::ToolStarted(tool("leading", "Leading tool")),
            cx,
        );
    }
    let text = |text: String| {
        Content::Text(Text {
            text,
            annotations: None,
            extensions: None,
        })
    };
    for update in [
        SessionUpdate::ThoughtChunk(text("Reasoning before answer".into())),
        SessionUpdate::ToolStarted(tool("one", "Inspect terminal")),
        SessionUpdate::AgentChunk(text(
            "A tall first answer with wrapped text.\n\n".repeat(80),
        )),
        SessionUpdate::ThoughtChunk(text("Reasoning between messages".into())),
    ] {
        deliver(f, &session, update, cx);
    }
    if first_question {
        complete_active(f, cx);
    }
    thread.update(cx, |thread, cx| {
        thread.send(
            "Second question with several paragraphs.\n\n".repeat(80),
            cx,
        )
    });
    cx.run_until_parked();
    for update in [
        SessionUpdate::ThoughtChunk(text("Additional reasoning".into())),
        SessionUpdate::AgentChunk(text(
            "A tall second answer with wrapped text.\n\n".repeat(80),
        )),
        SessionUpdate::ToolStarted(tool("two", "Inspect again")),
    ] {
        deliver(f, &session, update, cx);
    }
    complete_active(f, cx);
    thread.update(cx, |thread, cx| {
        for index in 0..thread.history_rows().len() {
            thread.mark_dirty(index);
        }
        cx.notify();
    });
    cx.run_until_parked();
    cx.update(|cx| cx.set_reduce_motion(true));
    settle(f, cx);
    thread
}

fn settle(f: &Fixture, cx: &mut TestAppContext) {
    for _ in 0..3 {
        cx.update_window(f.handle, |_, window, cx| window.render_frame(cx))
            .unwrap();
        cx.run_until_parked();
    }
}

#[gpui_kit::test]
fn scrolled_chat_exposes_previous_and_first_controls_alongside_existing_latest(
    cx: &mut TestAppContext,
) {
    let f = fixture(cx);
    transcript(&f, cx);
    cx.update_window(f.handle, |_, window, cx| {
        window.render_frame(cx);
        window.scroll(
            "agent-transcript-area",
            gpui_kit::ScrollDelta::Pixels(point(px(0.), px(500.))),
            cx,
        );
        window.render_frame(cx);
        assert!(
            window.try_find("agent-message-navigation").is_some(),
            "scrolled chat is missing the message-navigation pill"
        );
        assert!(window.find("agent-previous-message").visible());
        assert!(window.find("agent-first-message").visible());
        assert!(
            window
                .find((
                    gpui_kit::ElementId::from("agent-transcript"),
                    "jump-to-latest"
                ))
                .visible()
        );
    })
    .unwrap();
}

pub(super) fn scroll_to(f: &Fixture, index: usize, cx: &mut TestAppContext) {
    let mut previous = None;
    let mut unchanged = 0;
    for _ in 0..1000 {
        let (done, rows, viewport) = cx
            .update_window(f.handle, |_, window, cx| {
                window.render_frame(cx);
                let viewport = window.find("agent-transcript-area").bounds();
                let count = f
                    .panel
                    .read(cx)
                    .current()
                    .unwrap()
                    .read(cx)
                    .history_rows()
                    .len();
                let rows: Vec<_> = (0..count)
                    .filter_map(|row| {
                        let bounds = window.try_find(("agent-entry", row))?.bounds();
                        (bounds.bottom() > viewport.top() && bounds.top() < viewport.bottom())
                            .then_some((row, bounds.top(), bounds.bottom()))
                    })
                    .collect();
                let top = viewport.top() + px(8.);
                let delta = window
                    .try_find(("agent-entry", index))
                    .map(|row| top - row.bounds().top())
                    .unwrap_or_else(|| {
                        let direction = if rows.first().is_some_and(|row| index < row.0) {
                            1.
                        } else {
                            -1.
                        };
                        viewport.size.height * (0.7 * direction)
                    });
                if delta.abs() <= px(0.5) {
                    return (true, rows, viewport);
                }
                window.scroll(
                    "agent-transcript-area",
                    gpui_kit::ScrollDelta::Pixels(point(px(0.), delta)),
                    cx,
                );
                (false, rows, viewport)
            })
            .unwrap();
        if done {
            return;
        }
        if previous.as_ref() == Some(&rows) {
            unchanged += 1;
        } else {
            unchanged = 0;
        }
        assert!(
            unchanged < 3,
            "native scroll made no progress toward entry {index}: visible {rows:?}, viewport {viewport:?}"
        );
        previous = Some(rows);
        cx.run_until_parked();
    }
    panic!("native wheel did not reveal transcript entry {index}");
}

fn top(f: &Fixture, cx: &mut TestAppContext) -> ListOffset {
    cx.update_window(f.handle, |_, window, cx| {
        window.render_frame(cx);
        let top = window.find("agent-transcript-area").bounds().top() + px(8.);
        let count = f
            .panel
            .read(cx)
            .current()
            .unwrap()
            .read(cx)
            .history_rows()
            .len();
        (0..count)
            .find_map(|index| {
                let row = window.try_find(("agent-entry", index))?;
                (row.bounds().bottom() > top).then_some(ListOffset {
                    item_ix: index,
                    offset_in_item: (top - row.bounds().top()).max(px(0.)),
                })
            })
            .expect("a rendered message intersects the viewport")
    })
    .unwrap()
}
fn click(f: &Fixture, button: &str, cx: &mut TestAppContext) {
    cx.update_window(f.handle, |_, window, cx| {
        window.render_frame(cx);
        window.click(button.to_string(), cx);
        window.render_frame(cx);
    })
    .unwrap();
    cx.run_until_parked();
}

#[gpui_kit::test]
fn previous_rewinds_a_partial_tall_user_message_then_skips_all_other_rows(cx: &mut TestAppContext) {
    let f = fixture(cx);
    transcript(&f, cx);
    scroll_to(&f, 5, cx);
    cx.update_window(f.handle, |_, window, cx| {
        window.scroll(
            "agent-transcript-area",
            gpui_kit::ScrollDelta::Pixels(point(px(0.), px(-80.))),
            cx,
        );
        window.render_frame(cx);
    })
    .unwrap();
    let partial = top(&f, cx);
    assert_eq!(partial.item_ix, 5);
    assert!(
        partial.offset_in_item > px(0.),
        "wheel must leave viewport within tall message"
    );
    click(&f, "agent-previous-message", cx);
    assert_eq!(top(&f, cx).item_ix, 5);
    assert_eq!(top(&f, cx).offset_in_item, px(0.));
    click(&f, "agent-previous-message", cx);
    assert_eq!(
        top(&f, cx).item_ix,
        0,
        "previous user message must skip assistant/content/tool/thought rows"
    );
    assert_eq!(top(&f, cx).offset_in_item, px(0.));
    click(&f, "agent-previous-message", cx);
    click(&f, "agent-first-message", cx);
    assert_eq!(top(&f, cx).item_ix, 0);
    assert!(
        cx.update_window(f.handle, |_, window, _| window
            .try_find("agent-message-navigation")
            .is_some())
            .unwrap()
    );
}

#[gpui_kit::test]
fn previous_from_assistant_first_and_latest_keep_distinct_targets_and_follow_mode(
    cx: &mut TestAppContext,
) {
    let f = fixture(cx);
    transcript(&f, cx);
    scroll_to(&f, 7, cx);
    cx.update_window(f.handle, |_, window, cx| {
        window.scroll(
            "agent-transcript-area",
            gpui_kit::ScrollDelta::Pixels(point(px(0.), px(-80.))),
            cx,
        );
        window.render_frame(cx);
    })
    .unwrap();
    assert_eq!(top(&f, cx).item_ix, 7);
    assert!(top(&f, cx).offset_in_item > px(0.));
    click(&f, "agent-previous-message", cx);
    assert_eq!(
        top(&f, cx).item_ix,
        5,
        "assistant and Content entries must be skipped"
    );
    click(&f, "agent-first-message", cx);
    assert_eq!(top(&f, cx).item_ix, 0);
    cx.update_window(f.handle, |_, window, cx| {
        assert!(window.try_find("agent-message-navigation").is_some());
        window.click(
            (
                gpui_kit::ElementId::from("agent-transcript"),
                "jump-to-latest",
            ),
            cx,
        );
        window.render_frame(cx);
        assert!(window.try_find("agent-message-navigation").is_none());
        assert!(window.try_find("agent-message-navigation").is_none());
    })
    .unwrap();
}

#[gpui_kit::test]
fn streaming_and_new_messages_preserve_reader_anchor_and_new_chat_clears_navigation(
    cx: &mut TestAppContext,
) {
    let f = fixture(cx);
    let thread = transcript(&f, cx);
    scroll_to(&f, 5, cx);
    cx.update_window(f.handle, |_, window, cx| {
        window.scroll(
            "agent-transcript-area",
            gpui_kit::ScrollDelta::Pixels(point(px(0.), px(-80.))),
            cx,
        );
    })
    .unwrap();
    cx.run_until_parked();
    click(&f, "agent-previous-message", cx);
    let before = top(&f, cx);
    let session = cx.update(|cx| thread.read(cx).session().clone().unwrap());
    deliver(
        &f,
        &session,
        SessionUpdate::AgentChunk(content("New streamed text. ".repeat(50))),
        cx,
    );
    thread.update(cx, |thread, cx| {
        thread.send("Third question".into(), cx);
    });
    cx.run_until_parked();
    cx.update_window(f.handle, |_, window, cx| window.render_frame(cx))
        .unwrap();
    assert_eq!(top(&f, cx).item_ix, before.item_ix);
    assert_eq!(top(&f, cx).offset_in_item, before.offset_in_item);
    assert!(
        cx.update_window(f.handle, |_, window, _| window
            .try_find("agent-message-navigation")
            .is_some())
            .unwrap()
    );
    click(&f, "agent-previous-message", cx);
    assert_eq!(top(&f, cx).item_ix, 0);
    new_chat(&f, cx);
    cx.update_window(f.handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("agent-message-navigation").is_none());
        assert!(window.try_find(("agent-entry", 0usize)).is_none());
        assert!(window.try_find("agent-message-navigation").is_none());
    })
    .unwrap();
}

#[gpui_kit::test]
fn navigation_pill_fits_narrow_chat_in_both_themes_and_keeps_rows_virtual(cx: &mut TestAppContext) {
    let f = fixture_with_width(cx, 14., usage_source(), workspace_resolver(), image_deps());
    let thread = transcript(&f, cx);
    thread.update(cx, |thread, cx| {
        let entries = (0..500)
            .map(|i| {
                Entry::Tool(tool(
                    &format!("extra-{i}"),
                    &format!("Additional message {i}"),
                ))
            })
            .collect();
        restore_entries(&f, thread, entries, cx);
        cx.notify();
    });
    for mode in [
        nocterm_ui::AppearanceMode::Dark,
        nocterm_ui::AppearanceMode::Light,
    ] {
        cx.update(|cx| {
            f.installation
                .ready
                .settings()
                .update::<nocterm_ui::AppearanceSettings>(cx, move |settings| settings.mode = mode)
                .detach()
        });
        cx.run_until_parked();
        scroll_to(&f, 5, cx);
        cx.update_window(f.handle, |_, window, cx| {
            window.render_frame(cx);
            let pill = window.find("agent-message-navigation").bounds();
            let transcript = window.find("agent-transcript-area").bounds();
            assert!(pill.left() >= transcript.left());
            assert!(
                pill.right() <= transcript.right() - px(8.),
                "pill overlaps the scrollbar"
            );
            assert!(pill.top() >= transcript.top());
            assert!(pill.bottom() < transcript.bottom());
            assert!(pill.size.width <= px(44.));
            let user = window.find(("agent-user-bubble", 5usize)).bounds();
            assert!(
                user.right() <= pill.left() - px(4.),
                "navigation overlaps readable message content"
            );
            let latest = window
                .find((
                    gpui_kit::ElementId::from("agent-transcript"),
                    "jump-to-latest",
                ))
                .bounds();
            assert!(
                latest.right() < pill.left(),
                "navigation overlaps the latest control"
            );
            let previous = window.find("agent-previous-message").bounds();
            let first = window.find("agent-first-message").bounds();
            assert!(
                first.top() < previous.top(),
                "double-up must be the upper button"
            );
            assert!(
                (previous.center().y - latest.center().y).abs() <= px(2.),
                "lower navigation button must align with latest"
            );
            assert!(
                window.try_find(("agent-entry", 508usize)).is_none(),
                "offscreen row was eagerly rendered"
            );
        })
        .unwrap();
    }
}

#[gpui_kit::test]
fn top_of_chat_includes_leading_tools_before_the_first_user_message(cx: &mut TestAppContext) {
    let f = fixture(cx);
    conversation(&f, false, cx);
    scroll_to(&f, 7, cx);
    click(&f, "agent-previous-message", cx);
    assert_eq!(top(&f, cx).item_ix, 5);
    click(&f, "agent-previous-message", cx);
    assert_eq!(
        top(&f, cx).item_ix,
        5,
        "previous is disabled at the first user message"
    );
    click(&f, "agent-first-message", cx);
    assert_eq!(
        top(&f, cx).item_ix,
        0,
        "chat top must include the leading tool"
    );
    assert_eq!(top(&f, cx).offset_in_item, px(0.));
}
#[gpui_kit::test]
fn copy_completed_message_uses_full_text_while_reveal_is_pending(cx: &mut TestAppContext) {
    let f = fixture(cx);
    new_chat(&f, cx);
    cx.update_window(f.handle, |_, window, cx| window.render_frame(cx))
        .unwrap();
    let thread = cx.update(|cx| f.panel.read(cx).current().unwrap());
    thread.update(cx, |thread, cx| thread.send("question".into(), cx));
    cx.run_until_parked();
    cx.update_window(f.handle, |_, window, cx| window.render_frame(cx))
        .unwrap();
    let full = "Ответ 👨‍👩‍👧‍👦 世界 ".repeat(1000);
    let session = cx.update(|cx| thread.read(cx).session().clone().unwrap());
    f.events
        .try_send(AgentEvent::Session(SessionNotice {
            session,
            update: SessionUpdate::AgentChunk(Content::Text(Text {
                text: full.clone(),
                annotations: None,
                extensions: None,
            })),
        }))
        .unwrap();
    cx.run_until_parked();
    cx.update_window(f.handle, |_, window, cx| window.render_frame(cx))
        .unwrap();
    complete_active(&f, cx);
    let before = cx
        .update_window(f.handle, |_, window, cx| {
            window.render_frame(cx);
            let height = window.find(("agent-entry", 1usize)).bounds().size.height;
            window.hover(("agent-entry", 1usize), cx);
            window.click(("copy-message", 1usize), cx);
            assert_eq!(cx.read_from_clipboard().unwrap().text().unwrap(), full);
            height
        })
        .unwrap();
    cx.executor()
        .advance_clock(std::time::Duration::from_millis(500));
    cx.run_until_parked();
    cx.update_window(f.handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(
            window.find(("agent-entry", 1usize)).bounds().size.height > before,
            "visible streamed text must continue revealing after copy"
        );
        window.hover(("agent-entry", 1usize), cx);
        window.click(("copy-message", 1usize), cx);
        assert_eq!(cx.read_from_clipboard().unwrap().text().unwrap(), full);
    })
    .unwrap();
}
#[gpui_kit::test]
fn streaming_keeps_reader_scroll_until_jump_to_latest(cx: &mut TestAppContext) {
    let f = fixture(cx);
    new_chat(&f, cx);
    cx.update(|cx| cx.set_reduce_motion(true));
    for index in 0..15 {
        exchange(
            &f,
            &format!("question {index}"),
            &"Long answer with wrapped text. ".repeat(30),
            cx,
        );
    }
    cx.update_window(f.handle, |_, window, cx| {
        window.render_frame(cx);
        window.scroll(
            "agent-transcript-area",
            gpui_kit::ScrollDelta::Pixels(gpui_kit::point(
                gpui_kit::px(0.),
                gpui_kit::px(1_000_000.),
            )),
            cx,
        );
        window.render_frame(cx);
        assert!(window.try_find("agent-message-navigation").is_some());
        assert!(window.find(("agent-entry", 0usize)).visible());
    })
    .unwrap();
    let thread = cx.update(|cx| f.panel.read(cx).current().unwrap());
    thread.update(cx, |thread, cx| thread.send("new question".into(), cx));
    cx.run_until_parked();
    let session = cx.update(|cx| thread.read(cx).session().clone().unwrap());
    f.events
        .try_send(AgentEvent::Session(SessionNotice {
            session,
            update: SessionUpdate::AgentChunk(Content::Text(Text {
                text: "new streamed answer".repeat(100),
                annotations: None,
                extensions: None,
            })),
        }))
        .unwrap();
    cx.run_until_parked();
    cx.update_window(f.handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("agent-message-navigation").is_some());
        assert!(window.find(("agent-entry", 0usize)).visible());
        window.click(
            (
                gpui_kit::ElementId::from("agent-transcript"),
                "jump-to-latest",
            ),
            cx,
        );
        window.render_frame(cx);
        assert!(window.try_find("agent-message-navigation").is_none());
    })
    .unwrap();
    complete_active(&f, cx);
}
