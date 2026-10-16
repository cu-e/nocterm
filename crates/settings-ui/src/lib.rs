//! A single settings tab, using the same schema and store as the terminal.
//!
//! Every control saves as soon as it changes; text saves when typing pauses,
//! on Enter and when the field loses focus. There is no draft and no Apply.
//! Owns: settings Item navigation, shared diagnostics, eager builtin and lazy guest lifetimes.
//! Does not own: page controls, schema policy, native probing or settings persistence.
//! Ports used: Installed supplies SettingsHandle; OwnerPages supplies owner factories.
//! Ports provided: workspace settings Item. Settings section: none. Globals: none.
//! Invariants: builtin construction and all-builtin flush precede selection; guest lifetime stays lazy.

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

/// Owner factories supplied by the app; the five builtin slots retain their established order.
#[derive(Clone)]
pub struct OwnerPages {
    builtins: [SettingsPageSpec; 5],
    guests: Vec<SettingsPageSpec>,
}
impl OwnerPages {
    pub fn with_guests(&self, guests: Vec<SettingsPageSpec>) -> Self {
        Self {
            builtins: self.builtins.clone(),
            guests,
        }
    }
    pub fn new(builtins: [SettingsPageSpec; 5], guests: Vec<SettingsPageSpec>) -> Self {
        Self { builtins, guests }
    }
}
struct PageBook {
    builtins: [(SettingsPageSpec, Box<dyn SettingsPageHandle>); 5],
    guests: Vec<(SettingsPageSpec, Option<Box<dyn SettingsPageHandle>>)>,
}
/// Installs the settings Item with factories supplied by the actual feature owners.
pub fn register(workspace: &mut Workspace, pages: OwnerPages, installed: Installed) {
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
    pages: &OwnerPages,
    installed: Installed,
    window: &mut Window,
    cx: &mut Context<Workspace>,
) {
    let item = workspace.find_item::<SettingsView>().unwrap_or_else(|| {
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
        pages: OwnerPages,
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
        let builtins = pages.builtins.map(|spec| {
            let page = spec.create(context.clone(), window, cx);
            (spec, page)
        });
        Self {
            installed,
            selected_page: 0,
            pages: PageBook {
                builtins,
                guests: pages.guests.into_iter().map(|spec| (spec, None)).collect(),
            },
            focus: cx.focus_handle(),
            feedback,
            _subscriptions: vec![changed, settings],
        }
    }
    pub fn selected_page_id(&self) -> &str {
        match Page::ALL.get(self.selected_page) {
            Some(page) => page.id(),
            None => self.pages.guests[self.selected_page - Page::ALL.len()].0.id,
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
                    .guests
                    .iter()
                    .position(|(spec, _)| spec.id == id)
                    .map(|index| index + Page::ALL.len())
            })
    }
    fn guest(&self) -> Option<&dyn SettingsPageHandle> {
        self.selected_page
            .checked_sub(Page::ALL.len())
            .and_then(|index| self.pages.guests.get(index))
            .and_then(|(_, page)| page.as_deref())
    }
    fn select_page(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        if index >= Page::ALL.len() + self.pages.guests.len() || index == self.selected_page {
            return;
        }
        // Leaving a page saves what is being typed there.
        self.commit_all(window, cx);
        if let Some(page) = self.guest() {
            page.deactivate(window, cx);
        }
        self.selected_page = index;
        if let Some((spec, page)) = index
            .checked_sub(Page::ALL.len())
            .and_then(|index| self.pages.guests.get_mut(index))
            && page.is_none()
        {
            let context =
                SettingsPageContext::new(self.installed.settings().clone(), self.feedback.clone());
            *page = Some(spec.create(context, window, cx));
        }
        window.focus(&self.focus_handle(cx), cx);
        cx.notify();
    }

    /// Every eager builtin flushes before the active guest is deactivated.
    fn commit_all(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        for (_, page) in &self.pages.builtins {
            page.flush(window, cx);
        }
    }

    fn render_nav(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let builtin = Page::ALL
            .iter()
            .enumerate()
            .map(|(index, page)| (index, SharedString::from(page.title()), page.icon()));
        let guests = self
            .pages
            .guests
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
        self.commit_all(window, cx);
        for (_, page) in &self.pages.builtins {
            page.close(window, cx);
        }
        for (_, page) in &self.pages.guests {
            if let Some(page) = page {
                page.close(window, cx);
            }
        }
    }
}
impl Render for SettingsView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let content = match self.builtin() {
            Some(_) => self.pages.builtins[self.selected_page]
                .1
                .view()
                .into_any_element(),
            None => self
                .guest()
                .map(|page| page.view().into_any_element())
                .unwrap_or_else(|| div().into_any_element()),
        };
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
