//! The User section entity: whether this run loaded the user plugins and the switch for the
//! next run, which user plugin is selected, the consent in progress and the last refusal.

use std::rc::Rc;

use gpui_kit::{
    App, AppContext as _, Context, Entity, FocusHandle, PathPromptOptions, Render, SharedString,
    Subscription, Task, Window,
};
use nocterm_plugin_model::PluginId;
use nocterm_ui::{FormFrame, FormScope, PluginSettingsDirectory, PluginSettingsForm};

use super::{
    consent::{Consent, INERT},
    folder::{FolderState, PluginsFolder},
    port::{Candidate, UserPluginError, UserPluginRow, UserPlugins, describe},
    removal::RemovalKind,
    section_view,
    session::{HeldBy, UserPluginsSwitch, UserSession},
};

/// The installed user plugins, the install dialog and the detail of the selected plugin.
pub struct UserPluginsSection {
    session: UserSession,
    switch: Rc<dyn UserPluginsSwitch>,
    directory: PluginSettingsDirectory,
    folder: FolderState,
    selected: Option<PluginId>,
    /// The fingerprint of the package the selection was made on, to notice a replacement.
    selected_fingerprint: Option<String>,
    form: Option<Entity<PluginSettingsForm>>,
    consent: Option<Consent>,
    /// Why the last action was refused.
    notice: Option<SharedString>,
    /// Which folded lists of the detail the user opened.
    unfolded: Unfolded,
    /// Where focus goes when the consent opens: refusing is the default answer.
    deny_focus: FocusHandle,
    _subscriptions: Vec<Subscription>,
    operation: Option<Task<()>>,
    generation: u64,
}

impl UserPluginsSection {
    pub fn new(
        session: UserSession,
        switch: Rc<dyn UserPluginsSwitch>,
        directory: PluginSettingsDirectory,
        folder: std::sync::Arc<dyn PluginsFolder>,
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
        let section = Self {
            session,
            switch,
            directory,
            folder: FolderState::new(folder),
            selected: None,
            selected_fingerprint: None,
            form: None,
            consent: None,
            notice: None,
            unfolded: Unfolded::default(),
            deny_focus: cx.focus_handle(),
            _subscriptions: subscriptions,
            operation: None,
            generation: 0,
        };
        section.refresh_folder(cx);
        section
    }

    pub(super) fn folder(&self) -> &FolderState {
        &self.folder
    }

    /// Lists the plugins folder again, when this run can install from it.
    pub(super) fn refresh_folder(&self, cx: &mut Context<Self>) {
        if !self.session.is_loaded() {
            return;
        }
        let folder = self.folder.port();
        let listed = cx
            .background_executor()
            .spawn(async move { folder.packages() });
        cx.spawn(async move |section, cx| {
            let listed = listed.await;
            let _ = section.update(cx, |section, cx| {
                section.folder.listed(listed);
                cx.notify();
            });
        })
        .detach();
    }

    /// Creates the plugins folder if needed and opens it in the file manager.
    pub(super) fn open_folder(&mut self, cx: &mut Context<Self>) {
        let folder = self.folder.port();
        let prepared = cx
            .background_executor()
            .spawn(async move { folder.prepare() });
        cx.spawn(async move |section, cx| {
            let prepared = prepared.await;
            let _ = section.update(cx, |section, cx| {
                match prepared {
                    Ok(path) => cx.open_with_system(&path),
                    Err(error) => section.folder.failed(&error),
                }
                cx.notify();
            });
        })
        .detach();
        self.refresh_folder(cx);
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

    pub(super) fn manager_error(&self, cx: &App) -> Option<SharedString> {
        self.session
            .loaded()?
            .error(cx)
            .map(|error| describe(&error).into())
    }

    pub(super) fn notice(&self) -> Option<&SharedString> {
        self.notice.as_ref()
    }

    /// Whether one of the detail's lists shows all of its lines.
    pub(super) fn is_unfolded(&self, list: FoldedList) -> bool {
        match list {
            FoldedList::Granted => self.unfolded.granted,
            FoldedList::Audit => self.unfolded.audit,
        }
    }

    /// Opens or folds one of the detail's lists.
    pub(super) fn toggle_fold(&mut self, list: FoldedList, cx: &mut Context<Self>) {
        let open = match list {
            FoldedList::Granted => &mut self.unfolded.granted,
            FoldedList::Audit => &mut self.unfolded.audit,
        };
        *open = !*open;
        cx.notify();
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
        if self.operation.is_some() {
            return;
        }
        self.notice = None;
        let Some(manager) = self.session.loaded() else {
            return;
        };
        let result = manager.inspect(location, cx);
        self.generation = self.generation.wrapping_add(1);
        let generation = self.generation;
        self.notice = Some("Reading plugin…".into());
        self.operation = Some(cx.spawn_in(window, async move |this, cx| {
            let result = result.await;
            let _ = this.update_in(cx, |section, window, cx| {
                if section.generation != generation {
                    return;
                }
                section.operation = None;
                section.notice = None;
                match result {
                    Ok(candidate) => section.open_consent(candidate, window, cx),
                    Err(error) => section.notice = Some(describe(&error).into()),
                }
                cx.notify();
            });
        }));
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
        if self.operation.is_some() {
            return;
        }
        let Some(consent) = self.consent.as_ref().filter(|consent| consent.can_accept()) else {
            return;
        };
        let (request, id) = (consent.request(), consent.candidate().id.clone());
        let Some(manager) = self.session.loaded().cloned() else {
            return;
        };
        let result = manager.install(request, cx);
        self.consent = None;
        self.notice = Some("Installing plugin…".into());
        self.generation = self.generation.wrapping_add(1);
        let generation = self.generation;
        self.operation = Some(cx.spawn_in(window, async move |this, cx| {
            let result = result.await;
            let _ = this.update_in(cx, |section, window, cx| {
                if section.generation != generation {
                    return;
                }
                section.operation = None;
                match result {
                    Ok(()) => {
                        section.notice = None;
                        section.select(&id, window, cx);
                    }
                    Err(error) => section.notice = Some(describe(&error).into()),
                }
                section.refresh_folder(cx);
                cx.notify();
            });
        }));
        cx.notify();
    }

    pub(super) fn deny(&mut self, cx: &mut Context<Self>) {
        self.consent = None;
        self.generation = self.generation.wrapping_add(1);
        self.operation = None;
        cx.notify();
    }

    /// Shows `plugin` in the detail, first sending what was typed into the previous form. The
    /// page hears of it ([`UserSelected`]) and lets go of its own selection.
    pub fn select(&mut self, plugin: &PluginId, window: &mut Window, cx: &mut Context<Self>) {
        cx.emit(UserSelected);
        if self.selected.as_ref() == Some(plugin) && self.form.is_some() {
            return;
        }
        self.flush(cx);
        self.unfolded = Unfolded::default();
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

    /// Lets go of the selection, after sending what was typed into its form.
    pub fn deselect(&mut self, cx: &mut Context<Self>) {
        if self.selected.is_none() {
            return;
        }
        self.flush(cx);
        self.selected = None;
        self.selected_fingerprint = None;
        self.form = None;
        cx.notify();
    }

    pub fn has_selection(&self) -> bool {
        self.selected.is_some()
    }

    /// Sends what was typed into the form.
    pub fn flush(&self, cx: &mut Context<Self>) {
        if let Some(form) = &self.form {
            form.update(cx, |form, cx| form.flush(cx));
        }
    }

    /// The one switch of a plugin. On enables it and allows it in windows, each only if it is
    /// not already, one after the other; off disables it and keeps its place in the layout.
    pub(super) fn set_on(&mut self, row: &UserPluginRow, on: bool, cx: &mut Context<Self>) {
        let Some(manager) = self.session.loaded().cloned() else {
            return;
        };
        let (plugin, enabled, placeable) = (row.id.clone(), row.enabled, row.placeable);
        self.act(cx, move |first, cx| {
            if !on {
                return first.set_enabled(&plugin, false, cx);
            }
            let enabling = (!enabled).then(|| first.set_enabled(&plugin, true, cx));
            cx.spawn(async move |cx| {
                if let Some(enabling) = enabling {
                    enabling.await?;
                }
                if !placeable {
                    cx.update(|cx| manager.set_placeable(&plugin, true, cx))
                        .await?;
                }
                Ok(())
            })
        });
    }

    /// Asks in a dialog what to remove of `plugin`; nothing is removed until the user confirms
    /// there ([`Self::remove`]).
    pub(super) fn ask_removal(
        &mut self,
        plugin: &PluginId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(name) = self
            .rows(cx)
            .into_iter()
            .find(|row| &row.id == plugin)
            .map(|row| row.name)
        else {
            return;
        };
        self.notice = None;
        cx.notify();
        super::removal::ask(cx.weak_entity(), plugin.clone(), name, window, cx);
    }

    /// Removes what the user confirmed in the removal dialog. A plugin that went meanwhile is
    /// left to the port, which says why it refused.
    pub(super) fn remove(&mut self, id: &PluginId, kind: RemovalKind, cx: &mut Context<Self>) {
        let id = id.clone();
        self.act(cx, |manager, cx| match kind {
            RemovalKind::Keep => manager.remove(&id, false, cx),
            RemovalKind::WithData => manager.remove(&id, true, cx),
            RemovalKind::DataOnly => manager.remove_data(&id, cx),
        });
    }

    /// Acts through the port and shows why it was refused. Nothing happens in a held run.
    fn act(
        &mut self,
        cx: &mut Context<Self>,
        action: impl FnOnce(&dyn UserPlugins, &mut App) -> Task<Result<(), UserPluginError>>,
    ) {
        if self.operation.is_some() {
            return;
        }
        let Some(manager) = self.session.loaded().cloned() else {
            return;
        };
        let outcome = action(manager.as_ref(), cx);
        self.notice = Some("Saving plugin changes…".into());
        self.operation = Some(cx.spawn(async move |section, cx| {
            let outcome = outcome.await;
            let _ = section.update(cx, |section, cx| {
                section.operation = None;
                section.notice = outcome.err().map(|error| describe(&error).into());
                cx.notify();
            });
        }));
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

/// The lists of the detail that fold.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum FoldedList {
    /// What the plugin is allowed to do.
    Granted,
    /// The audit log.
    Audit,
}

/// Which folded lists of the detail are open; they fold again when another plugin is selected.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Unfolded {
    granted: bool,
    audit: bool,
}

/// The user selected an installed plugin in the section.
pub struct UserSelected;

impl gpui_kit::EventEmitter<UserSelected> for UserPluginsSection {}

impl Render for UserPluginsSection {
    fn render(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl gpui_kit::IntoElement {
        section_view::view(self, window, cx)
    }
}
