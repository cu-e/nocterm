//! Feature-owned pages embedded in the Settings Item, without feature dependencies.
use gpui_kit::{
    AnyView, App, Context, Entity, FocusHandle, Focusable, Render, SharedString, Window,
};
use std::rc::Rc;

/// An independent settings page. Hidden and closing pages can clear sensitive drafts.
pub trait SettingsPage: Render + Focusable {
    fn on_deactivate(&mut self, _window: &mut Window, _cx: &mut Context<Self>) {}
    fn on_close(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.on_deactivate(window, cx);
    }
}
/// Type-erased page owned by its host Settings Item.
pub trait SettingsPageHandle: 'static {
    fn view(&self) -> AnyView;
    fn focus_handle(&self, cx: &App) -> FocusHandle;
    fn deactivate(&self, window: &mut Window, cx: &mut App);
    fn close(&self, window: &mut Window, cx: &mut App);
}
impl<T: SettingsPage> SettingsPageHandle for Entity<T> {
    fn view(&self) -> AnyView {
        self.clone().into()
    }
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        self.read(cx).focus_handle(cx)
    }
    fn deactivate(&self, window: &mut Window, cx: &mut App) {
        self.update(cx, |page, cx| page.on_deactivate(window, cx));
    }
    fn close(&self, window: &mut Window, cx: &mut App) {
        self.update(cx, |page, cx| page.on_close(window, cx));
    }
}
type PageFactory = dyn Fn(&mut Window, &mut App) -> Box<dyn SettingsPageHandle>;
/// A lazy page factory supplied by the composition root.
#[derive(Clone)]
pub struct SettingsPageSpec {
    pub id: &'static str,
    pub title: SharedString,
    factory: Rc<PageFactory>,
}
impl SettingsPageSpec {
    pub fn new<T: SettingsPage>(
        id: &'static str,
        title: impl Into<SharedString>,
        factory: impl Fn(&mut Window, &mut App) -> Entity<T> + 'static,
    ) -> Self {
        Self {
            id,
            title: title.into(),
            factory: Rc::new(move |window, cx| Box::new(factory(window, cx))),
        }
    }
    pub fn create(&self, window: &mut Window, cx: &mut App) -> Box<dyn SettingsPageHandle> {
        (self.factory)(window, cx)
    }
}
