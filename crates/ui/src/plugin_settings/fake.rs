//! An in-memory source for tests of the form and of the views that host it.

use std::{cell::RefCell, rc::Rc};

use gpui_kit::{App, AppContext as _, Context, Entity, Subscription, Task};
use nocterm_plugin_model::{FieldEdit, SettingValue, SettingValues, SettingsSchema, apply};

use super::{PluginSettingsError, PluginSettingsSource};

struct Changed;

#[derive(Default)]
struct State {
    values: SettingValues,
    edits: Vec<(String, FieldEdit)>,
    refusal: Option<PluginSettingsError>,
}

/// A [`PluginSettingsSource`] that keeps its values in memory.
///
/// Like the real source it applies every edit to the value it holds at that moment. Clones
/// share the values, so a test keeps one and hands another to the form.
#[derive(Clone)]
pub struct FakePluginSettings {
    schema: &'static SettingsSchema,
    state: Rc<RefCell<State>>,
    changed: Entity<Changed>,
}

impl FakePluginSettings {
    /// A source holding the defaults of `schema`.
    pub fn new(schema: &'static SettingsSchema, cx: &mut App) -> Self {
        let state = State {
            values: schema.defaults(),
            ..State::default()
        };
        Self {
            schema,
            state: Rc::new(RefCell::new(state)),
            changed: cx.new(|_| Changed),
        }
    }

    /// Changes a value from outside the form, as another window or a hand edit would.
    pub fn set(&self, key: &str, value: SettingValue, cx: &mut App) {
        self.state.borrow_mut().values.insert(key, value);
        self.announce(cx);
    }

    /// Makes the next write fail with `error` and store nothing.
    pub fn refuse_next(&self, error: PluginSettingsError) {
        self.state.borrow_mut().refusal = Some(error);
    }

    /// Every edit the source received, accepted or not, in order.
    pub fn edits(&self) -> Vec<(String, FieldEdit)> {
        self.state.borrow().edits.clone()
    }

    fn announce(&self, cx: &mut App) {
        self.changed
            .update(cx, |_, cx: &mut Context<Changed>| cx.notify());
    }
}

impl PluginSettingsSource for FakePluginSettings {
    fn values(&self, _: &App) -> SettingValues {
        self.state.borrow().values.clone()
    }

    fn write(
        &self,
        key: &str,
        edit: FieldEdit,
        cx: &mut App,
    ) -> Task<Result<(), PluginSettingsError>> {
        let result = {
            let mut state = self.state.borrow_mut();
            state.edits.push((key.to_string(), edit.clone()));
            match state.refusal.take() {
                Some(error) => Err(error),
                None => apply(self.schema, &state.values, key, edit)
                    .map(|value| state.values.insert(key, value))
                    .map_err(PluginSettingsError::from),
            }
        };
        if result.is_ok() {
            self.announce(cx);
        }
        Task::ready(result)
    }

    fn observe(&self, cx: &mut App, mut on_change: Box<dyn FnMut(&mut App)>) -> Subscription {
        cx.observe(&self.changed, move |_, cx| on_change(cx))
    }
}
