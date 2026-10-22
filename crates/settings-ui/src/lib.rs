//! A single settings tab, using the same schema and store as the terminal.
//!
//! Every control saves as soon as it changes; text saves when typing pauses,
//! on Enter and when the field loses focus. There is no draft and no Apply.
//! Owns: checked ordered page catalogs, Item navigation, shared diagnostics and eager/lazy lifetimes.
//! Does not own: page controls, schema policy, native probing or settings persistence.
//! Ports used: Installed supplies SettingsHandle; SettingsCatalog supplies owner factories.
//! Ports provided: workspace settings Item. Settings section: none. Globals: none.
//! Invariants: catalogs validate before factories run; eager construction/flush preserve order; lazy pages are reused.

mod installation;
pub use installation::Installed;

use gpui_kit::{
    AnyElement, App, Context, EventEmitter, FocusHandle, Focusable, SharedString, Subscription,
    TestSupportExt as _, Window,
    component::{StyledExt as _, h_flex, v_flex},
    div,
    prelude::*,
    px, rems,
};
use nocterm_ui::{IconName, form};
use nocterm_ui::{SettingsPageContext, SettingsPageFeedback, SettingsPageHandle, SettingsPageSpec};
use nocterm_workspace::{Item, ItemEvent, OpenSettings, Workspace};

mod catalog;
use catalog::Lifecycle;
pub use catalog::{SettingsCatalog, SettingsCatalogError, SettingsPageRegistration};

struct PageSlot {
    spec: SettingsPageSpec,
    lifecycle: Lifecycle,
    page: Option<Box<dyn SettingsPageHandle>>,
}
struct PageBook {
    slots: Vec<PageSlot>,
}
impl PageBook {
    fn new(
        catalog: &SettingsCatalog,
        context: SettingsPageContext,
        window: &mut Window,
        cx: &mut App,
    ) -> Self {
        let slots = catalog
            .pages()
            .iter()
            .map(|registration| {
                let spec = registration.spec().clone();
                let lifecycle = registration.lifecycle();
                let page = (lifecycle == Lifecycle::Eager)
                    .then(|| spec.create(context.clone(), window, cx));
                PageSlot {
                    spec,
                    lifecycle,
                    page,
                }
            })
            .collect();
        let mut book = Self { slots };
        book.activate(catalog.default_index(), context, window, cx);
        book
    }
    fn selected(&self, index: usize) -> Option<&dyn SettingsPageHandle> {
        self.slots.get(index)?.page.as_deref()
    }
    fn activate(
        &mut self,
        index: usize,
        context: SettingsPageContext,
        window: &mut Window,
        cx: &mut App,
    ) {
        let slot = &mut self.slots[index];
        if slot.page.is_none() {
            slot.page = Some(slot.spec.create(context, window, cx));
        }
    }
}
/// Installs the settings Item with factories supplied by the actual feature owners.
pub fn register(workspace: &mut Workspace, pages: SettingsCatalog, installed: Installed) {
    workspace.register_action(move |_, _: &OpenSettings, window, cx| {
        let workspace = cx.entity().downgrade();
        let pages = pages.clone();
        let installed = installed.clone();
        // Popup dismissal restores its old action context before the Item takes focus.
        window.defer(cx, move |window, cx| {
            let _ = workspace.update(cx, |workspace, cx| {
                open_page(workspace, "", &pages, installed, window, cx)
            });
        });
    });
}
/// Opens the singleton Settings Item; empty ID preserves its current selection.
pub fn open_page(
    workspace: &mut Workspace,
    id: &str,
    pages: &SettingsCatalog,
    installed: Installed,
    window: &mut Window,
    cx: &mut Context<Workspace>,
) {
    let item = workspace.find_item::<SettingsView>(cx).unwrap_or_else(|| {
        let item = cx.new(|cx| SettingsView::with_pages(pages.clone(), installed, window, cx));
        workspace.add_item(item.clone(), window, cx);
        item
    });
    if !id.is_empty() {
        item.update(cx, |view, cx| {
            if let Some(index) = view.page_index(id) {
                view.select_page(index, window, cx);
            }
        });
    }
    workspace.activate_item_by_id(item.entity_id(), window, cx);
}
pub struct SettingsView {
    installed: Installed,
    selected_page: usize,
    pages: PageBook,
    focus: FocusHandle,
    feedback: SettingsPageFeedback,
    _subscriptions: Vec<Subscription>,
}
impl SettingsView {
    pub fn with_pages(
        pages: SettingsCatalog,
        installed: Installed,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let feedback = SettingsPageFeedback::new(cx);
        let context = SettingsPageContext::new(installed.settings().clone(), feedback.clone());
        let weak = cx.weak_entity();
        let original_window = window.window_handle();
        let changed = feedback.observe(cx, move |cx| {
            let _ = original_window.update(cx, |_, _, cx| {
                let _ = weak.update(cx, |_, cx| cx.notify());
            });
        });
        let weak = cx.weak_entity();
        let original_window = window.window_handle();
        let settings = installed.settings().observe(cx, move |cx| {
            let _ = original_window.update(cx, |_, _, cx| {
                let _ = weak.update(cx, |_, cx| cx.notify());
            });
        });
        let selected_page = pages.default_index();
        let pages = PageBook::new(&pages, context, window, cx);
        Self {
            installed,
            selected_page,
            pages,
            focus: cx.focus_handle(),
            feedback,
            _subscriptions: vec![changed, settings],
        }
    }
    pub fn selected_page_id(&self) -> &str {
        self.pages.slots[self.selected_page].spec.id
    }
    fn page_index(&self, id: &str) -> Option<usize> {
        self.pages.slots.iter().position(|slot| slot.spec.id == id)
    }
    fn select_page(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        if index >= self.pages.slots.len() || index == self.selected_page {
            return;
        }
        self.commit_all(window, cx);
        if self.pages.slots[self.selected_page].lifecycle == Lifecycle::Lazy
            && let Some(page) = self.pages.selected(self.selected_page)
        {
            page.deactivate(window, cx);
        }
        self.selected_page = index;
        let context =
            SettingsPageContext::new(self.installed.settings().clone(), self.feedback.clone());
        self.pages.activate(index, context, window, cx);
        window.focus(&self.focus_handle(cx), cx);
        cx.notify();
    }
    fn commit_all(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        for slot in &self.pages.slots {
            if slot.lifecycle == Lifecycle::Eager
                && let Some(page) = &slot.page
            {
                page.flush(window, cx);
            }
        }
    }
    fn render_nav(&self, cx: &mut Context<Self>) -> impl IntoElement {
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
            .children(self.pages.slots.iter().enumerate().map(|(index, slot)| {
                self.nav_item(index, slot.spec.title.clone(), slot.spec.icon, cx)
            }))
    }
    fn nav_item(
        &self,
        index: usize,
        title: SharedString,
        icon: IconName,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        form::nav_item(index, icon, title, self.selected_page == index, cx)
            .test_support()
            .on_click(cx.listener(move |this, _, window, cx| this.select_page(index, window, cx)))
            .into_any_element()
    }
}

impl EventEmitter<ItemEvent> for SettingsView {}
impl Focusable for SettingsView {
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        if self.pages.slots[self.selected_page].lifecycle == Lifecycle::Eager {
            self.focus.clone()
        } else {
            self.pages
                .selected(self.selected_page)
                .map(|page| page.focus_handle(cx))
                .unwrap_or_else(|| self.focus.clone())
        }
    }
}
impl Item for SettingsView {
    fn tab_title(&self, _: &App) -> SharedString {
        "Settings".into()
    }
    fn tab_icon(&self, _: &App) -> IconName {
        IconName::Settings
    }
    fn on_close(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.commit_all(window, cx);
        for slot in &self.pages.slots {
            if let Some(page) = &slot.page {
                page.close(window, cx);
            }
        }
    }
}
impl Render for SettingsView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let content = self
            .pages
            .selected(self.selected_page)
            .map(|page| page.view().into_any_element())
            .unwrap_or_else(|| div().into_any_element());
        let section_errors: Vec<_> = self.installed.settings().section_errors(cx)
            .iter()
            .map(|error| {
                format!(
                    "The [{}] section of settings.toml has an error and uses its defaults until it is fixed or changed here: {}",
                    error.key, error.error
                )
            })
            .collect();
        let content = v_flex()
            .w_full()
            .max_w(form::page_width(cx))
            .px_8()
            .py_6()
            .children(
                section_errors
                    .into_iter()
                    .map(|error| form::error_text(error, cx)),
            )
            .when_some(self.feedback.error(cx), |content, error| {
                content.child(
                    div()
                        .id("settings-save-error")
                        .aria_label(error.clone())
                        .test_support()
                        .child(form::error_text(error, cx)),
                )
            })
            .child(content);
        h_flex()
            .id("settings-view")
            .test_support()
            .size_full()
            .min_h_0()
            .bg(nocterm_ui::hsla(
                nocterm_ui::TerminalStyle::with_settings(
                    self.installed
                        .settings()
                        .setting::<nocterm_ui::TerminalSettings>(cx),
                    cx,
                )
                .background,
            ))
            .track_focus(&self.focus)
            .child(self.render_nav(cx))
            .child(
                div()
                    .id(("settings-scroll", self.selected_page))
                    .flex_1()
                    .min_w_0()
                    .h_full()
                    .overflow_y_scroll()
                    .child(content),
            )
    }
}

#[cfg(test)]
mod tests;
