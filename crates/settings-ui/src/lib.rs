//! A single settings tab, using the same schema and store as the terminal.
//!
//! Every control saves as soon as it changes; text saves when typing pauses,
//! on Enter and when the field loses focus. There is no draft and no Apply.

mod ai;
mod field;
mod pages;

use std::collections::BTreeMap;

use field::Field;
use gpui_kit::{
    AnyElement, App, Context, EventEmitter, FocusHandle, Focusable, SharedString, Subscription,
    TestSupportExt as _, Window,
    component::{StyledExt as _, h_flex, input::InputEvent, v_flex},
    div,
    prelude::*,
    px, rems,
};
use nocterm_settings::Settings;
use nocterm_ui::{IconName, SettingsStore, edit_settings, form};
use nocterm_workspace::{
    Item, ItemEvent, OpenSettings, SettingsPageHandle, SettingsPageSpec, Workspace,
};

/// The pages every build has, in navigation order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Page {
    Appearance,
    Terminal,
    LocalShell,
    Ssh,
    Ai,
}

impl Page {
    const ALL: [Page; 5] = [
        Page::Appearance,
        Page::Terminal,
        Page::LocalShell,
        Page::Ssh,
        Page::Ai,
    ];
    fn id(self) -> &'static str {
        match self {
            Page::Appearance => "appearance",
            Page::Terminal => "terminal",
            Page::LocalShell => "local-shell",
            Page::Ssh => "ssh",
            Page::Ai => "ai",
        }
    }
    fn title(self) -> &'static str {
        match self {
            Page::Appearance => "Appearance",
            Page::Terminal => "Terminal",
            Page::LocalShell => "Local shell",
            Page::Ssh => "SSH",
            Page::Ai => "AI agents",
        }
    }
    fn icon(self) -> IconName {
        match self {
            Page::Appearance => IconName::Palette,
            Page::Terminal => IconName::Terminal,
            Page::LocalShell => IconName::SquareTerminal,
            Page::Ssh => IconName::Server,
            Page::Ai => IconName::Bot,
        }
    }
}

/// Installs Settings with its built-in pages.
pub fn register(workspace: &mut Workspace) {
    register_with_pages(workspace, Vec::new());
}
/// Installs Settings and feature-owned pages supplied by the application.
pub fn register_with_pages(workspace: &mut Workspace, pages: Vec<SettingsPageSpec>) {
    workspace.register_action(move |_, _: &OpenSettings, window, cx| {
        let workspace = cx.entity().downgrade();
        let pages = pages.clone();
        // Popup dismissal restores its old action context first. Opening the
        // Item afterwards lets its own field keep keyboard focus.
        window.defer(cx, move |window, cx| {
            let _ = workspace.update(cx, |workspace, cx| {
                open_page(workspace, "", &pages, window, cx);
            });
        });
    });
}
/// Opens the singleton Settings Item and selects a page. Empty preserves its selection.
pub fn open_page(
    workspace: &mut Workspace,
    id: &str,
    pages: &[SettingsPageSpec],
    window: &mut Window,
    cx: &mut Context<Workspace>,
) {
    let item = workspace.find_item::<SettingsView>().unwrap_or_else(|| {
        let item = cx.new(|cx| SettingsView::with_pages(pages.to_vec(), window, cx));
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
    selected_page: usize,
    /// Pages supplied by other features, created when first shown.
    pages: Vec<(SettingsPageSpec, Option<Box<dyn SettingsPageHandle>>)>,
    focus: FocusHandle,
    /// Text settings by key, such as `terminal.font_size`.
    fields: BTreeMap<SharedString, Field>,
    ai: ai::AiPage,
    session_options: gpui_kit::Entity<nocterm_ui::SessionOptionsEditor>,
    session_options_error: Option<SharedString>,
    session_options_save: Option<gpui_kit::Task<()>>,
    /// The last save failure, shown until the next successful save.
    error: Option<SharedString>,
    _subscriptions: Vec<Subscription>,
}

impl SettingsView {
    #[cfg(test)]
    fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        Self::with_pages(Vec::new(), window, cx)
    }
    fn with_pages(
        pages: Vec<SettingsPageSpec>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let session_options = pages::new_session_options(window, cx);
        let sync = cx.observe_global_in::<SettingsStore>(window, |this, window, cx| {
            this.sync(window, cx);
        });
        let options = cx.observe_in(&session_options, window, |this, _, window, cx| {
            this.schedule_session_options(window, cx);
        });
        let mut this = Self {
            selected_page: 0,
            pages: pages.into_iter().map(|spec| (spec, None)).collect(),
            focus: cx.focus_handle(),
            fields: BTreeMap::new(),
            ai: ai::AiPage::default(),
            session_options,
            session_options_error: None,
            session_options_save: None,
            error: None,
            _subscriptions: vec![sync, options],
        };
        pages::add_fields(&mut this, window, cx);
        ai::add_fields(&mut this, window, cx);
        this
    }

    pub fn selected_page_id(&self) -> &str {
        match Page::ALL.get(self.selected_page) {
            Some(page) => page.id(),
            None => self.pages[self.selected_page - Page::ALL.len()].0.id,
        }
    }
    fn builtin(&self) -> Option<Page> {
        Page::ALL.get(self.selected_page).copied()
    }
    fn page_index(&self, id: &str) -> Option<usize> {
        Page::ALL
            .iter()
            .position(|page| page.id() == id)
            .or_else(|| {
                self.pages
                    .iter()
                    .position(|(spec, _)| spec.id == id)
                    .map(|index| index + Page::ALL.len())
            })
    }
    fn guest(&self) -> Option<&dyn SettingsPageHandle> {
        self.selected_page
            .checked_sub(Page::ALL.len())
            .and_then(|index| self.pages.get(index))
            .and_then(|(_, page)| page.as_deref())
    }
    fn select_page(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        if index >= Page::ALL.len() + self.pages.len() || index == self.selected_page {
            return;
        }
        // Leaving a page saves what is being typed there.
        self.commit_all(cx);
        if let Some(page) = self.guest() {
            page.deactivate(window, cx);
        }
        self.selected_page = index;
        if let Some((spec, page)) = index
            .checked_sub(Page::ALL.len())
            .and_then(|index| self.pages.get_mut(index))
            && page.is_none()
        {
            *page = Some(spec.create(window, cx));
        }
        window.focus(&self.focus_handle(cx), cx);
        cx.notify();
    }

    /// Adds a text setting saved under `key`.
    pub(crate) fn add_field(
        &mut self,
        key: impl Into<SharedString>,
        read: impl Fn(&Settings) -> String + 'static,
        write: impl Fn(&mut Settings, &str) -> Result<(), String> + 'static,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let key = key.into();
        let event_key = key.clone();
        let field = Field::new(read, write, window, cx, move |this, event, window, cx| {
            this.field_event(&event_key, event, window, cx)
        });
        self.fields.insert(key, field);
    }
    pub(crate) fn remove_fields(&mut self, prefix: &str) {
        self.fields.retain(|key, _| !key.starts_with(prefix));
    }
    fn field_event(
        &mut self,
        key: &SharedString,
        event: &InputEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            InputEvent::Change => {
                let pending = key.clone();
                let task = cx.spawn_in(window, async move |this, cx| {
                    cx.background_executor().timer(field::DEBOUNCE).await;
                    let _ = this.update(cx, |this, cx| {
                        if let Some(field) = this.fields.get_mut(&pending) {
                            field.commit(cx);
                        }
                        cx.notify();
                    });
                });
                if let Some(field) = self.fields.get_mut(key) {
                    field.debounce = Some(task);
                }
            }
            InputEvent::PressEnter { .. } | InputEvent::Blur => {
                if let Some(field) = self.fields.get_mut(key) {
                    field.commit(cx);
                }
                cx.notify();
            }
            InputEvent::Focus => {}
        }
    }
    /// Saves every field with unsaved text now.
    fn commit_all(&mut self, cx: &mut Context<Self>) {
        for field in self.fields.values_mut() {
            if field.debounce.is_some() {
                field.commit(cx);
            }
        }
        if self.session_options_save.take().is_some() {
            self.save_session_options(cx);
        }
    }
    fn sync(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        for field in self.fields.values_mut() {
            field.sync(window, cx);
        }
        ai::sync(self, window, cx);
        cx.notify();
    }

    /// Saves a change that needs no validation, such as a switch.
    pub(crate) fn save(
        &mut self,
        edit: impl FnOnce(&mut Settings) + 'static,
        cx: &mut Context<Self>,
    ) {
        let task = edit_settings(cx, edit);
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |this, cx| {
                this.error = result
                    .err()
                    .map(|error| format!("Could not save settings: {error}").into());
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    fn schedule_session_options(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.session_options_save = Some(cx.spawn_in(window, async move |this, cx| {
            cx.background_executor().timer(field::DEBOUNCE).await;
            let _ = this.update(cx, |this, cx| {
                this.session_options_save = None;
                this.save_session_options(cx);
            });
        }));
    }
    fn save_session_options(&mut self, cx: &mut Context<Self>) {
        match self.session_options.read(cx).options(cx) {
            Ok(options) => {
                self.session_options_error = None;
                self.save(
                    move |settings| pages::apply_session_options(settings, options),
                    cx,
                );
            }
            Err(error) => {
                self.session_options_error = Some(error.into());
                cx.notify();
            }
        }
    }

    pub(crate) fn field(&self, key: &str) -> &Field {
        &self.fields[key]
    }

    fn render_nav(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let builtin = Page::ALL
            .iter()
            .enumerate()
            .map(|(index, page)| (index, SharedString::from(page.title()), page.icon()));
        let guests = self
            .pages
            .iter()
            .enumerate()
            .map(|(index, (spec, _))| (Page::ALL.len() + index, spec.title.clone(), spec.icon));
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
                builtin
                    .chain(guests)
                    .map(|(index, title, icon)| self.nav_item(index, title, icon, cx)),
            )
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
        match self.builtin() {
            Some(_) => self.focus.clone(),
            None => self
                .guest()
                .map(|page| page.focus_handle(cx))
                .unwrap_or_else(|| self.focus.clone()),
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
        self.commit_all(cx);
        for (_, page) in &self.pages {
            if let Some(page) = page {
                page.close(window, cx);
            }
        }
    }
}
impl Render for SettingsView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let content = match self.builtin() {
            Some(Page::Ai) => ai::render(self, cx),
            Some(page) => pages::render(self, page, cx),
            None => self
                .guest()
                .map(|page| page.view().into_any_element())
                .unwrap_or_else(|| div().into_any_element()),
        };
        let content = v_flex()
            .w_full()
            .max_w(form::page_width(cx))
            .px_8()
            .py_6()
            .when_some(self.error.clone(), |content, error| {
                content.child(form::error_text(error, cx))
            })
            .child(content);
        h_flex()
            .size_full()
            .min_h_0()
            .bg(form::page_background(cx))
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
