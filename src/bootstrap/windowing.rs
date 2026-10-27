//! What the application knows about its windows: the hub servers a window can show and the
//! list of windows that are open, remembered for the next start.

use nocterm_core::Paths;
use nocterm_hubs_model::{Environment, HubId, HubWindowMemory, HubsSettings, WindowRecord};
use nocterm_settings::SettingsSection as _;
use nocterm_window_set::{WindowSet, WindowSetError};

use gpui_kit::{App, WindowId};

/// The retained installations that belong to the set of windows rather than to one window.
#[derive(Clone)]
pub(crate) struct Windowing {
    hubs: Option<nocterm_hubs_ui::Installed>,
    /// Absent when its save could not be registered for shutdown: windows still work, they are
    /// just not remembered.
    window_set: Option<WindowSet>,
    /// Whether the list of servers is known to be complete in this run: hubs started and the
    /// `[hubs]` section was read. Only then is a remembered server that is not in it removed.
    hub_list_known: bool,
}

impl Windowing {
    /// `hub_list_read` is false when the settings file could not be read, so `[hubs]` is empty
    /// only because of that.
    pub(crate) fn install(
        hubs: Option<nocterm_hubs_ui::Installed>,
        hub_list_read: bool,
        settings: &nocterm_store::SettingsHandle,
        stored: Option<&Paths>,
        shutdown: &nocterm_store::ShutdownCoordinator,
        cx: &mut App,
    ) -> Self {
        let hub_list_known = hubs.is_some()
            && hub_list_read
            && !settings
                .section_errors(cx)
                .iter()
                .any(|error| error.key == HubsSettings::KEY);
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
        Self {
            hubs,
            window_set,
            hub_list_known,
        }
    }

    /// Servers of nocterm hubs, when the application has them.
    pub(crate) fn hubs(&self) -> Option<&nocterm_hubs_ui::Installed> {
        self.hubs.as_ref()
    }

    /// The windows the previous run left open, in order, with what each kept of its hub.
    pub(crate) fn previous(&self) -> &[WindowRecord] {
        self.window_set.as_ref().map_or(&[], WindowSet::previous)
    }

    /// A remembered window could not open because its server is unknown. The server is
    /// forgotten when the list says it was removed; when the list is unavailable in this run
    /// the record stays in the document, so the next run can open it.
    pub(crate) fn not_restored(&self, record: WindowRecord, cx: &mut App) {
        if !self.hub_list_known
            && let Some(set) = &self.window_set
        {
            set.kept(record, cx);
        }
    }

    /// A window opened to show `record`: a new window, or one that comes back with what it kept
    /// of its hub.
    pub(crate) fn opened(&self, window: WindowId, record: WindowRecord, cx: &mut App) {
        if let Some(set) = &self.window_set {
            set.restored(window, record, cx);
        }
    }

    /// The window shows `hub` and now keeps `memory` of it.
    pub(crate) fn remembered(
        &self,
        window: WindowId,
        hub: &HubId,
        memory: HubWindowMemory,
        cx: &mut App,
    ) {
        if let Some(set) = &self.window_set {
            set.remembered(window, hub, memory, cx);
        }
    }

    /// An open window now shows `environment`.
    pub(crate) fn switched(&self, window: WindowId, environment: Environment, cx: &mut App) {
        if let Some(set) = &self.window_set {
            set.switched(window, environment, cx);
        }
    }

    /// The windows that are open now, with what each keeps of its hub.
    #[cfg(test)]
    pub(crate) fn records(&self, cx: &App) -> Vec<WindowRecord> {
        self.window_set
            .as_ref()
            .map_or_else(Vec::new, |set| set.records(cx))
    }

    /// The environments of the windows that are open now.
    #[cfg(test)]
    pub(crate) fn environments(&self, cx: &App) -> Vec<Environment> {
        self.window_set
            .as_ref()
            .map_or_else(Vec::new, |set| set.environments(cx))
    }
}
