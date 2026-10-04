//! The page's state: the search, and the binding being recorded.
use gpui_kit::{
    App, AppContext as _, Context, Entity, FocusHandle, Focusable, KeystrokeEvent, SharedString,
    Subscription, Window,
    component::input::{InputEvent, InputState},
};
use nocterm_keymap::{Keymap, humanize};
use nocterm_workspace::SettingsPage;

use crate::rows::default_context;

/// What the next keystroke is for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Recording {
    /// A shortcut for `action` in `context`, replacing `replacing`.
    Bind {
        action: String,
        context: Option<String>,
        replacing: Option<String>,
    },
    /// The search: show what a shortcut runs.
    Search,
}

pub struct KeymapView {
    focus: FocusHandle,
    pub(crate) search: Entity<InputState>,
    pub(crate) recording: Option<Recording>,
    /// The result of the last change, or why it failed.
    pub(crate) message: Option<(SharedString, bool)>,
    interceptor: Option<Subscription>,
    _subscription: Subscription,
}

impl KeymapView {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let search = cx.new(|cx| {
            InputState::new(window, cx).placeholder("Search commands, descriptions or keys…")
        });
        let subscription = cx.subscribe(&search, |_, _, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Change) {
                cx.notify();
            }
        });
        Self {
            focus: cx.focus_handle(),
            search,
            recording: None,
            message: None,
            interceptor: None,
            _subscription: subscription,
        }
    }

    /// Waits for the next keystroke, which `recording` says what to do with.
    pub(crate) fn record(&mut self, recording: Recording, cx: &mut Context<Self>) {
        let view = cx.weak_entity();
        self.interceptor = Some(cx.intercept_keystrokes(
            move |event: &KeystrokeEvent, window, cx| {
                let keystroke = &event.keystroke;
                let modifier_only = matches!(
                    keystroke.key.as_str(),
                    "shift" | "control" | "alt" | "platform" | "function"
                );
                if modifier_only {
                    return;
                }
                cx.stop_propagation();
                let cancel = keystroke.key == "escape" && !keystroke.modifiers.modified();
                let keys = keystroke.unparse();
                let _ = view.update(cx, |view, cx| {
                    if cancel {
                        view.stop_recording(cx);
                    } else {
                        view.recorded(keys, window, cx);
                    }
                });
            },
        ));
        self.recording = Some(recording);
        self.message = None;
        cx.notify();
    }

    pub(crate) fn stop_recording(&mut self, cx: &mut Context<Self>) {
        self.interceptor = None;
        self.recording = None;
        cx.notify();
    }

    fn recorded(&mut self, keys: String, window: &mut Window, cx: &mut Context<Self>) {
        let Some(recording) = self.recording.take() else {
            return;
        };
        self.interceptor = None;
        match recording {
            Recording::Search => self
                .search
                .update(cx, |search, cx| search.set_value(keys, window, cx)),
            Recording::Bind {
                action,
                context,
                replacing,
            } => self.bind(&action, context, replacing.as_deref(), &keys, cx),
        }
        cx.notify();
    }

    /// Binds `keys` to `action` and says what else they run there.
    pub(crate) fn bind(
        &mut self,
        action: &str,
        context: Option<String>,
        replacing: Option<&str>,
        keys: &str,
        cx: &mut Context<Self>,
    ) {
        let others = Keymap::global(cx).conflicts(action, &context, keys);
        self.message = Some(match Keymap::bind(action, context, replacing, keys, cx) {
            Err(error) => (format!("Not saved: {error}").into(), true),
            Ok(()) if others.is_empty() => {
                (format!("{keys} runs {}.", humanize(action)).into(), false)
            }
            Ok(()) => (
                format!(
                    "{keys} now runs {} instead of {}.",
                    humanize(action),
                    others
                        .iter()
                        .map(|other| humanize(other))
                        .collect::<Vec<_>>()
                        .join(", ")
                )
                .into(),
                false,
            ),
        });
        cx.notify();
    }

    /// Starts recording a new shortcut for `action`.
    pub(crate) fn add(&mut self, action: &str, cx: &mut Context<Self>) {
        let context = default_context(action, &Keymap::global(cx).defaults());
        self.record(
            Recording::Bind {
                action: action.to_owned(),
                context,
                replacing: None,
            },
            cx,
        );
    }

    pub(crate) fn report(&mut self, result: Result<(), String>, cx: &mut Context<Self>) {
        self.message = result
            .err()
            .map(|error| (format!("Not saved: {error}").into(), true));
        cx.notify();
    }
}

impl Focusable for KeymapView {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl SettingsPage for KeymapView {
    fn on_deactivate(&mut self, _: &mut Window, cx: &mut Context<Self>) {
        self.stop_recording(cx);
    }
}
