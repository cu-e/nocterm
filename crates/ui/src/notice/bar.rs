//! The footer line of a window's notices and the list it opens into.
use std::time::Duration;

use gpui_kit::{
    Anchor, Animation, AnimationExt as _, AnyElement, App, IntoElement, RenderOnce, SharedString,
    TestSupportExt as _, Window,
    component::{
        ActiveTheme as _, Icon, Selectable as _, Sizable as _, StyledExt as _,
        button::{Button, ButtonVariants as _},
        h_flex,
        popover::Popover,
        v_flex,
    },
    div, ease_in_out, ease_out_quint,
    prelude::*,
    px, rems,
};

use super::{Level, Notice, act, clear, dismiss, notices, set_expanded};
use crate::IconName;

/// The widest the line's text grows before it scrolls.
pub(super) const LINE_WIDTH: f32 = 340.;
/// Space between the end of scrolling text and its repeat.
const MARQUEE_GAP: f32 = 48.;
/// How fast scrolling text moves, in pixels a second.
const MARQUEE_SPEED: f32 = 40.;
/// How many times long text scrolls past before it rests at its start.
/// Bounded: a running animation redraws the whole window every frame.
const MARQUEE_PASSES: u32 = 3;
/// How often scrolling text moves; each step redraws the window.
const MARQUEE_FPS: f32 = 30.;

/// The newest notice on one line, with its icon, action and dismiss button,
/// and a bell that opens every notice of the window. Long text scrolls.
///
/// Place it in the window's footer; it renders nothing while there are no
/// notices.
#[derive(IntoElement, Default)]
pub struct NoticeBar;

impl NoticeBar {
    pub fn new() -> Self {
        Self
    }
}

fn icon(level: Level, cx: &App) -> Icon {
    let theme = cx.theme();
    match level {
        Level::Info => Icon::new(IconName::Info).text_color(theme.info),
        Level::Success => Icon::new(IconName::CircleCheck).text_color(theme.success),
        Level::Warning => Icon::new(IconName::TriangleAlert).text_color(theme.warning),
        Level::Error => Icon::new(IconName::CircleX).text_color(theme.danger),
    }
}

/// The notice as one line of text.
fn line_text(notice: &Notice) -> SharedString {
    let text = if notice.message.is_empty() {
        notice.title.to_string()
    } else {
        format!("{} — {}", notice.title, notice.message)
    };
    text.replace(['\n', '\r'], " ").into()
}

impl RenderOnce for NoticeBar {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let list = notices(window, cx);
        let total = list.notices.len();
        let expanded = list.expanded;
        let live = list
            .notices
            .iter()
            .rev()
            .find(|notice| notice.live)
            .cloned();
        h_flex()
            .id("notice-bar")
            .test_support()
            .min_w_0()
            .gap_1()
            .when_some(live, |bar, notice| bar.child(line(notice, window, cx)))
            .when(total > 0, |bar| bar.child(center(total, expanded)))
    }
}

fn line(notice: Notice, window: &mut Window, cx: &mut App) -> AnyElement {
    let id = notice.id;
    let text = line_text(&notice);
    h_flex()
        .id(("notice-line", id))
        .min_w_0()
        .gap_1()
        .child(icon(notice.level, cx).small())
        .child(marquee(id, text, window))
        .when_some(notice.action, |line, (label, _)| {
            line.child(
                Button::new("notice-action")
                    .outline()
                    .xsmall()
                    .label(label)
                    .on_click(move |_, window, cx| act(window, cx, id)),
            )
        })
        .child(
            Button::new("notice-dismiss")
                .ghost()
                .xsmall()
                .icon(IconName::Close)
                .tooltip("Dismiss")
                .on_click(move |_, window, cx| dismiss(window, cx, id)),
        )
        .with_animation(
            ("notice-enter", id),
            Animation::new(Duration::from_millis(250)).with_easing(ease_out_quint()),
            |line, delta| line.opacity(delta).ml(px(12.) * (1. - delta)),
        )
        .into_any_element()
}

/// `text` on one line; when it is wider than the line it scrolls past a few
/// times, then rests at its start. Clicking it opens the list.
fn marquee(id: u64, text: SharedString, window: &mut Window) -> AnyElement {
    let font_size = rems(0.75).to_pixels(window.rem_size());
    let run = window.text_style().to_run(text.len());
    let width = window
        .text_system()
        .shape_line(text.clone(), font_size, &[run], None)
        .width;
    let viewport = px(LINE_WIDTH);
    let container = div()
        .id("notice-text")
        .test_support()
        .min_w_0()
        .max_w(viewport)
        .overflow_hidden()
        .text_xs()
        .whitespace_nowrap()
        .cursor_pointer()
        .on_click(|_, window, cx| set_expanded(window, cx, true));
    if width <= viewport {
        return container.child(text).into_any_element();
    }
    let travel = width + px(MARQUEE_GAP);
    let passes = MARQUEE_PASSES as f32;
    let duration = Duration::from_secs_f32(f32::from(travel) / MARQUEE_SPEED * passes);
    container
        .w(viewport)
        .child(
            h_flex()
                .id("notice-marquee")
                .test_support()
                .gap(px(MARQUEE_GAP))
                .child(div().flex_none().child(text.clone()))
                .child(div().flex_none().child(text))
                .with_animation(
                    ("notice-marquee", id),
                    Animation::new(duration).with_max_fps(MARQUEE_FPS),
                    move |row, delta| row.ml(-(travel * (delta * passes).fract())),
                ),
        )
        .into_any_element()
}

/// The bell with the number of notices, opening the list above it.
fn center(total: usize, expanded: bool) -> impl IntoElement {
    Popover::new("notice-center")
        .anchor(Anchor::BottomRight)
        .trigger(
            Button::new("notice-toggle")
                .ghost()
                .xsmall()
                .icon(IconName::Bell)
                .label(total.to_string())
                .tooltip("Notifications")
                .selected(expanded),
        )
        .open(expanded)
        .on_open_change(|open, window, cx| set_expanded(window, cx, *open))
        .content(|_, window, cx| list(window, cx).into_any_element())
}

fn list(window: &mut Window, cx: &mut App) -> AnyElement {
    let items: Vec<Notice> = notices(window, cx).notices.iter().rev().cloned().collect();
    let items: Vec<AnyElement> = items.into_iter().map(|notice| item(notice, cx)).collect();
    v_flex()
        .id("notice-list")
        .test_support()
        .w(px(400.))
        .gap_2()
        .child(
            h_flex()
                .justify_between()
                .child(div().text_sm().font_semibold().child("Notifications"))
                .child(
                    Button::new("notice-clear")
                        .ghost()
                        .xsmall()
                        .label("Clear all")
                        .on_click(|_, window, cx| clear(window, cx)),
                ),
        )
        .child(
            v_flex()
                .id("notice-items")
                .max_h(px(420.))
                .overflow_y_scroll()
                .gap_1p5()
                .children(items),
        )
        .with_animation(
            "notice-list-enter",
            Animation::new(Duration::from_millis(200)).with_easing(ease_in_out),
            |list, delta| list.opacity(delta).mt(px(8.) * (1. - delta)),
        )
        .into_any_element()
}

fn item(notice: Notice, cx: &App) -> AnyElement {
    let theme = cx.theme();
    let id = notice.id;
    h_flex()
        .id(("notice-item", id))
        .test_support()
        .items_start()
        .gap_2()
        .p_2()
        .rounded(theme.radius)
        .border_1()
        .border_color(theme.border)
        .bg(theme.secondary)
        .when(!notice.live, |item| item.opacity(0.85))
        .child(div().pt_0p5().child(icon(notice.level, cx).small()))
        .child(
            v_flex()
                .flex_1()
                .min_w_0()
                .gap_0p5()
                .child(div().text_sm().font_semibold().child(notice.title.clone()))
                .when(!notice.message.is_empty(), |text| {
                    text.child(
                        div()
                            .text_xs()
                            .text_color(theme.muted_foreground)
                            .child(notice.message.clone()),
                    )
                })
                .when_some(notice.action, |text, (label, _)| {
                    text.child(
                        h_flex().pt_1().child(
                            Button::new(("notice-item-action", id))
                                .outline()
                                .xsmall()
                                .label(label)
                                .on_click(move |_, window, cx| act(window, cx, id)),
                        ),
                    )
                }),
        )
        .child(
            Button::new(("notice-item-dismiss", id))
                .ghost()
                .xsmall()
                .icon(IconName::Close)
                .tooltip("Dismiss")
                .on_click(move |_, window, cx| dismiss(window, cx, id)),
        )
        .into_any_element()
}
