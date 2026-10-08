//! The context ring under the composer and the usage card it opens: how full
//! the context window is, the session's tokens and the plan's limits.

use gpui_kit::{
    App, ClickEvent, Context, Focusable as _, MouseButton, PathBuilder, TestSupportExt as _,
    Window, canvas,
    component::{
        ActiveTheme as _, Selectable as _, Sizable as _, StyledExt as _,
        button::{Button, ButtonVariants as _},
        h_flex, v_flex,
    },
    div, point,
    prelude::*,
    px, relative,
};
use nocterm_ai::acp;
use nocterm_ui::IconName;

use super::{AgentPanel, MenuKind};
use crate::runtime::Runtime;
use nocterm_ai::usage::{Breakdown, RateLimit};

/// The card's sections, gathered before anything is laid out.
struct Report {
    context: Option<acp::UsageUpdate>,
    breakdown: Breakdown,
    tokens: Option<acp::Usage>,
    limits: Vec<RateLimit>,
    /// Shown when no limits are known yet.
    limits_hint: Option<&'static str>,
}

impl AgentPanel {
    pub(super) fn dismiss_usage(&mut self, cx: &mut Context<Self>) -> bool {
        if self.menu != Some(MenuKind::Usage) {
            return false;
        }
        self.menu = None;
        cx.notify();
        true
    }

    pub(super) fn close_usage(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let restore = self.usage_focus.contains_focused(window, cx);
        self.dismiss_usage(cx);
        if restore {
            window.focus(&self.input.read(cx).focus_handle(cx), cx);
        }
    }

    pub(super) fn usage_button(
        &self,
        usage: Option<&acp::UsageUpdate>,
        cx: &mut Context<Self>,
    ) -> Button {
        // Match native popovers: toggle on mouse-down, using the rendered state.
        // Outside dismissal may already have run in this event's capture phase.
        let open = self.menu == Some(MenuKind::Usage);
        context_ring(usage, cx)
            .selected(open)
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, _, window, cx| {
                    this.toggle_usage(open, window, cx);
                }),
            )
            .on_click(cx.listener(move |this, event, window, cx| {
                if !matches!(event, ClickEvent::Mouse(_)) {
                    this.toggle_usage(open, window, cx);
                }
            }))
    }

    fn toggle_usage(&mut self, open: bool, window: &mut Window, cx: &mut Context<Self>) {
        if open {
            self.close_usage(window, cx);
        } else {
            self.refresh_usage(cx);
            self.menu = Some(MenuKind::Usage);
            // Read-only information must not interrupt composing.
            if !self.input.read(cx).focus_handle(cx).is_focused(window) {
                window.focus(&self.usage_focus, cx);
            }
            cx.notify();
        }
    }

    /// Reads the plan limits again when the card opens.
    pub(super) fn refresh_usage(&mut self, cx: &mut Context<Self>) {
        if let Some(thread) = self.current() {
            let agent = thread.read(cx).agent_id.clone();
            Runtime::global(cx).update(cx, |runtime, cx| runtime.refresh_limits(&agent, cx));
        }
    }

    fn usage_report(&self, cx: &App) -> Option<Report> {
        let thread = self.current()?;
        let thread = thread.read(cx);
        let context = thread.state.usage.clone().filter(|usage| usage.size > 0);
        let breakdown = context
            .as_ref()
            .map(|usage| Breakdown::estimate(&thread.state.entries, usage.used))
            .unwrap_or_default();
        let limits: Vec<RateLimit> = Runtime::global(cx)
            .read(cx)
            .limits(&thread.agent_id)
            .map(|limits| limits.iter().cloned().collect())
            .unwrap_or_default();
        let limits_hint = match thread.agent_id.as_str() {
            "claude" if limits.is_empty() => Some("Claude reports plan limits after a reply."),
            "codex" if limits.is_empty() => Some("Codex reports plan limits after a reply."),
            _ => None,
        };
        Some(Report {
            context,
            breakdown,
            tokens: thread.state.tokens.clone(),
            limits,
            limits_hint,
        })
    }

    pub(super) fn render_usage(&mut self, cx: &mut Context<Self>) -> gpui_kit::AnyElement {
        let Some(report) = self.usage_report(cx) else {
            return div().into_any_element();
        };
        let muted = cx.theme().muted_foreground;
        let mut card = v_flex()
            .id("agent-usage")
            .test_support()
            .track_focus(&self.usage_focus)
            .tab_group()
            .key_context("Popover")
            .on_action(
                cx.listener(|this, _: &gpui_kit::base::actions::Cancel, window, cx| {
                    this.close_usage(window, cx);
                }),
            )
            .on_mouse_down_out(cx.listener(|this, _, _, cx| {
                this.dismiss_usage(cx);
            }))
            .occlude()
            .w_full()
            .p_3()
            .gap_3()
            .text_sm()
            .rounded(px(12.))
            .border_1()
            .border_color(cx.theme().border)
            .bg(cx.theme().popover)
            .text_color(cx.theme().popover_foreground)
            .shadow_lg()
            .child(
                h_flex()
                    .justify_between()
                    .child(div().font_medium().child("Context Usage"))
                    .child(
                        Button::new("agent-usage-close")
                            .ghost()
                            .xsmall()
                            .icon(IconName::X)
                            .tooltip("Close")
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.close_usage(window, cx);
                            })),
                    ),
            );

        let mut context = v_flex().gap_2();
        match &report.context {
            Some(usage) => {
                let size = usage.size as f32;
                let parts = [
                    (
                        "System prompt, tools & rules",
                        report.breakdown.agent,
                        cx.theme().chart_1,
                    ),
                    ("Messages", report.breakdown.messages, cx.theme().chart_2),
                    ("Reasoning", report.breakdown.reasoning, cx.theme().chart_3),
                    ("Tool calls", report.breakdown.tools, cx.theme().chart_4),
                ];
                context = context
                    .child(
                        h_flex()
                            .justify_between()
                            .child(format!("{}% Full", percent(usage.used as f32 / size)))
                            .child(div().text_color(muted).child(format!(
                                "~{} / {} Tokens",
                                compact(usage.used),
                                compact(usage.size)
                            ))),
                    )
                    .child(bar(
                        parts.map(|(_, tokens, color)| (tokens as f32 / size, color)),
                        cx,
                    ));
                for (label, tokens, color) in parts.into_iter().filter(|part| part.1 > 0) {
                    context = context.child(legend_row(label, compact(tokens), Some(color), cx));
                }
                context =
                    context.child(div().text_xs().text_color(muted).child(
                        "The agent reports the total; the split is estimated from this chat.",
                    ));
            }
            None => {
                context = context.child(
                    div()
                        .text_xs()
                        .text_color(muted)
                        .child("The agent has not reported its context window yet."),
                );
            }
        }
        card = card.child(context);

        if let Some(tokens) = &report.tokens {
            let rows = [
                ("Input", Some(tokens.input_tokens)),
                ("Cache read", tokens.cached_read_tokens),
                ("Cache write", tokens.cached_write_tokens),
                ("Output", Some(tokens.output_tokens)),
                ("Reasoning", tokens.thought_tokens),
                ("Total", Some(tokens.total_tokens)),
            ];
            card = card.child(
                section("Session tokens", cx).children(
                    rows.into_iter()
                        .filter_map(|(label, value)| Some((label, value?)))
                        .filter(|(_, value)| *value > 0)
                        .map(|(label, value)| legend_row(label, compact(value), None, cx)),
                ),
            );
        }
        if let Some(cost) = report
            .context
            .as_ref()
            .and_then(|usage| usage.cost.as_ref())
        {
            card = card.child(legend_row(
                "Session cost",
                format!("{:.2} {}", cost.amount, cost.currency),
                None,
                cx,
            ));
        }

        if !report.limits.is_empty() || report.limits_hint.is_some() {
            let mut limits = section("Plan limits", cx);
            let now = nocterm_ai::history::now();
            for limit in &report.limits {
                limits = limits.child(limit_row(limit, now, cx));
            }
            if let Some(hint) = report.limits_hint {
                limits = limits.child(div().text_xs().text_color(muted).child(hint));
            }
            card = card.child(limits);
        }
        card.into_any_element()
    }
}

fn section(title: &'static str, cx: &App) -> gpui_kit::Div {
    v_flex()
        .gap_1p5()
        .pt_2()
        .border_t_1()
        .border_color(cx.theme().border)
        .child(
            div()
                .text_xs()
                .font_semibold()
                .text_color(cx.theme().muted_foreground)
                .child(title),
        )
}

fn legend_row(
    label: &'static str,
    value: String,
    color: Option<gpui_kit::Hsla>,
    cx: &App,
) -> gpui_kit::Div {
    h_flex()
        .gap_2()
        .when_some(color, |row, color| {
            row.child(div().flex_shrink_0().size(px(10.)).rounded_xs().bg(color))
        })
        .child(div().flex_1().min_w_0().truncate().child(label))
        .child(
            div()
                .flex_shrink_0()
                .text_color(cx.theme().muted_foreground)
                .child(value),
        )
}

fn limit_row(limit: &RateLimit, now: u64, cx: &App) -> gpui_kit::Div {
    let color = if limit.reached || limit.used.is_some_and(|used| used >= 0.9) {
        cx.theme().danger
    } else if limit.used.is_some_and(|used| used >= 0.7) {
        cx.theme().warning
    } else {
        cx.theme().progress_bar
    };
    let state = match (limit.reached, limit.used) {
        (true, _) => "Limit reached".to_owned(),
        (false, Some(used)) => format!("{}% used", percent(used as f32)),
        (false, None) => "Within limit".to_owned(),
    };
    let resets = limit
        .resets_at
        .map(|at| format!("Resets {}", until(at.saturating_sub(now))));
    v_flex()
        .gap_1()
        .child(
            h_flex()
                .justify_between()
                .child(limit.window.label())
                .child(div().text_color(cx.theme().muted_foreground).child(state)),
        )
        .child(bar(
            [(
                if limit.reached {
                    1.
                } else {
                    limit.used.unwrap_or(0.) as f32
                },
                color,
            )],
            cx,
        ))
        .children(resets.map(|resets| {
            div()
                .text_xs()
                .text_color(cx.theme().muted_foreground)
                .child(resets)
        }))
}

/// A track filled with `parts`, each a share of the whole.
fn bar<const N: usize>(parts: [(f32, gpui_kit::Hsla); N], cx: &App) -> gpui_kit::Div {
    let mut remaining = 1f32;
    let mut track = h_flex()
        .w_full()
        .h(px(6.))
        .gap(px(1.))
        .rounded_full()
        .overflow_hidden()
        .bg(cx.theme().muted.blend(cx.theme().foreground.opacity(0.10)));
    for (share, color) in parts {
        let share = share.clamp(0., remaining);
        remaining -= share;
        if share > 0. {
            track = track.child(div().h_full().w(relative(share)).bg(color));
        }
    }
    track
}

/// A share as a whole percentage; a small share is never shown as nothing.
fn percent(share: f32) -> u32 {
    let percent = (share.clamp(0., 1.) * 100.).round() as u32;
    if percent == 0 && share > 0. {
        1
    } else {
        percent
    }
}

/// A token count as it is read: 455, 3.4K, 1.2M.
fn compact(tokens: u64) -> String {
    match tokens {
        0..1_000 => tokens.to_string(),
        1_000..1_000_000 => format!("{:.1}K", tokens as f64 / 1e3),
        _ => format!("{:.1}M", tokens as f64 / 1e6),
    }
}

/// When something `seconds` from now happens: "in 2h 13m".
fn until(seconds: u64) -> String {
    let minutes = seconds.div_ceil(60);
    match minutes {
        0 => "now".into(),
        1..60 => format!("in {minutes}m"),
        60..1440 => format!("in {}h {}m", minutes / 60, minutes % 60),
        _ => format!("in {}d {}h", minutes / 1440, minutes % 1440 / 60),
    }
}

pub(super) fn usage_fraction(usage: Option<&acp::UsageUpdate>) -> f32 {
    usage
        .filter(|usage| usage.size > 0)
        .map(|usage| (usage.used as f64 / usage.size as f64).clamp(0., 1.) as f32)
        .unwrap_or(0.)
}

/// The ring that shows how full the context window is and opens the card.
pub(super) fn context_ring(usage: Option<&acp::UsageUpdate>, cx: &App) -> Button {
    let fraction = usage_fraction(usage);
    let background = cx.theme().muted.blend(cx.theme().foreground.opacity(0.10));
    let foreground = if fraction >= 0.85 {
        cx.theme().warning
    } else {
        cx.theme().muted_foreground
    };
    let tooltip = usage
        .filter(|usage| usage.size > 0)
        .map(|usage| {
            format!(
                "Context {}% full · {} / {} tokens",
                percent(fraction),
                compact(usage.used),
                compact(usage.size)
            )
        })
        .unwrap_or_else(|| "Context usage".into());
    Button::new("agent-context-usage")
        .ghost()
        .small()
        .accessibility_label("Context usage")
        .tooltip(tooltip)
        .child(
            canvas(
                |_, _, _| (),
                move |bounds, _, window, _| {
                    for (portion, color) in [(1., background), (fraction, foreground)] {
                        if portion <= 0. {
                            continue;
                        }
                        let mut path = PathBuilder::stroke(px(2.));
                        let center = bounds.center();
                        let radius = bounds.size.width / 2. - px(2.);
                        let steps = (64. * portion).ceil() as usize;
                        for step in 0..=steps {
                            let angle = -std::f32::consts::FRAC_PI_2
                                + std::f32::consts::TAU * portion * step as f32 / steps as f32;
                            let position = point(
                                center.x + radius * angle.cos(),
                                center.y + radius * angle.sin(),
                            );
                            if step == 0 {
                                path.move_to(position);
                            } else {
                                path.line_to(position);
                            }
                        }
                        if let Ok(path) = path.build() {
                            window.paint_path(path, color);
                        }
                    }
                },
            )
            .w(px(16.))
            .h(px(16.)),
        )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbers_read_like_the_agents_own() {
        assert_eq!(compact(455), "455");
        assert_eq!(compact(3_400), "3.4K");
        assert_eq!(compact(256_000), "256.0K");
        assert_eq!(compact(1_200_000), "1.2M");
        assert_eq!(percent(0.104), 10);
        assert_eq!(percent(0.001), 1);
        assert_eq!(percent(0.), 0);
        assert_eq!(until(0), "now");
        assert_eq!(until(61), "in 2m");
        assert_eq!(until(2 * 3600 + 13 * 60), "in 2h 13m");
        assert_eq!(until(3 * 86_400 + 4 * 3600), "in 3d 4h");
    }
}
