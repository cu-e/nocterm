//! The User section entity: whether this run loaded the user plugins and the switch for the
//! next run, which user plugin is selected, the consent in progress and the last refusal.

use std::rc::Rc;

use gpui_kit::{
    App, AppContext as _, Context, Entity, FocusHandle, PathPromptOptions, Render, SharedString,
    Subscription, Window,
};
use nocterm_plugin_model::PluginId;
use nocterm_ui::{FormFrame, FormScope, PluginSettingsDirectory, PluginSettingsForm};

use super::{
    consent::{Consent, INERT},
    port::{Candidate, UserPluginError, UserPluginRow, UserPlugins, describe},
    section_view,
    session::{HeldBy, UserPluginsSwitch, UserSession},
};

/// The installed user plugins, the install dialog and the detail of the selected plugin.
pub struct UserPluginsSection {
    session: UserSession,
    switch: Rc<dyn UserPluginsSwitch>,
    directory: PluginSettingsDirectory,
    selected: Option<PluginId>,
    /// The fingerprint of the package the selection was made on, to notice a replacement.
    selected_fingerprint: Option<String>,
    form: Option<Entity<PluginSettingsForm>>,
    consent: Option<Consent>,
    /// Why the last action was refused.
    notice: Option<SharedString>,
    /// Where focus goes when the consent opens: refusing is the default answer.
    deny_focus: FocusHandle,
    _subscriptions: Vec<Subscription>,
}

impl UserPluginsSection {
    pub fn new(
        session: UserSession,
        switch: Rc<dyn UserPluginsSwitch>,
        directory: PluginSettingsDirectory,
        cx: &mut Context<Self>,
    ) -> Self {
        let weak = cx.weak_entity();
        let mut subscriptions = Vec::new();
        if let Some(manager) = session.loaded() {
            let weak = weak.clone();
            subscriptions.push(manager.observe(
                cx,
                Box::new(move |cx| {
                    let _ = weak.update(cx, |section, cx| section.changed(cx));
                }),
            ));
        }
        subscriptions.push(switch.observe(
            cx,
            Box::new(move |cx| {
                let _ = weak.update(cx, |_, cx| cx.notify());
            }),
        ));
        Self {
            session,
            switch,
            directory,
            selected: None,
            selected_fingerprint: None,
            form: None,
            consent: None,
            notice: None,
            deny_focus: cx.focus_handle(),
            _subscriptions: subscriptions,
        }
    }

    /// Why this run did not load the plugins, or `None` when it did.
    pub(super) fn held(&self) -> Option<&HeldBy> {
        match &self.session {
            UserSession::Loaded(_) => None,
            UserSession::Held(by) => Some(by),
        }
    }

    /// Whether this run loaded the plugins.
    pub(super) fn loaded(&self) -> bool {
        self.session.is_loaded()
    }

    /// The stored choice for the next start.
    pub(super) fn allowed(&self, cx: &App) -> bool {
        self.switch.allowed(cx)
    }

    /// Stores the choice for the next start; a refusal shows as the notice and the switch keeps
    /// the stored value.
    pub(super) fn set_allowed(&mut self, allowed: bool, cx: &mut Context<Self>) {
        self.notice = None;
        let stored = self.switch.set_allowed(allowed, cx);
        cx.spawn(async move |section, cx| {
            if let Err(error) = stored.await {
                let _ = section.update(cx, |section, cx| {
                    section.notice = Some(describe(&error).into());
                    cx.notify();
                });
            }
        })
        .detach();
        cx.notify();
    }

    /// The file a developer session loads, when this installation is one.
    pub(super) fn dev_location(&self, cx: &App) -> Option<String> {
        self.session.loaded()?.dev_location(cx)
    }

    pub(super) fn rows(&self, cx: &App) -> Vec<UserPluginRow> {
        self.session
            .loaded()
            .map_or_else(Vec::new, |manager| manager.rows(cx))
    }

    pub(super) fn audit(&self, cx: &App) -> Vec<super::AuditLine> {
        self.session
            .loaded()
            .map_or_else(Vec::new, |manager| manager.audit(cx))
    }

    pub(super) fn selected(&self) -> Option<&PluginId> {
        self.selected.as_ref()
    }

    pub(super) fn form(&self) -> Option<&Entity<PluginSettingsForm>> {
        self.form.as_ref()
    }

    pub(super) fn consent(&self) -> Option<&Consent> {
        self.consent.as_ref()
    }

    pub(super) fn consent_mut(&mut self) -> Option<&mut Consent> {
        self.consent.as_mut()
    }

    pub(super) fn notice(&self) -> Option<&SharedString> {
        self.notice.as_ref()
    }

    pub(super) fn deny_focus(&self) -> &FocusHandle {
        &self.deny_focus
    }

    /// Whether the section knows a plugin with this id.
    pub fn holds(&self, plugin: &PluginId, cx: &App) -> bool {
        self.rows(cx).iter().any(|row| &row.id == plugin)
    }

    /// Asks the user for a package file, then starts the install from it.
    pub(super) fn choose_file(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let picked = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: None,
        });
        cx.spawn_in(window, async move |this, cx| {
            if let Ok(Ok(Some(paths))) = picked.await
                && let Some(path) = paths.first()
            {
                let location = path.to_string_lossy().into_owned();
                let _ = this.update_in(cx, |section, window, cx| {
                    section.begin_install(&location, window, cx);
                });
            }
        })
        .detach();
    }

    /// Reads the package at `location` and, when it is acceptable, opens the consent. Nothing is
    /// stored until the user accepts.
    pub fn begin_install(&mut self, location: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.notice = None;
        let Some(manager) = self.session.loaded() else {
            return;
        };
        match manager.inspect(location, cx) {
            Ok(candidate) => self.open_consent(candidate, window, cx),
            Err(error) => self.notice = Some(describe(&error).into()),
        }
        cx.notify();
    }

    fn open_consent(&mut self, candidate: Candidate, window: &mut Window, cx: &mut Context<Self>) {
        let timer = cx.spawn(async move |this, cx| {
            cx.background_executor().timer(INERT).await;
            let _ = this.update(cx, |section, cx| {
                if let Some(consent) = section.consent.as_mut() {
                    consent.arm();
                }
                cx.notify();
            });
        });
        self.consent = Some(Consent::new(candidate, timer));
        window.focus(&self.deny_focus, cx);
    }

    /// Installs what the consent shows, if its buttons are live.
    pub(super) fn accept(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(consent) = self.consent.as_ref().filter(|consent| consent.can_accept()) else {
            return;
        };
        let (request, id) = (consent.request(), consent.candidate().id.clone());
        let Some(manager) = self.session.loaded().cloned() else {
            return;
        };
        match manager.install(request, cx) {
            Ok(()) => {
                self.consent = None;
                self.notice = None;
                self.select(&id, window, cx);
            }
            Err(error) => {
                self.consent = None;
                self.notice = Some(describe(&error).into());
            }
        }
        cx.notify();
    }

    pub(super) fn deny(&mut self, cx: &mut Context<Self>) {
        self.consent = None;
        cx.notify();
    }

    /// Shows `plugin` in the detail, first sending what was typed into the previous form.
    pub fn select(&mut self, plugin: &PluginId, window: &mut Window, cx: &mut Context<Self>) {
        if self.selected.as_ref() == Some(plugin) && self.form.is_some() {
            return;
        }
        self.flush(cx);
        let name = self
            .rows(cx)
            .into_iter()
            .find(|row| &row.id == plugin)
            .map(|row| row.name);
        self.form = name.and_then(|name| {
            let (schema, source) = self.directory.get(plugin, cx)?;
            let plugin = plugin.clone();
            Some(cx.new(|cx| {
                PluginSettingsForm::new(
                    source,
                    plugin,
                    schema,
                    FormScope::Plugin,
                    FormFrame::Plugin { name: name.into() },
                    window,
                    cx,
                )
            }))
        });
        self.selected = Some(plugin.clone());
        self.selected_fingerprint = self.fingerprint_of(plugin, cx);
        cx.notify();
    }

    fn fingerprint_of(&self, plugin: &PluginId, cx: &App) -> Option<String> {
        self.rows(cx)
            .into_iter()
            .find(|row| &row.id == plugin)
            .map(|row| row.fingerprint)
    }

    /// Sends what was typed into the form.
    pub fn flush(&self, cx: &mut Context<Self>) {
        if let Some(form) = &self.form {
            form.update(cx, |form, cx| form.flush(cx));
        }
    }

    pub(super) fn set_enabled(&mut self, plugin: &PluginId, enabled: bool, cx: &mut Context<Self>) {
        self.act(cx, |manager, cx| manager.set_enabled(plugin, enabled, cx));
    }

    pub(super) fn set_placeable(
        &mut self,
        plugin: &PluginId,
        placeable: bool,
        cx: &mut Context<Self>,
    ) {
        self.act(cx, |manager, cx| {
            manager.set_placeable(plugin, placeable, cx)
        });
    }

    pub(super) fn remove(&mut self, plugin: &PluginId, with_data: bool, cx: &mut Context<Self>) {
        self.act(cx, |manager, cx| manager.remove(plugin, with_data, cx));
    }

    pub(super) fn remove_data(&mut self, plugin: &PluginId, cx: &mut Context<Self>) {
        self.act(cx, |manager, cx| manager.remove_data(plugin, cx));
    }

    /// Acts through the port and shows why it was refused. Nothing happens in a held run.
    fn act(
        &mut self,
        cx: &mut Context<Self>,
        action: impl FnOnce(&dyn UserPlugins, &mut App) -> Result<(), UserPluginError>,
    ) {
        let Some(manager) = self.session.loaded().cloned() else {
            return;
        };
        let outcome = action(manager.as_ref(), cx);
        self.notice = outcome.err().map(|error| describe(&error).into());
        cx.notify();
    }

    /// The installed plugins changed: drop the selection of one that is gone.
    fn changed(&mut self, cx: &mut Context<Self>) {
        // A plugin that is gone, or replaced by another package, may have other settings than
        // the form shows, so the selection is dropped and the next one builds a new form.
        let stale = self
            .selected
            .as_ref()
            .is_some_and(|selected| self.fingerprint_of(selected, cx) != self.selected_fingerprint);
        if stale {
            self.selected = None;
            self.selected_fingerprint = None;
            self.form = None;
        }
        cx.notify();
    }
}

impl Render for UserPluginsSection {
    fn render(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl gpui_kit::IntoElement {
        section_view::view(self, window, cx)
    }
}
