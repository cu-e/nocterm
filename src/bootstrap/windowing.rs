//! What the application knows about its windows: the hub servers a window can show and the
//! list of windows that are open, remembered for the next start.

use nocterm_core::Paths;
use nocterm_hubs_model::Environment;
use nocterm_window_set::{WindowSet, WindowSetError};

use gpui_kit::{App, WindowId};

/// The retained installations that belong to the set of windows rather than to one window.
#[derive(Clone)]
pub(crate) struct Windowing {
    hubs: Option<nocterm_hubs_ui::Installed>,
    /// Absent when its save could not be registered for shutdown: windows still work, they are
    /// just not remembered.
    window_set: Option<WindowSet>,
}

impl Windowing {
    pub(crate) fn install(
        hubs: Option<nocterm_hubs_ui::Installed>,
        stored: Option<&Paths>,
        shutdown: &nocterm_store::ShutdownCoordinator,
        cx: &mut App,
    ) -> Self {
        let (prepared, writer) = match stored {
            Some(paths) => crate::window_set_setup::native(paths),
            None => crate::window_set_setup::memory(),
        };
        let window_set = match WindowSet::install(prepared, writer, shutdown, cx) {
            Ok(set) => Some(set),
            Err(error @ WindowSetError::Registration(_)) => {
                tracing::error!(?error, "open windows will not be remembered");
                None
            }
        };
        Self { hubs, window_set }
    }

    /// Servers of nocterm hubs, when the application has them.
    pub(crate) fn hubs(&self) -> Option<&nocterm_hubs_ui::Installed> {
        self.hubs.as_ref()
    }

    /// The environments the previous run left open, in order.
    pub(crate) fn previous(&self) -> &[Environment] {
        self.window_set.as_ref().map_or(&[], WindowSet::previous)
    }

    /// A window opened showing `environment`.
    pub(crate) fn opened(&self, window: WindowId, environment: Environment, cx: &mut App) {
        if let Some(set) = &self.window_set {
            set.opened(window, environment, cx);
        }
    }

    /// An open window now shows `environment`.
    pub(crate) fn switched(&self, window: WindowId, environment: Environment, cx: &mut App) {
        if let Some(set) = &self.window_set {
            set.switched(window, environment, cx);
        }
    }

    /// The environments of the windows that are open now.
    #[cfg(test)]
    pub(crate) fn environments(&self, cx: &App) -> Vec<Environment> {
        self.window_set
            .as_ref()
            .map_or_else(Vec::new, |set| set.environments(cx))
    }
}
