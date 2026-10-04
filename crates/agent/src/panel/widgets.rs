//! Small pieces of the agent panel's look, shared by its parts.
use std::time::Duration;

use gpui_kit::{
    App, IntoElement, SharedString,
    component::{
        ActiveTheme as _, Sizable as _,
        button::{Button, ButtonCustomVariant, ButtonVariants as _},
        h_flex,
        shimmer::ShimmerText,
        text::{TextView, TextViewStyle},
    },
    div,
    prelude::*,
    px, rems,
};
use nocterm_ai::{acp, thread::Entry};
use nocterm_ui::IconName;

use crate::thread::Attachment;

// Inline Markdown image URLs never trigger a resource request. Images travel
// through explicit ACP image blocks and the same bounded validation as attachments.
pub(super) fn safe_markdown(text: &str) -> String {
    text.replace("![", "[").replace("<img", "&lt;img")
}

pub(super) fn mode_label(state: &nocterm_ai::thread::ThreadState) -> String {
    state
        .modes
        .as_ref()
        .and_then(|modes| {
            let selected = state
                .current_mode
                .as_ref()
                .unwrap_or(&modes.current_mode_id);
            modes
                .available_modes
                .iter()
                .find(|mode| &mode.id == selected)
                .map(|mode| mode.name.clone())
        })
        .unwrap_or_else(|| "Mode".into())
}
/// The workspace's tab strip height.
pub(super) const HEADER_HEIGHT: f32 = 32.;

pub(super) fn menu_row(
    id: impl Into<gpui_kit::ElementId>,
    label: impl Into<SharedString>,
    cx: &App,
) -> Button {
    menu_row_with(id, label, None, cx)
}
/// A menu row with `leading` in place of an icon, before the label.
pub(super) fn menu_row_with(
    id: impl Into<gpui_kit::ElementId>,
    label: impl Into<SharedString>,
    leading: Option<gpui_kit::AnyElement>,
    cx: &App,
) -> Button {
    let label = label.into();
    Button::new(id)
        .custom(menu_variant(cx))
        .small()
        .w_full()
        .accessibility_label(label.clone())
        .tooltip(label.clone())
        .children(leading)
        .child(div().w_full().min_w_0().truncate().text_left().child(label))
}
pub(super) fn relative_prompt_time(elapsed: Option<Duration>) -> String {
    let Some(elapsed) = elapsed else {
        return "No requests yet".into();
    };
    let elapsed = elapsed.as_secs();
    match elapsed {
        0..60 => "Just now".into(),
        60..120 => "1 minute ago".into(),
        120..3600 => format!("{} minutes ago", elapsed / 60),
        3600..7200 => "1 hour ago".into(),
        7200..86400 => format!("{} hours ago", elapsed / 3600),
        86400..172800 => "1 day ago".into(),
        _ => format!("{} days ago", elapsed / 86400),
    }
}

pub(super) fn menu_variant(cx: &App) -> ButtonCustomVariant {
    let surface = cx.theme().muted;
    ButtonCustomVariant::new(cx)
        .foreground(cx.theme().foreground)
        .hover(surface.blend(cx.theme().foreground.opacity(0.10)))
        .active(surface.blend(cx.theme().foreground.opacity(0.18)))
}
pub(super) fn attachment_icon(attachment: &Attachment) -> IconName {
    match attachment {
        Attachment::Group(_) => IconName::FolderTree,
        Attachment::Connection(_) => IconName::Server,
        Attachment::Terminal(_) => IconName::Terminal,
    }
}
/// A saved server's system icon, the size of a menu icon.
pub(super) fn server_image(icon: std::sync::Arc<gpui_kit::Image>) -> gpui_kit::AnyElement {
    gpui_kit::img(icon)
        .size_4()
        .flex_shrink_0()
        .into_any_element()
}
pub(super) fn flag_image(flag: std::sync::Arc<gpui_kit::Image>) -> impl IntoElement {
    div().flex_shrink_0().rounded_xs().overflow_hidden().child(
        gpui_kit::img(flag)
            .w(px(16.))
            .h(px(11.))
            .object_fit(gpui_kit::ObjectFit::Cover),
    )
}
pub(super) fn running_dot(cx: &App) -> impl IntoElement {
    div()
        .flex_shrink_0()
        .size(px(5.))
        .rounded_full()
        .bg(cx.theme().success)
}
pub(super) fn entry_is_live(entries: &[Entry], index: usize, generating: bool) -> bool {
    generating
        && match entries.get(index) {
            Some(Entry::Thought(_)) => index + 1 == entries.len(),
            Some(Entry::Tool(call)) => matches!(
                call.status,
                acp::ToolCallStatus::Pending | acp::ToolCallStatus::InProgress
            ),
            _ => false,
        }
}
pub(super) fn activity_text(
    id: impl Into<gpui_kit::ElementId>,
    text: String,
    live: bool,
    cx: &App,
) -> gpui_kit::AnyElement {
    let label = div().flex_1().min_w_0().max_w_full().truncate().text_sm();
    if live {
        label
            .child(
                ShimmerText::new(text)
                    .id(id)
                    .text_color(cx.theme().muted_foreground)
                    .highlight_color(cx.theme().foreground),
            )
            .into_any_element()
    } else {
        label.child(text).into_any_element()
    }
}
pub(super) fn chat_markdown(id: impl Into<gpui_kit::ElementId>, text: String) -> TextView {
    let mut code_block = gpui_kit::StyleRefinement::default().text_sm();
    code_block.overflow.x = Some(gpui_kit::Overflow::Scroll);
    TextView::markdown(id, text)
        .selectable(true)
        .text_sm()
        .min_w_0()
        .style(TextViewStyle {
            paragraph_gap: rems(0.5),
            heading_base_font_size: px(14.),
            code_block,
            ..Default::default()
        })
}

pub(super) fn disclosure_header(
    id: impl Into<gpui_kit::ElementId>,
    activity_id: impl Into<gpui_kit::ElementId>,
    icon: IconName,
    title: String,
    live: bool,
    expanded: bool,
    cx: &App,
) -> Button {
    Button::new(id)
        .text()
        .small()
        .w_full()
        .min_w_0()
        .max_w_full()
        .px_0()
        .accessibility_label(title.clone())
        .tooltip(title.clone())
        .child(
            h_flex()
                .w_full()
                .min_w_0()
                .justify_start()
                .gap_2()
                .child(gpui_kit::component::Icon::new(icon).small().flex_shrink_0())
                .child(activity_text(activity_id, title, live, cx))
                .child(
                    gpui_kit::component::Icon::new(if expanded {
                        IconName::ChevronDown
                    } else {
                        IconName::ChevronRight
                    })
                    .small()
                    .flex_shrink_0(),
                ),
        )
}
