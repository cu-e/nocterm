//! Membership owns both the shared field and its original form redraw subscription.
use crate::SettingsTextField;
use gpui_kit::{Context, Subscription, Window};

pub struct FieldPresentation {
    field: SettingsTextField,
    _notification: Subscription,
}

impl FieldPresentation {
    pub fn new<T: 'static>(field: SettingsTextField, window: &Window, cx: &mut Context<T>) -> Self {
        let weak = cx.weak_entity();
        let original_window = window.window_handle();
        let notification = field.observe(cx, move |cx| {
            let _ = original_window.update(cx, |_, _, cx| {
                let _ = weak.update(cx, |_, cx| cx.notify());
            });
        });
        Self {
            field,
            _notification: notification,
        }
    }

    pub fn field(&self) -> &SettingsTextField {
        &self.field
    }
}
