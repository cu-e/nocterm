//! The page list: which pages it shows, which entry stays highlighted, and the way back from a
//! page nested under another one.
use super::{PageBook, SettingsCatalog, SettingsView};
use crate::catalog::Nav;
use gpui_kit::{
    AnyElement, Context, IntoElement, TestSupportExt as _,
    component::{StyledExt as _, v_flex},
    div,
    prelude::*,
    px, rems,
};
use nocterm_ui::form;

#[derive(Clone, Copy)]
struct Parent {
    index: usize,
    anchor: &'static str,
}

/// The nesting of the catalog's pages, by page index.
pub(crate) struct PageNav {
    parents: Vec<Option<Parent>>,
}

impl PageNav {
    pub(crate) fn new(catalog: &SettingsCatalog) -> Self {
        let parents = catalog
            .pages()
            .iter()
            .map(|page| match page.nav() {
                Nav::Top => None,
                Nav::Under { parent, anchor } => catalog
                    .pages()
                    .iter()
                    .position(|candidate| candidate.spec().id == parent)
                    .map(|index| Parent { index, anchor }),
            })
            .collect();
        Self { parents }
    }

    pub(crate) fn render(
        &self,
        pages: &PageBook,
        selected: usize,
        cx: &mut Context<SettingsView>,
    ) -> impl IntoElement {
        let highlighted = self.parents[selected].map_or(selected, |parent| parent.index);
        v_flex()
            .id("settings-pages")
            .test_support()
            .w(rems(14.))
            .flex_shrink_0()
            .h_full()
            .p_2()
            .gap(px(2.))
            .child(
                div()
                    .px_3()
                    .py_2()
                    .text_base()
                    .font_semibold()
                    .child("Settings"),
            )
            .children(
                pages
                    .slots
                    .iter()
                    .enumerate()
                    .filter(|(index, _)| self.parents[*index].is_none())
                    .map(|(index, slot)| {
                        form::nav_item(
                            index,
                            slot.spec.icon,
                            slot.spec.title.clone(),
                            highlighted == index,
                            cx,
                        )
                        .test_support()
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.select_page(index, window, cx)
                        }))
                        .into_any_element()
                    }),
            )
    }

    /// "‹ Parent title" above a nested page; it selects the parent and reveals the anchor.
    pub(crate) fn back_link(
        &self,
        pages: &PageBook,
        selected: usize,
        cx: &mut Context<SettingsView>,
    ) -> Option<AnyElement> {
        let parent = self.parents[selected]?;
        let title = pages.slots[parent.index].spec.title.clone();
        Some(
            div()
                .id("settings-back")
                .test_support()
                .pb_2()
                .text_sm()
                .cursor_pointer()
                .child(format!("‹ {title}"))
                .on_click(cx.listener(move |this, _, window, cx| {
                    this.select_page(parent.index, window, cx);
                    this.reveal(parent.anchor, window, cx);
                }))
                .into_any_element(),
        )
    }
}
