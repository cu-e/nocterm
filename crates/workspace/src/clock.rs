//! The Clock plugin: the local time in the status bar.
//!
//! The plugin is optional and off the Default arrangement (its widget waits in the tray until a
//! layout places it). The window creates the `ClockView` only while the layout places the widget
//! and the plugin is enabled, so the view and its timer exist exactly then. The time comes from
//! the [`WallClock`] the application gives to [`Deps::with_clock`](crate::Deps::with_clock); a
//! window composed without one has no clock widget.
//!
//! The plugin owns the `[clock]` settings section ([`ClockSettings`]: 24-hour or 12-hour time,
//! seconds, a short date) and its schema; [`plugin_settings`] binds the two for the Plugins
//! page and the layout editor. Every window follows a change at once.

use std::sync::Arc;

use gpui_kit::{AppContext as _, Context, Window};
use nocterm_core::clock::WallClock;
use nocterm_store::SettingsHandle;

use crate::{RegistrationError, WidgetRegistration, Workspace};

mod binding;
mod plugin;
mod settings;
pub(crate) mod view;

pub use binding::plugin_settings;
pub use plugin::{PLUGIN, TIME};
pub use settings::{ClockFormat, ClockSettings};

/// Registers the clock widget of `clock` in `workspace`; its views follow `settings`.
pub(crate) fn register(
    clock: Arc<dyn WallClock>,
    settings: SettingsHandle,
    workspace: &mut Workspace,
    window: &mut Window,
    cx: &mut Context<Workspace>,
) -> Result<(), RegistrationError> {
    settings.register::<ClockSettings>(cx);
    workspace.register_widget(
        WidgetRegistration::new(&TIME, move |_, _, cx| {
            let (clock, settings) = (clock.clone(), settings.clone());
            cx.new(|cx| view::ClockView::new(clock, settings, cx))
        }),
        window,
        cx,
    )
}

#[cfg(test)]
mod tests;
