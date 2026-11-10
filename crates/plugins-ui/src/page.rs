//! The page entity: which plugin is selected, its settings form and the result of the last switch.

use std::{rc::Rc, sync::Arc};

use gpui_kit::{
    App, AppContext as _, Context, Entity, FocusHandle, Focusable, Render, SharedString,
    Subscription, Window,
};
use nocterm_plugin_model::{PluginCatalog, PluginId};
use nocterm_ui::{FormFrame, FormScope, PluginSettingsDirectory, PluginSettingsForm, SettingsPage};

use crate::{EnablementError, PluginToggles, view};

/// The Plugins settings page: every plugin in one list on the left, built-in plugins first and
/// external (user) plugins after them, and the selected one on the right. One plugin is
/// selected at a time, of either kind.
pub struct PluginsPage {
    catalog: Arc<PluginCatalog>,
    toggles: Rc<dyn PluginToggles>,
    directory: PluginSettingsDirectory,
    selected: Option<PluginId>,
    form: Option<Entity<PluginSettingsForm>>,
    /// Why the last switch of this plugin was refused.
    refusal: Option<(PluginId, SharedString)>,
    focus: FocusHandle,
    user: Option<UserParts>,
    _subscriptions: Vec<Subscription>,
}

/// The User section and the parts of it the page draws in place.
pub(crate) struct UserParts {
    section: Entity<crate::UserPluginsSection>,
    head: Entity<crate::UserPartView>,
    rows: Entity<crate::UserPartView>,
    detail: Entity<crate::UserPartView>,
}

impl UserParts {
    pub(crate) fn section(&self) -> &Entity<crate::UserPluginsSection> {
        &self.section
    }

    pub(crate) fn head(&self) -> &Entity<crate::UserPartView> {
        &self.head
    }

    pub(crate) fn rows(&self) -> &Entity<crate::UserPartView> {
        &self.rows
    }

    pub(crate) fn detail(&self) -> &Entity<crate::UserPartView> {
        &self.detail
    }
}

impl PluginsPage {
    pub fn new(
        catalog: Arc<PluginCatalog>,
        toggles: Rc<dyn PluginToggles>,
        directory: PluginSettingsDirectory,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let weak = cx.weak_entity();
        let toggled = toggles.observe(
            cx,
            Box::new(move |cx| {
                let _ = weak.update(cx, |_, cx| cx.notify());
            }),
        );
        let first = catalog.plugins().next().map(|plugin| plugin.id.clone());
        let mut page = Self {
            catalog,
            toggles,
            directory,
            selected: None,
            form: None,
            refusal: None,
            focus: cx.focus_handle(),
            user: None,
            _subscriptions: vec![toggled],
        };
        if let Some(id) = first {
            page.select(&id, window, cx);
        }
        page
    }

    /// Adds the User section below the system plugins.
    pub fn with_user_plugins(
        mut self,
        session: crate::UserSession,
        switch: Rc<dyn crate::UserPluginsSwitch>,
        folder: Arc<dyn crate::PluginsFolder>,
        cx: &mut Context<Self>,
    ) -> Self {
        use crate::{UserPart, UserPartView};
        let directory = self.directory.clone();
        let section =
            cx.new(|cx| crate::UserPluginsSection::new(session, switch, directory, folder, cx));
        // Selecting an external plugin lets go of the built-in one, so one plugin is selected.
        self._subscriptions.push(
            cx.subscribe(&section, |page, _, _: &crate::UserSelected, cx| {
                page.flush_system_form(cx);
                page.selected = None;
                page.form = None;
                cx.notify();
            }),
        );
        let part = |part, cx: &mut Context<Self>| {
            let section = section.clone();
            cx.new(|cx| UserPartView::new(section, part, cx))
        };
        self.user = Some(UserParts {
            head: part(UserPart::Head, cx),
            rows: part(UserPart::Rows, cx),
            detail: part(UserPart::Detail, cx),
            section,
        });
        self
    }

    pub(crate) fn user(&self) -> Option<&UserParts> {
        self.user.as_ref()
    }

    pub(crate) fn catalog(&self) -> &PluginCatalog {
        &self.catalog
    }

    pub(crate) fn selected(&self) -> Option<&PluginId> {
        self.selected.as_ref()
    }

    pub(crate) fn form(&self) -> Option<&Entity<PluginSettingsForm>> {
        self.form.as_ref()
    }

    pub(crate) fn is_enabled(&self, plugin: &PluginId, cx: &App) -> bool {
        self.toggles.enabled(plugin, cx)
    }

    pub(crate) fn refusal(&self, plugin: &PluginId) -> Option<SharedString> {
        self.refusal
            .as_ref()
            .filter(|(refused, _)| refused == plugin)
            .map(|(_, text)| text.clone())
    }

    /// Shows `id` in the detail, first sending what was typed into the previous form, and lets go
    /// of a selected external plugin.
    pub(crate) fn select(&mut self, id: &PluginId, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(user) = &self.user {
            user.section.update(cx, |section, cx| section.deselect(cx));
        }
        if self.selected.as_ref() == Some(id) {
            return;
        }
        self.flush_form(cx);
        self.form = self.directory.get(id, cx).map(|(schema, source)| {
            cx.new(|cx| {
                PluginSettingsForm::new(
                    source,
                    id.clone(),
                    schema,
                    FormScope::Plugin,
                    FormFrame::Host,
                    window,
                    cx,
                )
            })
        });
        self.selected = Some(id.clone());
        cx.notify();
    }

    pub(crate) fn set_enabled(&mut self, id: PluginId, enabled: bool, cx: &mut Context<Self>) {
        self.refusal = None;
        let switched = self.toggles.set_enabled(&id, enabled, cx);
        cx.spawn(async move |page, cx| {
            let result = switched.await;
            let _ = page.update(cx, |page, cx| page.settled(id, result, cx));
        })
        .detach();
        cx.notify();
    }

    fn settled(
        &mut self,
        id: PluginId,
        result: Result<(), EnablementError>,
        cx: &mut Context<Self>,
    ) {
        if let Err(error) = result {
            self.refusal = Some((id, error.to_string().into()));
        }
        cx.notify();
    }

    fn flush_form(&self, cx: &mut Context<Self>) {
        self.flush_system_form(cx);
        if let Some(user) = &self.user {
            user.section.update(cx, |user, cx| user.flush(cx));
        }
    }

    fn flush_system_form(&self, cx: &mut Context<Self>) {
        if let Some(form) = &self.form {
            form.update(cx, |form, cx| form.flush(cx));
        }
    }
}

impl PluginsPage {
    /// Selects the user plugin whose id is `anchor`, if the page has a User section.
    fn reveal_user(&mut self, anchor: &str, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let id = PluginId::owned(anchor);
        let Some(user) = self
            .user
            .as_ref()
            .map(|user| user.section.clone())
            .filter(|user| user.read(cx).holds(&id, cx))
        else {
            return false;
        };
        user.update(cx, |user, cx| user.select(&id, window, cx));
        true
    }
}

impl SettingsPage for PluginsPage {
    fn flush(&mut self, _: &mut Window, cx: &mut Context<Self>) {
        self.flush_form(cx);
    }

    fn on_deactivate(&mut self, _: &mut Window, cx: &mut Context<Self>) {
        self.flush_form(cx);
    }

    /// Selects the plugin whose id is `anchor`.
    fn reveal(&mut self, anchor: &str, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let found = self
            .catalog
            .plugins()
            .find(|plugin| plugin.id.as_str() == anchor)
            .map(|plugin| plugin.id.clone());
        match found {
            Some(id) => {
                self.select(&id, window, cx);
                true
            }
            None => self.reveal_user(anchor, window, cx),
        }
    }
}

impl Focusable for PluginsPage {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for PluginsPage {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl gpui_kit::IntoElement {
        view::page(self, cx)
    }
}
