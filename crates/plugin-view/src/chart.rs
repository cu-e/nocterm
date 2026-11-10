//! A plugin's chart: recent values as a light fill under a line, the newest at the right end,
//! close to how the application draws its own graphs. It is built of plain boxes, one slot per
//! value, so the view crate paints nothing itself.

use gpui::{
    AnyElement, Hsla, InteractiveElement as _, IntoElement, ParentElement, Styled, div,
    prelude::FluentBuilder as _, px, relative,
};

use crate::frame::FrameKind;

/// The height of a chart in a panel, and of the line a chart is in a bar.
const PANEL_HEIGHT: f32 = 44.;
const BAR_HEIGHT: f32 = 16.;
/// The width of a chart in a status bar item or across a tab's strip.
const BAR_WIDTH: f32 = 56.;
/// Most values a chart in a bar shows: two pixels each.
const BAR_SLOTS: usize = 28;
/// How much of a series' colour its fill keeps.
const FILL: f32 = 0.18;

/// One series and the colour it is drawn in.
pub(crate) struct Line {
    pub(crate) values: Vec<u32>,
    pub(crate) color: Hsla,
}

/// The chart of `lines`, from 0 to `max` (the largest value when `None`), its width holding
/// `span` values (the longest series when `None`). A panel draws it full width in a light
/// frame; a bar draws a short line of its newest values.
pub(crate) fn chart(
    kind: FrameKind,
    lines: Vec<Line>,
    max: Option<u32>,
    span: Option<u16>,
    frame: Hsla,
) -> AnyElement {
    let longest = lines
        .iter()
        .map(|line| line.values.len())
        .max()
        .unwrap_or(0);
    let mut slots = span.map_or(longest, usize::from).max(1);
    if matches!(kind, FrameKind::Bar | FrameKind::Strip) {
        slots = slots.min(BAR_SLOTS);
    }
    let top = max
        .unwrap_or_else(|| {
            lines
                .iter()
                .flat_map(|line| line.values.iter().copied())
                .max()
                .unwrap_or(0)
        })
        .max(1);
    let panel = kind == FrameKind::Panel;
    div()
        .debug_selector(|| "plugin-chart".to_string())
        .relative()
        .flex_none()
        .overflow_hidden()
        .map(|chart| match kind {
            FrameKind::Panel | FrameKind::Rail => chart.w_full(),
            FrameKind::Bar | FrameKind::Strip => chart.w(px(BAR_WIDTH)),
        })
        .h(px(if panel { PANEL_HEIGHT } else { BAR_HEIGHT }))
        .when(panel, |chart| {
            chart
                .rounded_md()
                .border_1()
                .border_color(frame.opacity(0.4))
        })
        .children(lines.iter().map(|line| layer(line, slots, top)))
        .into_any_element()
}

/// One series over the whole chart: a slot per value, right-aligned, each filled to its value
/// with the line along its top.
fn layer(line: &Line, slots: usize, top: u32) -> AnyElement {
    let shown = &line.values[line.values.len().saturating_sub(slots)..];
    let width = relative(1. / slots as f32);
    div()
        .absolute()
        .inset_0()
        .flex()
        .flex_row()
        .items_end()
        .justify_end()
        .children(shown.iter().map(|value| {
            let height = (*value as f32 / top as f32).clamp(0., 1.);
            div()
                .flex_none()
                .w(width)
                .h(relative(height))
                .min_h(px(1.))
                .bg(line.color.opacity(FILL))
                .border_t_1()
                .border_color(line.color)
        }))
        .into_any_element()
}
