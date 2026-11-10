//! Completion is a viewport-constrained popup, independent of the split pane's height.
use super::*;
use crate::ActiveDesign as _;
use gpui_kit::{
    AnyElement,
    base::{Align, POPUP_PRIORITY, Placement, Positioner},
    deferred, px, uniform_list,
};

impl PathInput {
    pub(super) fn render_popup(
        &self,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        if !self.popup_visible() {
            return None;
        }
        let bounds = self.input.read(cx).input_bounds();
        let viewport = window.viewport_size();
        let suggestions = self
            .completion
            .cycle()
            .map_or(0, |cycle| cycle.candidate_count());
        let available = (viewport.height - bounds.bottom()).max(bounds.top()) - px(14.);
        let width = bounds
            .size
            .width
            .max(px(120.))
            .min((viewport.width - px(16.)).max(px(0.)));
        let height = px(32. * suggestions.min(5) as f32).min((available - px(28.)).max(px(32.)));
        let surface = v_flex()
            .id(("path-completion-popup", cx.entity_id()))
            .test_support()
            .role(gpui_kit::Role::Group)
            .aria_label("Directory suggestions")
            .w(width)
            .overflow_hidden()
            .occlude()
            .border_1()
            .border_color(cx.theme().border)
            .rounded_sm()
            .bg(cx.theme().popover)
            .text_color(cx.theme().popover_foreground)
            .when(suggestions > 0, |popup| {
                popup.child(
                    uniform_list(
                        "path-suggestions",
                        suggestions,
                        cx.processor(|this, range: std::ops::Range<usize>, _, cx| {
                            range
                                .map(|index| this.render_suggestion(index, cx))
                                .collect::<Vec<_>>()
                        }),
                    )
                    .h(height)
                    .track_scroll(&self.scroll),
                )
            })
            .child(
                div()
                    .px_2()
                    .py_1()
                    .text_xs()
                    .whitespace_nowrap()
                    .truncate()
                    .text_color(cx.theme().muted_foreground)
                    .child(if self.completion.loading() {
                        "Loading suggestions…".to_owned()
                    } else if suggestions == 0 {
                        "No matching folders. Edit the path and press Tab.".to_owned()
                    } else {
                        format!("{suggestions} folders · ↑ / ↓ · Tab · Enter to open")
                    }),
            )
            .on_mouse_down_out(cx.listener(|this, _, _, cx| {
                this.invalidate(false);
                cx.notify();
            }));
        Some(
            deferred(
                Positioner::side(bounds)
                    .placement(Placement::Bottom)
                    .align(Align::Start)
                    .offset(px(6.))
                    .margin(px(8.))
                    .child(surface),
            )
            .with_priority(POPUP_PRIORITY)
            .into_any_element(),
        )
    }
    fn render_suggestion(&self, index: usize, cx: &mut Context<Self>) -> AnyElement {
        let Some(cycle) = self.completion.cycle() else {
            return div().into_any_element();
        };
        let Some(candidate) = cycle.candidate(index) else {
            return div().into_any_element();
        };
        div()
            .h(px(32.))
            .px_2()
            .flex()
            .items_center()
            .text_size(px(cx.design().typography.explorer_size.unwrap_or(12.)))
            .when(cycle.selected() == Some(index), |row| {
                row.bg(cx.theme().accent)
                    .text_color(cx.theme().accent_foreground)
            })
            .hover(|row| {
                row.bg(cx.theme().accent)
                    .text_color(cx.theme().accent_foreground)
            })
            .child(div().min_w_0().truncate().child(candidate.name.clone()))
            .id(("path-suggestion", index))
            .aria_label(candidate.name.clone())
            .aria_selected(cycle.selected() == Some(index))
            .test_support()
            .cursor_pointer()
            .on_mouse_down(
                gpui_kit::MouseButton::Left,
                cx.listener(move |this, _, window, cx| {
                    cx.stop_propagation();
                    this.choose(index, window, cx);
                }),
            )
            .into_any_element()
    }
}
