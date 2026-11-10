//! Whether this run loaded the user plugins, and the switch that decides it for the next run.

use std::rc::Rc;

use gpui_kit::{App, Subscription, Task};

use super::port::UserPlugins;
use crate::EnablementError;

/// Why a run did not load the user plugins.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HeldBy {
    /// The run was started in safe mode. This wins over the switch.
    SafeMode,
    /// The user has not allowed user plugins.
    NotAllowed,
    /// The `[plugins]` table of the settings file does not read, so the switch cannot be told
    /// and nothing loads. Whatever the file says about the switch is not what decided.
    Unreadable {
        /// What was wrong with the table.
        reason: String,
    },
}

/// What this run did with the user plugins. It is fixed at the start: the switch changes the
/// next run, never this one.
#[derive(Clone)]
pub enum UserSession {
    /// The plugins were loaded and the section manages them through the port.
    Loaded(Rc<dyn UserPlugins>),
    /// Nothing was opened or loaded; the section shows why.
    Held(HeldBy),
}

impl UserSession {
    pub(super) fn loaded(&self) -> Option<&Rc<dyn UserPlugins>> {
        match self {
            Self::Loaded(plugins) => Some(plugins),
            Self::Held(_) => None,
        }
    }

    /// Whether this run loaded the plugins.
    pub(super) fn is_loaded(&self) -> bool {
        matches!(self, Self::Loaded(_))
    }
}

/// The stored choice to load user plugins at the start. Named for the User section's need;
/// the application implements it over the `[plugins]` settings section.
pub trait UserPluginsSwitch: 'static {
    /// Whether user plugins are allowed to load at the next start. A table that does not read
    /// means not allowed.
    fn allowed(&self, cx: &App) -> bool;

    /// Stores the choice for the next start. The task reports whether it was accepted and
    /// stored: only [`EnablementError::Malformed`] and [`EnablementError::Write`] occur.
    fn set_allowed(&self, allowed: bool, cx: &mut App) -> Task<Result<(), EnablementError>>;

    /// Calls `on_change` after the settings changed, whoever changed them. The choice itself may
    /// be unchanged, so the observer reads it again.
    fn observe(&self, cx: &mut App, on_change: Box<dyn FnMut(&mut App)>) -> Subscription;
}
