//! Chat controls use the existing virtual scroller instead of a second viewport.
use super::*;
use gpui_kit::{ListOffset, point, px};
use nocterm_ai::thread::Entry;

fn transcript(f: &Fixture, cx: &mut TestAppContext) -> Entity<crate::thread::AgentThread> {
    new_chat(f, cx);
    let thread = cx.update(|cx| f.panel.read(cx).current().unwrap());
    thread.update(cx, |thread, cx| {
        thread.state.entries = vec![
            Entry::User(vec![acp::ContentBlock::Text(acp::TextContent::new(
                "First question",
            ))]),
            Entry::Thought("Reasoning before answer".into()),
            Entry::Tool(acp::ToolCall::new("one", "Inspect terminal")),
            Entry::Agent("A tall first answer with wrapped text.\n\n".repeat(80)),
            Entry::Thought("Reasoning between messages".into()),
            Entry::User(vec![acp::ContentBlock::Text(acp::TextContent::new(
                "Second question with several paragraphs.\n\n".repeat(80),
            ))]),
            Entry::Content(acp::ContentBlock::Text(acp::TextContent::new(
                "Additional content",
            ))),
            Entry::Agent("A tall second answer with wrapped text.\n\n".repeat(80)),
            Entry::Agent("A tall final answer with wrapped text.\n\n".repeat(80)),
        ];
        for index in 0..thread.state.entries.len() {
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
        f.panel.read(cx).list.clone().update(cx, |list, cx| {
            assert!(list.scroll_to_item(5, cx));
        });
        window.render_frame(cx);
        assert!(f.panel.read(cx).list.read(cx).is_scrolled_up());
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

fn scroll_to(f: &Fixture, index: usize, cx: &mut TestAppContext) {
    cx.update_window(f.handle, |_, window, cx| {
        window.render_frame(cx);
        f.panel.read(cx).list.clone().update(cx, |list, cx| {
            assert!(list.scroll_to_item(index, cx));
        });
        window.render_frame(cx);
    })
    .unwrap();
}
fn top(f: &Fixture, cx: &mut TestAppContext) -> ListOffset {
    cx.update(|cx| f.panel.read(cx).list.read(cx).logical_scroll_top())
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
    assert!(!cx.update(|cx| f.panel.read(cx).list.read(cx).is_following_tail()));
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
        assert!(!f.panel.read(cx).list.read(cx).is_following_tail());
        window.click(
            (
                gpui_kit::ElementId::from("agent-transcript"),
                "jump-to-latest",
            ),
            cx,
        );
        window.render_frame(cx);
        assert!(f.panel.read(cx).list.read(cx).is_following_tail());
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
    let before = top(&f, cx);
    thread.update(cx, |thread, cx| {
        if let Entry::Agent(text) = &mut thread.state.entries[8] {
            text.push_str(&"New streamed text. ".repeat(50));
        }
        thread.mark_dirty(8);
        thread
            .state
            .entries
            .push(Entry::User(vec![acp::ContentBlock::Text(
                acp::TextContent::new("Third question"),
            )]));
        thread.mark_dirty(9);
        cx.notify();
    });
    cx.run_until_parked();
    cx.update_window(f.handle, |_, window, cx| window.render_frame(cx))
        .unwrap();
    assert_eq!(top(&f, cx).item_ix, before.item_ix);
    assert_eq!(top(&f, cx).offset_in_item, before.offset_in_item);
    assert!(!cx.update(|cx| f.panel.read(cx).list.read(cx).is_following_tail()));
    click(&f, "agent-previous-message", cx);
    assert_eq!(top(&f, cx).item_ix, 0);
    new_chat(&f, cx);
    cx.update_window(f.handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(f.panel.read(cx).list.read(cx).is_following_tail());
        assert_eq!(f.panel.read(cx).list.read(cx).item_count(), 0);
        assert!(window.try_find("agent-message-navigation").is_none());
    })
    .unwrap();
}

#[gpui_kit::test]
fn navigation_pill_fits_narrow_chat_in_both_themes_and_keeps_rows_virtual(cx: &mut TestAppContext) {
    let f = fixture_with_width(cx, 14.);
    let thread = transcript(&f, cx);
    thread.update(cx, |thread, cx| {
        thread
            .state
            .entries
            .extend((0..500).map(|i| Entry::Agent(format!("Additional message {i}"))));
        cx.notify();
    });
    for mode in [
        nocterm_ui::AppearanceMode::Dark,
        nocterm_ui::AppearanceMode::Light,
    ] {
        cx.update(|cx| {
            cx.update_setting::<nocterm_ui::AppearanceSettings>(move |settings| {
                settings.mode = mode
            })
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
    let thread = transcript(&f, cx);
    thread.update(cx, |thread, cx| {
        thread.state.entries[0] = Entry::Tool(acp::ToolCall::new("leading", "Leading tool"));
        thread.mark_dirty(0);
        cx.notify();
    });
    cx.run_until_parked();
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
