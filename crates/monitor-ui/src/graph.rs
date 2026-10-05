//! Small drawings: level meters and time-based sparklines.

use gpui_kit::{
    Bounds, Hsla, PathBuilder, Pixels, Window, canvas,
    component::{ActiveTheme as _, StyledExt as _, h_flex},
    div, point,
    prelude::*,
    px, relative,
};
use nocterm_monitor::Series;

/// The colour of a level: calm, then a warning, then danger near the top.
pub(crate) fn level_color(percent: f32, calm: Hsla, cx: &gpui_kit::App) -> Hsla {
    let theme = cx.theme();
    if percent >= 90.0 {
        theme.danger
    } else if percent >= 75.0 {
        theme.warning
    } else {
        calm
    }
}

/// A horizontal bar filled to `percent`.
pub(crate) fn meter(percent: f32, color: Hsla, cx: &gpui_kit::App) -> impl IntoElement {
    let fill = (percent / 100.0).clamp(0.0, 1.0);
    div()
        .h(px(4.))
        .w_full()
        .rounded_full()
        .overflow_hidden()
        .bg(cx.theme().muted)
        .child(
            div()
                .h_full()
                .w(relative(fill))
                .rounded_full()
                .bg(level_color(percent, color, cx)),
        )
}

/// Vertical bars side by side, one per value, for the processor cores.
pub(crate) fn columns(values: &[f32], color: Hsla, cx: &gpui_kit::App) -> impl IntoElement {
    h_flex()
        .h(px(12.))
        .gap(px(1.))
        .items_end()
        .children(values.iter().map(|value| {
            div()
                .w(px(2.))
                .h(relative((value / 100.0).clamp(0.08, 1.0)))
                .bg(level_color(*value, color, cx))
        }))
}

/// One line of a sparkline.
pub(crate) struct Line {
    points: Vec<(f64, f32)>,
    color: Hsla,
}

impl Line {
    pub(crate) fn new(series: &Series, color: Hsla) -> Self {
        Self {
            points: series.points().collect(),
            color,
        }
    }
}

/// Lines over the last `span` seconds of the host's clock, newest at the
/// right edge. Time, not sample count, sets the x axis, so a slow stretch
/// before the details opened reads as the gap it was.
pub(crate) fn sparkline(
    lines: Vec<Line>,
    span: f64,
    ceiling: Option<f32>,
    cx: &gpui_kit::App,
) -> impl IntoElement {
    let grid = cx.theme().border.opacity(0.4);
    let now = lines
        .iter()
        .filter_map(|line| line.points.last().map(|(time, _)| *time))
        .fold(f64::MIN, f64::max);
    let top = ceiling.unwrap_or_else(|| {
        let highest = lines
            .iter()
            .flat_map(|line| line.points.iter().map(|(_, value)| *value))
            .fold(0.0, f32::max);
        (highest * 1.15).max(1024.0)
    });
    div()
        .w_full()
        .h(px(44.))
        .rounded_md()
        .border_1()
        .border_color(grid)
        .overflow_hidden()
        .child(
            canvas(
                |_, _, _| {},
                move |bounds, (), window, _| {
                    for line in &lines {
                        paint(line, bounds, now, span.max(1.0), top, window);
                    }
                },
            )
            .size_full(),
        )
}

fn paint(line: &Line, bounds: Bounds<Pixels>, now: f64, span: f64, top: f32, window: &mut Window) {
    let visible: Vec<_> = line
        .points
        .iter()
        .filter(|(time, _)| now - time <= span)
        .map(|(time, value)| {
            let x = bounds.right() - bounds.size.width * ((now - time) / span) as f32;
            let height = (value / top).clamp(0.0, 1.0);
            let y = bounds.bottom() - (bounds.size.height - px(2.)) * height - px(1.);
            point(x, y)
        })
        .collect();
    let (Some(first), Some(last)) = (visible.first(), visible.last()) else {
        return;
    };
    if visible.len() > 1 {
        let mut area = PathBuilder::fill();
        area.move_to(point(first.x, bounds.bottom()));
        for at in &visible {
            area.line_to(*at);
        }
        area.line_to(point(last.x, bounds.bottom()));
        area.close();
        if let Ok(path) = area.build() {
            window.paint_path(path, line.color.opacity(0.18));
        }
    }
    let mut stroke = PathBuilder::stroke(px(1.5));
    stroke.move_to(*first);
    for at in &visible[1..] {
        stroke.line_to(*at);
    }
    if visible.len() == 1 {
        stroke.line_to(point(first.x - px(1.), first.y));
    }
    if let Ok(path) = stroke.build() {
        window.paint_path(path, line.color);
    }
}

/// A small caption over a value, for the details' headline numbers.
pub(crate) fn figure(
    caption: &'static str,
    value: String,
    color: Option<Hsla>,
    cx: &gpui_kit::App,
) -> impl IntoElement {
    let theme = cx.theme();
    h_flex()
        .gap_1()
        .items_center()
        .when_some(color, |figure, color| {
            figure.child(div().size(px(6.)).rounded_full().bg(color))
        })
        .child(
            div()
                .text_xs()
                .text_color(theme.muted_foreground)
                .child(caption),
        )
        .child(div().text_xs().font_semibold().child(value))
}
