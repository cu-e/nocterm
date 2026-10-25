//! The form that edits one plugin's settings from its schema.

use std::{collections::BTreeMap, rc::Rc, time::Duration};

use gpui_kit::{
    AnyElement, AppContext as _, Context, Entity, IntoElement, Render, SharedString, Subscription,
    Task, TestSupportExt as _, Window,
    component::{
        ActiveTheme as _,
        input::{InputEvent, InputState},
        v_flex,
    },
    div,
    prelude::*,
};
use nocterm_plugin_model::{
    ContributionId, FieldEdit, FieldKind, FieldSpec, PluginId, SettingValue, SettingValues,
    SettingsSchema,
};

use super::{PluginSettingsError, PluginSettingsSource, SchemaRef, controls};
use crate::form;

/// How long typing must pause before a text value is sent.
const DEBOUNCE: Duration = Duration::from_millis(600);

/// Which groups of the schema the form shows.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FormScope {
    /// Every group of the plugin.
    Plugin,
    /// The groups of one contribution.
    Contribution(ContributionId),
}

/// The chrome around the form.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FormFrame {
    /// The form of a built-in plugin, drawn as part of the host page.
    Host,
    /// The form of a plugin that is not part of the host, labelled with the plugin's name.
    Plugin { name: SharedString },
}

/// A single-line input of a text field, with its event subscription.
struct TextInput {
    state: Entity<InputState>,
    _subscription: Subscription,
}

/// The controls for the settings of one plugin.
///
/// Every control sends a [`FieldEdit`] to the source and shows what the source reports. A
/// switch and a choice send the value shown; an option toggle, a step and text are applied by the
/// source to the value it holds when the write runs. The form never computes a number or a list
/// from its own copy.
pub struct PluginSettingsForm {
    source: Rc<dyn PluginSettingsSource>,
    plugin: PluginId,
    schema: SchemaRef,
    scope: FormScope,
    frame: FormFrame,
    values: SettingValues,
    errors: BTreeMap<String, SharedString>,
    /// Text fields typed into since their last send, with the timer that will send them.
    pending: BTreeMap<String, Task<()>>,
    inputs: BTreeMap<String, TextInput>,
    _subscription: Subscription,
}

impl PluginSettingsForm {
    pub fn new(
        source: Rc<dyn PluginSettingsSource>,
        plugin: PluginId,
        schema: SchemaRef,
        scope: FormScope,
        frame: FormFrame,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let values = source.values(cx);
        let mut inputs = BTreeMap::new();
        for field in fields_in(&schema, &scope) {
            if let FieldKind::Text { placeholder, .. } = &field.kind {
                let key = field.key.to_string();
                let text = text_of(&values, field);
                let placeholder = placeholder.as_deref().map(str::to_string);
                inputs.insert(key.clone(), text_input(key, text, placeholder, window, cx));
            }
        }
        let weak = cx.weak_entity();
        let window_handle = window.window_handle();
        let subscription = source.observe(
            cx,
            Box::new(move |cx| {
                let _ = window_handle.update(cx, |_, window, cx| {
                    let _ = weak.update(cx, |form, cx| form.sync(window, cx));
                });
            }),
        );
        Self {
            source,
            plugin,
            schema,
            scope,
            frame,
            values,
            errors: BTreeMap::new(),
            pending: BTreeMap::new(),
            inputs,
            _subscription: subscription,
        }
    }

    /// Sends every text value typed since its last send. A page calls it when it closes or
    /// loses focus, so the last words are not lost to the pause timer.
    pub fn flush(&mut self, cx: &mut Context<Self>) {
        let keys: Vec<String> = self.pending.keys().cloned().collect();
        for key in keys {
            self.commit(&key, cx);
        }
    }

    /// Sends one edit and shows why it failed, if it did.
    pub(super) fn edit(&mut self, key: &str, edit: FieldEdit, cx: &mut Context<Self>) {
        self.errors.remove(key);
        let written = self.source.write(key, edit, cx);
        let key = key.to_string();
        cx.spawn(async move |form, cx| {
            let result = written.await;
            let _ = form.update(cx, |form, cx| form.settled(key, result, cx));
        })
        .detach();
        cx.notify();
    }

    /// Asks the source to move a number field one step; the source steps from the value it
    /// holds when the write runs. A field already at that bound sends nothing.
    pub(super) fn nudge(&mut self, key: &str, up: bool, cx: &mut Context<Self>) {
        let Some(field) = self.schema.field(key) else {
            return;
        };
        let FieldKind::Number { min, max, .. } = &field.kind else {
            return;
        };
        let shown = field.current(&self.source.values(cx)).as_number();
        let at_bound = shown.is_some_and(|number| if up { number >= *max } else { number <= *min });
        if !at_bound {
            self.edit(key, FieldEdit::Step { up }, cx);
        }
    }

    fn settled(
        &mut self,
        key: String,
        result: Result<(), PluginSettingsError>,
        cx: &mut Context<Self>,
    ) {
        self.values = self.source.values(cx);
        if let Err(error) = result {
            self.errors.insert(key, error.to_string().into());
        }
        cx.notify();
    }

    /// Shows what the source holds now, leaving alone text the user is still typing and text the
    /// source refused (it stays beside its error until the user edits it again).
    fn sync(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.values = self.source.values(cx);
        for (key, input) in &self.inputs {
            let Some(field) = self.schema.field(key) else {
                continue;
            };
            let stored = text_of(&self.values, field);
            let held = self.pending.contains_key(key) || self.errors.contains_key(key);
            if !held && input.state.read(cx).value() != stored.as_str() {
                input
                    .state
                    .update(cx, |state, cx| state.set_value(stored, window, cx));
            }
        }
        cx.notify();
    }

    fn typed(
        &mut self,
        key: &str,
        event: &InputEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            InputEvent::Change => {
                self.errors.remove(key);
                if self.typed_text(key, cx) == self.stored_text(key) {
                    self.pending.remove(key);
                    return;
                }
                let key = key.to_string();
                let sent = key.clone();
                let timer = cx.spawn_in(window, async move |form, cx| {
                    cx.background_executor().timer(DEBOUNCE).await;
                    let _ = form.update(cx, |form, cx| form.commit(&sent, cx));
                });
                self.pending.insert(key, timer);
            }
            InputEvent::Blur | InputEvent::PressEnter { .. } => self.commit(key, cx),
            InputEvent::Focus => {}
        }
    }

    /// Sends the text typed into `key`, once, if it differs from what is stored.
    fn commit(&mut self, key: &str, cx: &mut Context<Self>) {
        if self.pending.remove(key).is_none() {
            return;
        }
        let text = self.typed_text(key, cx);
        if text != self.stored_text(key) {
            self.edit(key, FieldEdit::Set(SettingValue::Text(text)), cx);
        }
    }

    fn typed_text(&self, key: &str, cx: &Context<Self>) -> String {
        self.inputs
            .get(key)
            .map(|input| input.state.read(cx).value().to_string())
            .unwrap_or_default()
    }

    fn stored_text(&self, key: &str) -> String {
        self.schema
            .field(key)
            .map(|field| text_of(&self.values, field))
            .unwrap_or_default()
    }

    fn field_row(&self, field: &FieldSpec, cx: &mut Context<Self>) -> AnyElement {
        let key = field.key.to_string();
        let value = field.current(&self.values);
        let id = |part: &str| controls::control_id(&self.plugin, &key, part);
        let row = match (&field.kind, &value) {
            (FieldKind::Toggle { .. }, SettingValue::Bool(on)) => {
                controls::toggle_row(field, id(""), *on, cx)
            }
            (FieldKind::Choice { options, .. }, SettingValue::Choice(current)) => {
                controls::choice_row(field, options, current, &self.plugin, cx)
            }
            (FieldKind::MultiChoice { options, .. }, SettingValue::Choices(chosen)) => {
                controls::multi_row(field, options, chosen, &self.plugin, cx)
            }
            (FieldKind::Number { unit, .. }, SettingValue::Number(current)) => {
                controls::number_row(field, *current, unit.as_deref(), &self.plugin, cx)
            }
            (FieldKind::Text { .. }, SettingValue::Text(_)) => {
                let input = self.inputs.get(&key).map(|input| &input.state);
                controls::text_row(field, id("input"), input, cx)
            }
            _ => controls::mismatch_row(field, cx),
        };
        match self.errors.get(&key) {
            Some(error) => v_flex()
                .child(row)
                .child(
                    div()
                        .id(id("error"))
                        .aria_label(error.clone())
                        .test_support()
                        .child(form::error_text(error.clone(), cx)),
                )
                .into_any_element(),
            None => row,
        }
    }
}

impl Render for PluginSettingsForm {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let mut sections = Vec::new();
        for group in groups_in(&self.schema, &self.scope) {
            if group.fields.is_empty() {
                continue;
            }
            let rows: Vec<AnyElement> = group
                .fields
                .iter()
                .map(|field| self.field_row(field, cx))
                .collect();
            sections.push(form::section(group.title.to_string(), rows, cx));
        }
        let body = if sections.is_empty() {
            div()
                .id(SharedString::from(format!(
                    "plugin-settings-empty:{}",
                    self.plugin
                )))
                .aria_label("This item has no settings")
                .test_support()
                .child(form::note("This item has no settings", cx))
                .into_any_element()
        } else {
            v_flex().children(sections).into_any_element()
        };
        match &self.frame {
            FormFrame::Host => body,
            FormFrame::Plugin { name } => v_flex()
                .gap_2()
                .p_2()
                .border_1()
                .border_color(cx.theme().border)
                .rounded(cx.theme().radius)
                .child(
                    div()
                        .id(SharedString::from(format!(
                            "plugin-settings-frame:{}",
                            self.plugin
                        )))
                        .aria_label(format!("Plugin · {name}"))
                        .test_support()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child(format!("Plugin · {name}")),
                )
                .child(body)
                .into_any_element(),
        }
    }
}

fn groups_in<'a>(
    schema: &'a SettingsSchema,
    scope: &'a FormScope,
) -> Box<dyn Iterator<Item = &'a nocterm_plugin_model::FieldGroup> + 'a> {
    match scope {
        FormScope::Plugin => Box::new(schema.groups.iter()),
        FormScope::Contribution(id) => Box::new(schema.for_contribution(id)),
    }
}

fn fields_in<'a>(
    schema: &'a SettingsSchema,
    scope: &'a FormScope,
) -> impl Iterator<Item = &'a FieldSpec> + 'a {
    groups_in(schema, scope).flat_map(|group| group.fields.iter())
}

fn text_of(values: &SettingValues, field: &FieldSpec) -> String {
    match field.current(values) {
        SettingValue::Text(text) => text,
        _ => String::new(),
    }
}

fn text_input(
    key: String,
    text: String,
    placeholder: Option<String>,
    window: &mut Window,
    cx: &mut Context<PluginSettingsForm>,
) -> TextInput {
    let state = cx.new(|cx| {
        let state = InputState::new(window, cx).default_value(text);
        match placeholder {
            Some(placeholder) => state.placeholder(placeholder),
            None => state,
        }
    });
    let subscription = cx.subscribe_in(&state, window, move |form, _, event, window, cx| {
        form.typed(&key, event, window, cx);
    });
    TextInput {
        state,
        _subscription: subscription,
    }
}
