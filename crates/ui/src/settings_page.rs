//! Feature-owned pages embedded in the Settings view, without feature dependencies.
use gpui_kit::AppContext as _;
use gpui_kit::{
    AnyView, App, Context, Entity, FocusHandle, Focusable, Render, SharedString, Window,
};
use std::rc::Rc;

/// An independent settings page. Hidden and closing pages can clear sensitive drafts.
pub trait SettingsPage: Render + Focusable {
    fn flush(&mut self, _window: &mut Window, _cx: &mut Context<Self>) {}
    fn on_deactivate(&mut self, _window: &mut Window, _cx: &mut Context<Self>) {}
    fn on_close(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.on_deactivate(window, cx);
    }
    /// Brings the part of the page named by `anchor` into view and returns whether it knows
    /// the anchor. Pages without addressable parts keep the default.
    fn reveal(&mut self, _anchor: &str, _window: &mut Window, _cx: &mut Context<Self>) -> bool {
        false
    }
}

/// Type-erased page owned by its host Settings view.
pub trait SettingsPageHandle: 'static {
    fn view(&self) -> AnyView;
    fn focus_handle(&self, cx: &App) -> FocusHandle;
    fn flush(&self, window: &mut Window, cx: &mut App);
    fn deactivate(&self, window: &mut Window, cx: &mut App);
    fn close(&self, window: &mut Window, cx: &mut App);
    fn reveal(&self, anchor: &str, window: &mut Window, cx: &mut App) -> bool;
}

impl<T: SettingsPage> SettingsPageHandle for Entity<T> {
    fn view(&self) -> AnyView {
        self.clone().into()
    }
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        self.read(cx).focus_handle(cx)
    }
    fn flush(&self, window: &mut Window, cx: &mut App) {
        self.update(cx, |page, cx| page.flush(window, cx));
    }
    fn deactivate(&self, window: &mut Window, cx: &mut App) {
        self.update(cx, |page, cx| page.on_deactivate(window, cx));
    }
    fn close(&self, window: &mut Window, cx: &mut App) {
        self.update(cx, |page, cx| page.on_close(window, cx));
    }
    fn reveal(&self, anchor: &str, window: &mut Window, cx: &mut App) -> bool {
        self.update(cx, |page, cx| page.reveal(anchor, window, cx))
    }
}

type PageFactory =
    dyn Fn(SettingsPageContext, &mut Window, &mut App) -> Box<dyn SettingsPageHandle>;

/// A page factory supplied by the composition root.
#[derive(Clone)]
pub struct SettingsPageSpec {
    pub id: &'static str,
    pub title: SharedString,
    /// Shown beside the title in the page list.
    pub icon: crate::IconName,
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
            icon: crate::IconName::Wrench,
            factory: Rc::new(move |_, window, cx| Box::new(factory(window, cx))),
        }
    }

    /// Owner factories receive only the original settings and shared save feedback.
    pub fn contextual<T: SettingsPage>(
        id: &'static str,
        title: impl Into<SharedString>,
        factory: impl Fn(SettingsPageContext, &mut Window, &mut App) -> Entity<T> + 'static,
    ) -> Self {
        Self {
            id,
            title: title.into(),
            icon: crate::IconName::Wrench,
            factory: Rc::new(move |context, window, cx| Box::new(factory(context, window, cx))),
        }
    }
    pub fn create(
        &self,
        context: SettingsPageContext,
        window: &mut Window,
        cx: &mut App,
    ) -> Box<dyn SettingsPageHandle> {
        (self.factory)(context, window, cx)
    }
    pub fn with_icon(mut self, icon: crate::IconName) -> Self {
        self.icon = icon;
        self
    }

    /// Shows the page first and each section below it in the host's scroll. A section is
    /// created when the page first renders it; the page passes flush, hide and close on to the
    /// sections it created. `reveal("<key>")` scrolls to and focuses a section, and
    /// `reveal("<key>/<anchor>")` also asks the section to reveal `<anchor>`.
    pub fn with_sections(mut self, sections: Vec<EmbeddedPageSpec>) -> Self {
        if sections.is_empty() {
            return self;
        }
        let primary = self.factory.clone();
        self.factory = Rc::new(move |context, window, cx| {
            let page = primary(context.clone(), window, cx);
            let sections = EmbeddedPages::new(sections.clone());
            let sectioned = SectionedPage::new(page, sections, context.scroll());
            Box::new(cx.new(|_| sectioned))
        });
        self
    }
}

use crate::{EmbeddedPageSpec, EmbeddedPages, SectionedPage, SettingsPageContext};

#[cfg(test)]
mod tests {
    use super::*;
    use gpui_kit::{TestAppContext, WindowOptions, div};

    struct Page {
        focus: FocusHandle,
        anchors: Vec<&'static str>,
    }
    impl Focusable for Page {
        fn focus_handle(&self, _: &App) -> FocusHandle {
            self.focus.clone()
        }
    }
    impl Render for Page {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl gpui_kit::IntoElement {
            div()
        }
    }
    impl SettingsPage for Page {
        fn reveal(&mut self, anchor: &str, _: &mut Window, _: &mut Context<Self>) -> bool {
            self.anchors.contains(&anchor)
        }
    }
    struct Plain(FocusHandle);
    impl Focusable for Plain {
        fn focus_handle(&self, _: &App) -> FocusHandle {
            self.0.clone()
        }
    }
    impl Render for Plain {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl gpui_kit::IntoElement {
            div()
        }
    }
    impl SettingsPage for Plain {}

    #[gpui_kit::test]
    fn a_page_reveals_only_the_anchors_it_knows(cx: &mut TestAppContext) {
        let (window, _root) = cx.update(|cx| {
            gpui_kit::open_window(WindowOptions::default(), cx, |_, cx| {
                cx.new(|cx| Plain(cx.focus_handle()))
            })
            .unwrap()
        });
        cx.update_window(window, |_, window, cx| {
            let addressable: Box<dyn SettingsPageHandle> = Box::new(cx.new(|cx| Page {
                focus: cx.focus_handle(),
                anchors: vec!["monitor"],
            }));
            let plain: Box<dyn SettingsPageHandle> =
                Box::new(cx.new(|cx| Plain(cx.focus_handle())));
            assert!(addressable.reveal("monitor", window, cx));
            assert!(!addressable.reveal("elsewhere", window, cx));
            assert!(!plain.reveal("monitor", window, cx));
        })
        .unwrap();
    }
}
