//! Wheel routing and native scroll state for tool text inside the transcript.
use gpui_kit::{
    AnyElement, App, Axis, ElementId, IntoElement, RenderOnce, ScrollHandle, TestSupportExt as _,
    Window, component::scroll::ScrollableMask, div, prelude::*, px,
};

#[derive(IntoElement)]
pub(super) struct LiteralViewport {
    id: ElementId,
    content: AnyElement,
}

impl LiteralViewport {
    pub(super) fn new(id: impl Into<ElementId>, content: impl IntoElement) -> Self {
        Self {
            id: id.into(),
            content: content.into_any_element(),
        }
    }
}

impl RenderOnce for LiteralViewport {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        // GPUI releases this state when the section leaves the rendered tree.
        // Keep the same native offset through redraws, without a second owner
        // in the transcript or a handle recreated on every frame.
        let scroll = window
            .use_keyed_state(self.id.clone(), cx, |_, _| ScrollHandle::new())
            .read(cx)
            .clone();
        div()
            .id(self.id.clone())
            .test_support()
            .relative()
            .w_full()
            .min_w_0()
            .max_w_full()
            .overflow_hidden()
            .child(
                div()
                    .id((self.id.clone(), "content"))
                    .w_full()
                    .min_w_0()
                    .max_h(px(240.))
                    .overflow_hidden()
                    .track_scroll(&scroll)
                    .flex()
                    .items_start()
                    .child(self.content),
            )
            // Inner masks run before the transcript's capture-phase mask.
            // Vertical input chains to the transcript only at a text boundary;
            // horizontal input stays here instead of becoming vertical input.
            .child(ScrollableMask::new(Axis::Horizontal, &scroll).id(self.id.clone()))
            .child(ScrollableMask::new(Axis::Vertical, &scroll).id(self.id))
    }
}
