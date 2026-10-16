//! A native text setting retains literal drafts until its original checked receipt resolves.
use std::{rc::Rc, time::Duration};

use futures::FutureExt as _;
use gpui_kit::{
    AnyElement, App, AppContext as _, Context, Entity, FocusHandle, Focusable, Render,
    SharedString, Subscription, Task, Window,
    component::{
        Sizable as _,
        input::{Input, InputEvent, InputState},
    },
    prelude::*,
};
use nocterm_settings::SettingsDocument;
use nocterm_store::{SettingsHandle, StoreError};

const DEBOUNCE: Duration = Duration::from_millis(600);

type Read = Rc<dyn Fn(&SettingsDocument) -> String>;
type Write = Rc<dyn Fn(&mut SettingsDocument, &str) -> Result<(), StoreError>>;

struct Bindings {
    read: Read,
    write: Write,
}

struct PendingSubmission {
    generation: u64,
    // A later publication can precede UI receipt delivery; same literal may then be a new intention.
    admission_revision: u64,
    text: String,
    receipt: ReceiptState,
}

enum ReceiptState {
    Waiting { _task: Task<()> },
    Settled,
    Rejected,
}

/// A private native input, checked submission and original settings observation.
#[derive(Clone)]
pub struct SettingsTextField(Entity<TextField>);

impl SettingsTextField {
    pub fn new(
        settings: SettingsHandle,
        id: impl Into<SharedString>,
        read: impl Fn(&SettingsDocument) -> String + 'static,
        write: impl Fn(&mut SettingsDocument, &str) -> Result<(), StoreError> + 'static,
        window: &mut Window,
        cx: &mut App,
    ) -> Self {
        let bindings = Bindings {
            read: Rc::new(read),
            write: Rc::new(write),
        };
        let id = id.into();
        Self(cx.new(|cx| TextField::new(settings, id, bindings, window, cx)))
    }

    /// Render the actual input without exporting its mutable entity.
    pub fn input(&self) -> AnyElement {
        self.0.clone().into_any_element()
    }

    pub fn focus_handle(&self, cx: &App) -> FocusHandle {
        self.0.read(cx).focus_handle(cx)
    }

    /// The diagnostic shown by the surrounding form.
    pub fn error(&self, cx: &App) -> Option<SharedString> {
        self.0.read(cx).error.as_ref().map(|error| match error {
            StoreError::Edit(cause) => cause.to_string().into(),
            error => format!("Could not save settings: {error}").into(),
        })
    }

    /// Retain this subscription with the form membership that presents the error.
    pub fn observe(
        &self,
        cx: &mut App,
        mut changed: impl FnMut(&mut App) + 'static,
    ) -> Subscription {
        cx.observe(&self.0, move |_, cx| changed(cx))
    }

    /// Synchronously admit the current distinct literal; the store owns accepted work.
    pub fn flush(&self, window: &mut Window, cx: &mut App) {
        self.0.update(cx, |this, cx| this.flush(window, cx));
    }

    pub fn flush_for_close(&self, window: &mut Window, cx: &mut App) {
        self.flush(window, cx);
    }

    pub fn deactivate(&self, window: &mut Window, cx: &mut App) {
        self.flush(window, cx);
    }
}

struct TextField {
    input: Entity<InputState>,
    settings: SettingsHandle,
    bindings: Bindings,
    confirmed: String,
    generation: u64,
    pending: Option<PendingSubmission>,
    debounce: Option<Task<()>>,
    error: Option<StoreError>,
    _input_subscription: Subscription,
    _settings_subscription: Subscription,
    programmatic_value: Option<String>,
    id: SharedString,
}

impl TextField {
    fn new(
        settings: SettingsHandle,
        id: SharedString,
        bindings: Bindings,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let confirmed = (bindings.read)(settings.document(cx));
        let input = cx.new(|cx| InputState::new(window, cx).default_value(confirmed.clone()));
        let input_subscription = cx.subscribe_in(&input, window, |this, _, event, window, cx| {
            this.event(event, window, cx);
        });
        let weak = cx.weak_entity();
        let original_window = window.window_handle();
        let settings_subscription = settings.observe(cx, move |cx| {
            let _ = original_window.update(cx, |_, window, cx| {
                let _ = weak.update(cx, |this, cx| this.sync(window, cx));
            });
        });
        Self {
            input,
            settings,
            bindings,
            confirmed,
            generation: 0,
            pending: None,
            debounce: None,
            error: None,
            _input_subscription: input_subscription,
            _settings_subscription: settings_subscription,
            programmatic_value: None,
            id,
        }
    }

    fn event(&mut self, event: &InputEvent, window: &mut Window, cx: &mut Context<Self>) {
        match event {
            InputEvent::Change => {
                let text = self.input.read(cx).value().to_string();
                if self.programmatic_value.take().as_deref() == Some(text.as_str()) {
                    return;
                }
                self.generation = self.generation.wrapping_add(1);
                self.error = None;
                self.debounce = Some(cx.spawn_in(window, async move |this, cx| {
                    cx.background_executor().timer(DEBOUNCE).await;
                    let _ = cx.update(|window, cx| {
                        let _ = this.update(cx, |this, cx| this.flush(window, cx));
                    });
                }));
                cx.notify();
            }
            InputEvent::PressEnter { .. } | InputEvent::Blur => self.flush(window, cx),
            InputEvent::Focus => {}
        }
    }

    fn unchanged(&self, text: &str, cx: &App) -> bool {
        match self.pending.as_ref() {
            Some(pending) => match pending.receipt {
                ReceiptState::Waiting { .. } => {
                    pending.text == text && pending.admission_revision == self.settings.revision(cx)
                }
                ReceiptState::Settled => self.error.is_none() && self.confirmed == text,
                ReceiptState::Rejected => false,
            },
            None => self.error.is_none() && self.confirmed == text,
        }
    }

    fn flush(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.debounce = None;
        let text = self.input.read(cx).value().to_string();
        if self.unchanged(&text, cx) {
            return;
        }
        self.generation = self.generation.wrapping_add(1);
        let generation = self.generation;
        // Retire older completion authority before any new local/admission failure.
        self.pending = Some(PendingSubmission {
            generation,
            admission_revision: self.settings.revision(cx),
            text: text.clone(),
            receipt: ReceiptState::Rejected,
        });
        let mut probe = self.settings.document(cx).clone();
        if let Err(error) = (self.bindings.write)(&mut probe, &text) {
            self.error = Some(error);
            self.confirmed = (self.bindings.read)(self.settings.document(cx));
            cx.notify();
            return;
        }
        let write = self.bindings.write.clone();
        let submitted = text.clone();
        let mut receipt = self
            .settings
            .edit_checked(cx, move |document| write(document, &submitted));
        if let Some(result) = (&mut receipt).now_or_never() {
            self.complete(generation, &text, result, window, cx);
        } else {
            let task = cx.spawn_in(window, async move |this, cx| {
                let result = receipt.await;
                let _ = cx.update(|window, cx| {
                    let _ = this.update(cx, |this, cx| {
                        this.complete(generation, &text, result, window, cx);
                    });
                });
            });
            if let Some(pending) = self.pending.as_mut() {
                pending.receipt = ReceiptState::Waiting { _task: task };
            }
        }
        cx.notify();
    }

    fn complete(
        &mut self,
        generation: u64,
        submitted: &str,
        result: Result<u64, StoreError>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(pending) = self.pending.as_mut() else {
            return;
        };
        if pending.generation != generation || pending.text != submitted {
            return;
        }
        pending.receipt = if result.is_ok() {
            ReceiptState::Settled
        } else {
            ReceiptState::Rejected
        };
        self.confirmed = (self.bindings.read)(self.settings.document(cx));
        if self.generation == generation && self.input.read(cx).value() == submitted {
            match result {
                Ok(_) => {
                    self.error = None;
                    self.show_confirmed(window, cx);
                }
                Err(error) => self.error = Some(error),
            }
        }
        cx.notify();
    }

    fn sync(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let pristine = self.input.read(cx).value() == self.confirmed.as_str();
        self.confirmed = (self.bindings.read)(self.settings.document(cx));
        let waiting = self
            .pending
            .as_ref()
            .is_some_and(|p| matches!(p.receipt, ReceiptState::Waiting { .. }));
        if !pristine || waiting || self.debounce.is_some() || self.error.is_some() {
            return;
        }
        self.show_confirmed(window, cx);
    }

    fn show_confirmed(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.input.read(cx).value() == self.confirmed.as_str() {
            return;
        }
        self.programmatic_value = Some(self.confirmed.clone());
        self.input.update(cx, |input, cx| {
            input.set_value(self.confirmed.clone(), window, cx)
        });
    }
}

impl Focusable for TextField {
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        self.input.read(cx).focus_handle(cx)
    }
}

impl Render for TextField {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        Input::new(&self.input).id(self.id.clone()).small()
    }
}

#[cfg(test)]
mod tests;
