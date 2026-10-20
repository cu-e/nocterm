//! The "AI agents" settings page: the master switch, approvals, sandboxing,
//! resources and every available agent's launch settings.
//!
//! Owns: the page view, its text fields and their validation feedback, adding
//! and removing custom agents, and the sandbox availability shown on it.
//! Does not own: which agents exist or how settings resolve into them
//! (`nocterm-agent-directory`), the rules that validate a setting
//! (`nocterm-ai`), probing the sandbox natively (the injected `SandboxProbe`),
//! or chats.
//! Ports used: `SettingsHandle`, `AgentsHandle`, AI `SandboxProbe`. Ports
//! provided: `settings_page`, a `SettingsPageSpec` for the settings window.
//! Settings section: reads and edits `ai`, owned by `nocterm-ai`.
//! Globals: none.
//! Invariants: every edit is validated by the AI rules before it is saved;
//! the page reads agents only through the directory.
mod agents;
mod controls;
mod field;
mod render;
use gpui_kit::Entity;
use gpui_kit::component::input::InputState;
use nocterm_agent_directory::AgentsHandle;
use nocterm_ai::sandbox::{Availability, SandboxProbe};
use nocterm_store::SettingsHandle;
use std::{collections::BTreeSet, sync::Arc};
struct AiPage {
    /// Agents whose details are shown.
    expanded: BTreeSet<String>,
    /// Agents with fields, in display order.
    agents: Vec<String>,
    new_id: Option<Entity<InputState>>,
    new_command: Option<Entity<InputState>>,
    add_error: Option<SharedString>,
    /// Why the last switch could not be saved.
    error: Option<SharedString>,
    sandbox: Option<Availability>,
    probe: Arc<dyn SandboxProbe>,
}

use field::FieldError;
use gpui_kit::{
    App, AppContext as _, Context, FocusHandle, Focusable, SharedString, Subscription, Window,
    prelude::*,
};
use nocterm_settings::SettingsSection;
use nocterm_ui::{
    SettingsPage, SettingsPageContext, SettingsPageFeedback, SettingsPageSpec, SettingsTextField,
};
use std::collections::BTreeMap;

/// The page, editing `settings` and listing `agents`; `probe` reports whether
/// agents can be sandboxed here.
pub fn settings_page(
    settings: SettingsHandle,
    agents: AgentsHandle,
    probe: Arc<dyn SandboxProbe>,
) -> SettingsPageSpec {
    SettingsPageSpec::contextual("ai", "AI agents", move |context, window, cx| {
        let (settings, agents, probe) = (settings.clone(), agents.clone(), probe.clone());
        cx.new(|cx| AgentSettingsPage::new(settings, agents, probe, context, window, cx))
    })
    .with_icon(nocterm_ui::IconName::Bot)
}
struct AgentSettingsPage {
    settings: SettingsHandle,
    agents: AgentsHandle,
    fields: BTreeMap<SharedString, nocterm_ui::FieldPresentation>,
    focus: FocusHandle,
    _subscriptions: Vec<Subscription>,
    feedback: SettingsPageFeedback,
    ai: AiPage,
}
impl AgentSettingsPage {
    fn settings(&self) -> &SettingsHandle {
        &self.settings
    }
    fn agents(&self) -> &AgentsHandle {
        &self.agents
    }
    fn new(
        settings: SettingsHandle,
        agents: AgentsHandle,
        probe: Arc<dyn SandboxProbe>,
        context: SettingsPageContext,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let weak = cx.weak_entity();
        let original_window = window.window_handle();
        let subscription = settings.observe(cx, move |cx| {
            let _ = original_window.update(cx, |_, window, cx| {
                let _ = weak.update(cx, |this, cx| {
                    agents::sync(this, window, cx);
                    cx.notify();
                });
            });
        });
        let mut this = Self {
            ai: AiPage::new(probe),
            settings,
            agents,
            fields: BTreeMap::new(),
            focus: cx.focus_handle(),
            _subscriptions: vec![subscription],
            feedback: context.feedback().clone(),
        };
        agents::add_fields(&mut this, window, cx);
        this
    }
    /// Adds a text setting of section `S` saved under `key`.
    fn add_field<S: SettingsSection>(
        &mut self,
        key: impl Into<SharedString>,
        read: impl Fn(&S) -> String + 'static,
        write: impl Fn(&mut S, &str) -> Result<(), FieldError> + 'static,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let key = key.into();
        let field = SettingsTextField::new(
            self.settings().clone(),
            key.clone(),
            move |document| read(document.get::<S>()),
            move |document, text| {
                let mut section = document.get::<S>().clone();
                write(&mut section, text).map_err(nocterm_store::StoreError::edit)?;
                document.set(section);
                Ok(())
            },
            window,
            cx,
        );
        self.fields
            .insert(key, nocterm_ui::FieldPresentation::new(field, window, cx));
    }
    fn remove_fields(&mut self, prefix: &str) {
        self.fields.retain(|key, _| !key.starts_with(prefix));
    }
    fn field(&self, key: &str) -> &SettingsTextField {
        self.fields[key].field()
    }

    fn save<S: SettingsSection>(
        &mut self,
        edit: impl FnOnce(&mut S) + 'static,
        cx: &mut Context<Self>,
    ) {
        let task = self.settings().update::<S>(cx, edit);
        self.feedback.report(task, cx);
    }
}
impl Focusable for AgentSettingsPage {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}
impl SettingsPage for AgentSettingsPage {
    fn flush(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        for field in self.fields.values() {
            field.field().flush(window, cx);
        }
    }
}
impl Render for AgentSettingsPage {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        render::render(self, cx)
    }
}
