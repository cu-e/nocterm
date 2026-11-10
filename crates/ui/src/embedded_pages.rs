//! A settings page that stacks pages other features own under its own content.
//!
//! The Appearance page is the user's one place for how the window looks; the editor of the
//! status bar and panels belongs to another crate. A [`SettingsPageSpec`](crate::SettingsPageSpec)
//! given sections with `with_sections` shows its own page first and each section below it,
//! inside the one scroll the Settings host provides.

use std::rc::Rc;

use gpui_kit::{
    AnyElement, App, Context, ElementId, Entity, FocusHandle, Focusable, InteractiveElement as _,
    ParentElement as _, Render, ScrollAnchor, SharedString, StatefulInteractiveElement as _,
    Styled as _, TestSupportExt as _, Window,
    component::{ActiveTheme as _, StyledExt as _, v_flex},
    div,
    prelude::*,
};

use crate::{SettingsPage, SettingsPageHandle};

type SectionFactory = dyn Fn(&mut Window, &mut App) -> Box<dyn SettingsPageHandle>;

/// A page the owner of a feature supplies to sit inside another page.
#[derive(Clone)]
pub struct EmbeddedPageSpec {
    /// Names the section in anchors: `reveal("<key>")` and `reveal("<key>/<inner anchor>")`.
    pub key: &'static str,
    /// The heading above the section.
    pub title: SharedString,
    factory: Rc<SectionFactory>,
}

impl EmbeddedPageSpec {
    pub fn new<T: SettingsPage>(
        key: &'static str,
        title: impl Into<SharedString>,
        factory: impl Fn(&mut Window, &mut App) -> Entity<T> + 'static,
    ) -> Self {
        Self {
            key,
            title: title.into(),
            factory: Rc::new(move |window, cx| Box::new(factory(window, cx))),
        }
    }
}

/// The sections of one page. Each is created when it is first needed, and is told to flush,
/// to deactivate and to close once for each time the page is.
pub struct EmbeddedPages {
    sections: Vec<Section>,
}

struct Section {
    spec: EmbeddedPageSpec,
    page: Option<Box<dyn SettingsPageHandle>>,
}

impl EmbeddedPages {
    pub fn new(specs: Vec<EmbeddedPageSpec>) -> Self {
        let sections = specs
            .into_iter()
            .map(|spec| Section { spec, page: None })
            .collect();
        Self { sections }
    }

    pub(crate) fn len(&self) -> usize {
        self.sections.len()
    }

    pub(crate) fn key(&self, index: usize) -> Option<&'static str> {
        self.sections.get(index).map(|section| section.spec.key)
    }

    pub(crate) fn position(&self, key: &str) -> Option<usize> {
        self.sections
            .iter()
            .position(|section| section.spec.key == key)
    }

    pub(crate) fn title(&self, index: usize) -> Option<SharedString> {
        self.sections
            .get(index)
            .map(|section| section.spec.title.clone())
    }

    /// The section, created now if this is the first time it is needed.
    pub fn page(
        &mut self,
        index: usize,
        window: &mut Window,
        cx: &mut App,
    ) -> Option<&dyn SettingsPageHandle> {
        let section = self.sections.get_mut(index)?;
        if section.page.is_none() {
            section.page = Some((section.spec.factory)(window, cx));
        }
        section.page.as_deref()
    }

    pub fn flush(&self, window: &mut Window, cx: &mut App) {
        for page in self.created() {
            page.flush(window, cx);
        }
    }

    pub fn deactivate(&self, window: &mut Window, cx: &mut App) {
        for page in self.created() {
            page.deactivate(window, cx);
        }
    }

    pub fn close(&self, window: &mut Window, cx: &mut App) {
        for page in self.created() {
            page.close(window, cx);
        }
    }

    fn created(&self) -> impl Iterator<Item = &dyn SettingsPageHandle> {
        self.sections
            .iter()
            .filter_map(|section| section.page.as_deref())
    }
}

/// A page and, below it, the sections that other features supply.
pub struct SectionedPage {
    primary: Box<dyn SettingsPageHandle>,
    sections: EmbeddedPages,
    anchors: Vec<ScrollAnchor>,
}

impl SectionedPage {
    /// `scroll` is the host's scroll handle, which the sections scroll to.
    pub fn new(
        primary: Box<dyn SettingsPageHandle>,
        sections: EmbeddedPages,
        scroll: &gpui_kit::ScrollHandle,
    ) -> Self {
        let anchors = (0..sections.len())
            .map(|_| ScrollAnchor::for_handle(scroll.clone()))
            .collect();
        Self {
            primary,
            sections,
            anchors,
        }
    }

    fn section(
        &mut self,
        index: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let title = self.sections.title(index)?;
        let key = self.sections.key(index)?;
        let view = self.sections.page(index, window, cx)?.view();
        let anchor = self.anchors.get(index).cloned();
        let divider = cx.theme().border.opacity(0.5);
        Some(
            v_flex()
                .id(ElementId::Name(format!("settings-section:{key}").into()))
                .aria_label(title.clone())
                .test_support()
                .anchor_scroll(anchor)
                .w_full()
                .mt_8()
                .pt_6()
                .border_t_1()
                .border_color(divider)
                .child(div().pb_2().text_xl().font_semibold().child(title))
                .child(view)
                .into_any_element(),
        )
    }
}

impl Focusable for SectionedPage {
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        self.primary.focus_handle(cx)
    }
}

impl SettingsPage for SectionedPage {
    fn flush(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.primary.flush(window, cx);
        self.sections.flush(window, cx);
    }

    fn on_deactivate(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.primary.deactivate(window, cx);
        self.sections.deactivate(window, cx);
    }

    fn on_close(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.primary.close(window, cx);
        self.sections.close(window, cx);
    }

    /// `"<key>"` scrolls to the section and focuses it; `"<key>/<inner>"` also asks the section
    /// to reveal `<inner>`. Anything else is the primary page's anchor.
    fn reveal(&mut self, anchor: &str, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let (key, inner) = match anchor.split_once('/') {
            Some((key, inner)) => (key, Some(inner)),
            None => (anchor, None),
        };
        let Some(index) = self.sections.position(key) else {
            return self.primary.reveal(anchor, window, cx);
        };
        let Some(section) = self.sections.page(index, window, cx) else {
            return false;
        };
        let focus = section.focus_handle(cx);
        let known = inner.is_none_or(|inner| section.reveal(inner, window, cx));
        if let Some(anchor) = self.anchors.get(index).cloned() {
            // The section may not have been laid out yet, so the scroll waits one more frame.
            window.on_next_frame(move |window, cx| anchor.scroll_to(window, cx));
        }
        window.focus(&focus, cx);
        cx.notify();
        known
    }
}

impl Render for SectionedPage {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let sections: Vec<AnyElement> = (0..self.sections.len())
            .filter_map(|index| self.section(index, window, cx))
            .collect();
        v_flex()
            .id("sectioned-page")
            .w_full()
            .child(self.primary.view())
            .children(sections)
    }
}

#[cfg(test)]
mod tests;
