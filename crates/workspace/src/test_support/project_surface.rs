//! Shared outer workspace Item over an original supplied inner surface.
use crate::{Item, ItemEvent};
use gpui_kit::{
    App, Context, EventEmitter, FocusHandle, Focusable, IntoElement, Render, Window, div,
    prelude::*,
};

pub struct ProjectSurfaceProbe {
    surface: crate::ProjectSurface,
    _changes: gpui_kit::Subscription,
}
impl ProjectSurfaceProbe {
    pub fn new(
        factory: crate::ProjectSurfaceFactory,
        project: nocterm_project_model::ProjectId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<Self, crate::ContextSelectionError> {
        let surface = factory.create(project, cx.entity_id(), window, cx)?;
        let view = cx.weak_entity();
        let changes = surface.observe(cx, move |_, cx| {
            let _ = view.update(cx, |_, cx| cx.emit(ItemEvent::Changed));
        });
        Ok(Self {
            surface,
            _changes: changes,
        })
    }
    pub fn surface(&self) -> crate::ProjectSurface {
        self.surface.clone()
    }
}
impl EventEmitter<ItemEvent> for ProjectSurfaceProbe {}
impl Focusable for ProjectSurfaceProbe {
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        self.surface.focus_handle(cx)
    }
}
impl Render for ProjectSurfaceProbe {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .size_full()
            .min_h_0()
            .min_w_0()
            .child(self.surface.view())
    }
}
impl Item for ProjectSurfaceProbe {
    fn tab_title(&self, _: &App) -> gpui_kit::SharedString {
        "Workspace".into()
    }
    fn on_close(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.surface.close(window, cx);
    }
}
