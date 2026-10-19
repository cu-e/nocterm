//! Agent-owned settings page; runtime and native display probe retain one installation.
mod agents;
mod controls;
mod field;
mod render;
use gpui_kit::Entity;
use gpui_kit::component::input::InputState;
use nocterm_ai::sandbox::{Availability, SandboxProbe};
use std::collections::BTreeSet;
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
    probe: std::sync::Arc<dyn SandboxProbe>,
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

pub fn settings_page(installed: &crate::Installed) -> SettingsPageSpec {
    let installed = installed.clone();
    SettingsPageSpec::contextual("ai", "AI agents", move |context, window, cx| {
        cx.new(|cx| AgentSettingsPage::new(installed.clone(), context, window, cx))
    })
    .with_icon(nocterm_ui::IconName::Bot)
}
struct AgentSettingsPage {
    installed: crate::Installed,
    fields: BTreeMap<SharedString, nocterm_ui::FieldPresentation>,
    focus: FocusHandle,
    _subscriptions: Vec<Subscription>,
    feedback: SettingsPageFeedback,
    ai: AiPage,
}
impl AgentSettingsPage {
    fn settings(&self) -> &nocterm_store::SettingsHandle {
        self.installed.settings()
    }
    fn new(
        installed: crate::Installed,
        context: SettingsPageContext,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let weak = cx.weak_entity();
        let original_window = window.window_handle();
        let subscription = installed.settings().observe(cx, move |cx| {
            let _ = original_window.update(cx, |_, window, cx| {
                let _ = weak.update(cx, |this, cx| {
                    agents::sync(this, window, cx);
                    cx.notify();
                });
            });
        });
        let mut this = Self {
            ai: AiPage::new(installed.sandbox_probe()),
            installed,
            fields: BTreeMap::new(),
            focus: cx.focus_handle(),
            _subscriptions: vec![subscription],
            feedback: context.feedback().clone(),
        };
        agents::add_fields(&mut this, window, cx);
        this
    }
    /// Adds a text setting of section `S` saved under `key`.
    pub(super) fn add_field<S: SettingsSection>(
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
    pub(super) fn remove_fields(&mut self, prefix: &str) {
        self.fields.retain(|key, _| !key.starts_with(prefix));
    }
    pub(super) fn field(&self, key: &str) -> &SettingsTextField {
        self.fields[key].field()
    }

    pub(super) fn save<S: SettingsSection>(
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
