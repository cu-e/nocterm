//! The line under a message: copy it, fork the chat from it, and when it
//! was sent, shown while the pointer is over the message.
use gpui_kit::{
    AnyElement, ClipboardItem, Context, EntityId, SharedString, TestSupportExt as _,
    component::{
        ActiveTheme as _, Sizable as _,
        button::{Button, ButtonVariants as _},
        h_flex,
    },
    div,
    prelude::*,
};
use nocterm_ai::{acp, thread::Entry};
use nocterm_ui::IconName;

use super::AgentPanel;

/// The group a message row joins, so its actions show on hover.
pub(super) const MESSAGE_GROUP: &str = "agent-message-row";

/// The text a message copies as.
pub(super) fn message_text(entry: &Entry) -> Option<String> {
    match entry {
        Entry::User(parts) => Some(
            parts
                .iter()
                .filter_map(|part| match part {
                    acp::ContentBlock::Text(text) => Some(text.text.as_str()),
                    _ => None,
                })
                .collect::<Vec<_>>()
                .join("\n"),
        ),
        Entry::Agent(text) => Some(text.clone()),
        _ => None,
    }
}

/// `seconds` since the epoch as local time: the clock time today, the date
/// as well on other days.
pub(super) fn sent_at(seconds: u64) -> String {
    use chrono::{Local, TimeZone as _};
    let Some(time) = Local.timestamp_opt(seconds as i64, 0).single() else {
        return String::new();
    };
    if time.date_naive() == Local::now().date_naive() {
        time.format("%H:%M").to_string()
    } else {
        time.format("%-d %b %Y, %H:%M").to_string()
    }
}

/// Markdown that shows `text` literally.
pub(super) fn escape_markdown(text: &str) -> String {
    let mut escaped = String::with_capacity(text.len());
    for ch in text.chars() {
        if ch.is_ascii_punctuation() {
            escaped.push('\\');
        }
        escaped.push(ch);
    }
    // A single newline is a soft break in Markdown; keep the user's lines.
    escaped.replace('\n', "  \n")
}

impl AgentPanel {
    pub(super) fn render_message_actions(
        &self,
        thread: EntityId,
        index: usize,
        text: String,
        time: Option<u64>,
        user: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let muted = cx.theme().muted_foreground;
        h_flex()
            .id(("message-actions", index))
            .test_support()
            .gap_1()
            .when(user, |row| row.flex_row_reverse())
            .opacity(0.)
            .group_hover(MESSAGE_GROUP, |row| row.opacity(1.))
            .child(
                Button::new(("copy-message", index))
                    .ghost()
                    .xsmall()
                    .icon(IconName::Copy)
                    .tooltip("Copy")
                    .on_click(move |_, _, cx| {
                        cx.write_to_clipboard(ClipboardItem::new_string(text.clone()))
                    }),
            )
            .child(
                Button::new(("fork-message", index))
                    .ghost()
                    .xsmall()
                    .icon(IconName::GitFork)
                    .tooltip("Fork chat from here")
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.fork_thread_at(thread, Some(index), window, cx)
                    })),
            )
            .when_some(time, |row, time| {
                row.child(
                    div()
                        .px_1()
                        .text_xs()
                        .text_color(muted)
                        .child(SharedString::from(sent_at(time))),
                )
            })
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::escape_markdown;

    #[test]
    fn escaped_text_keeps_punctuation_and_lines() {
        assert_eq!(escape_markdown("*a*\nb"), "\\*a\\*  \nb");
    }
}
