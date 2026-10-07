//! Completion is a viewport-constrained popup, independent of the split pane's height.
use super::*;
use gpui_kit::{
    AnyElement,
    base::{Align, POPUP_PRIORITY, Placement, Positioner},
    deferred, px, uniform_list,
};
use nocterm_ui::ActiveDesign as _;

impl PathInput {
    pub(super) fn render_popup(
        &self,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        if !self.editing || (self.pending.is_none() && self.cycle.is_none()) {
            return None;
        }
        let bounds = self.input.read(cx).input_bounds();
        let viewport = window.viewport_size();
        let suggestions = self
            .cycle
            .as_ref()
            .map_or(0, |cycle| cycle.candidates.len());
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
                                .map(|index| {
                                    let cycle =
                                        this.cycle.as_ref().expect("suggestions have a cycle");
                                    let candidate = &cycle.candidates[index];
                                    super::super::row::ExplorerRow {
                                        name: candidate.name.clone(),
                                        directory: candidate.directory,
                                        symlink: false,
                                        selected: cycle.selected == Some(index),
                                        font_size: cx
                                            .design()
                                            .typography
                                            .explorer_size
                                            .unwrap_or(12.),
                                    }
                                    .render(false, cx)
                                    .id(("path-suggestion", index))
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
                                })
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
                    .child(if self.pending.is_some() {
                        "Loading suggestions…".to_owned()
                    } else if suggestions == 0 {
                        "No matching files or folders. Edit the path and press Tab.".to_owned()
                    } else {
                        format!("{suggestions} matches · Tab / Shift+Tab · Enter to open")
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
}
