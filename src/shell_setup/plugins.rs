//! Switching system plugins on and off from the Plugins page: the `[plugins]` section is the
//! only state, and the window shell follows it.
use std::sync::Arc;

use gpui_kit::{App, Subscription, Task};
use nocterm_core::SharedCause;
use nocterm_plugin_enablement::PluginEnablement;
use nocterm_plugin_model::{PluginCatalog, PluginId};
use nocterm_plugins_ui::{EnablementError, PluginToggles};
use nocterm_store::{SettingsHandle, StoreError};

/// The Plugins page's port over the `[plugins]` settings section.
pub(crate) struct AppPluginToggles {
    catalog: Arc<PluginCatalog>,
    handle: SettingsHandle,
}

impl AppPluginToggles {
    pub(crate) fn new(catalog: Arc<PluginCatalog>, handle: SettingsHandle, cx: &mut App) -> Self {
        handle.register::<PluginEnablement>(cx);
        Self { catalog, handle }
    }
}

impl PluginToggles for AppPluginToggles {
    fn enabled(&self, plugin: &PluginId, cx: &App) -> bool {
        !self
            .handle
            .setting::<PluginEnablement>(cx)
            .effective_disabled(&self.catalog)
            .contains(plugin)
    }

    fn set_enabled(
        &self,
        plugin: &PluginId,
        enabled: bool,
        cx: &mut App,
    ) -> Task<Result<(), EnablementError>> {
        let Some(known) = self.catalog.plugin(plugin) else {
            return Task::ready(Err(EnablementError::Unknown(plugin.clone())));
        };
        if known.required && !enabled {
            return Task::ready(Err(EnablementError::Required(plugin.clone())));
        }
        if let Some(reason) = PluginEnablement::read_error(self.handle.document(cx)) {
            return Task::ready(Err(EnablementError::Malformed { reason }));
        }
        let id = plugin.clone();
        let stored = self.handle.edit_checked(cx, move |document| {
            document.register::<PluginEnablement>();
            if let Some(reason) = PluginEnablement::read_error(document) {
                return Err(StoreError::edit(MalformedPlugins(reason)));
            }
            document.update::<PluginEnablement>(|section| section.set_enabled(&id, enabled));
            Ok(())
        });
        cx.spawn(async move |_| stored.await.map(drop).map_err(refusal))
    }

    fn observe(&self, cx: &mut App, on_change: Box<dyn FnMut(&mut App)>) -> Subscription {
        self.handle.observe(cx, on_change)
    }
}

/// The table stopped reading between the check and the ordered write.
#[derive(Debug, thiserror::Error)]
#[error("the [plugins] table does not read: {0}")]
struct MalformedPlugins(String);

fn refusal(error: StoreError) -> EnablementError {
    if let StoreError::Edit(cause) = &error
        && let Some(MalformedPlugins(reason)) = cause.downcast_ref()
    {
        return EnablementError::Malformed {
            reason: reason.clone(),
        };
    }
    EnablementError::Write(SharedCause::new(error))
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui_kit::TestAppContext;
    use nocterm_settings::SettingsDocument;
    use nocterm_store::SettingsConfig;

    fn toggles(text: &str, cx: &mut TestAppContext) -> (AppPluginToggles, SettingsHandle) {
        cx.update(|cx| {
            let document = SettingsDocument::from_table(text.parse().expect("toml"));
            let handle = SettingsConfig::in_memory(document).install(cx);
            let catalog = crate::shell_setup::plugin_catalog().expect("the application catalog");
            (
                AppPluginToggles::new(Arc::new(catalog), handle.clone(), cx),
                handle,
            )
        })
    }

    fn switch(
        toggles: &AppPluginToggles,
        plugin: &'static str,
        enabled: bool,
        cx: &mut TestAppContext,
    ) -> Result<(), EnablementError> {
        let task = cx.update(|cx| toggles.set_enabled(&PluginId::new(plugin), enabled, cx));
        cx.run_until_parked();
        cx.foreground_executor().block_test(task)
    }

    fn stored(handle: &SettingsHandle, cx: &mut TestAppContext) -> Vec<PluginId> {
        cx.update(|cx| handle.setting::<PluginEnablement>(cx).disabled.clone())
    }

    #[gpui_kit::test]
    fn an_optional_plugin_is_stored_off_and_on(cx: &mut TestAppContext) {
        let (toggles, handle) = toggles("", cx);
        assert_eq!(switch(&toggles, "monitor", false, cx).ok(), Some(()));
        assert_eq!(stored(&handle, cx), [PluginId::new("monitor")]);
        assert!(!cx.update(|cx| toggles.enabled(&PluginId::new("monitor"), cx)));
        assert_eq!(switch(&toggles, "monitor", true, cx).ok(), Some(()));
        assert!(stored(&handle, cx).is_empty());
    }

    #[gpui_kit::test]
    fn a_required_plugin_cannot_be_switched_off_and_nothing_is_written(cx: &mut TestAppContext) {
        let (toggles, handle) = toggles("", cx);
        let refused = switch(&toggles, "agent", false, cx);
        assert!(matches!(refused, Err(EnablementError::Required(id)) if id.as_str() == "agent"));
        assert!(stored(&handle, cx).is_empty());
        assert!(cx.update(|cx| toggles.enabled(&PluginId::new("agent"), cx)));
    }

    #[gpui_kit::test]
    fn an_unknown_plugin_is_refused_and_nothing_is_written(cx: &mut TestAppContext) {
        let (toggles, handle) = toggles("", cx);
        let refused = switch(&toggles, "nobody", false, cx);
        assert!(matches!(refused, Err(EnablementError::Unknown(id)) if id.as_str() == "nobody"));
        assert!(stored(&handle, cx).is_empty());
    }

    #[gpui_kit::test]
    fn a_table_that_does_not_read_is_refused_and_left_as_written(cx: &mut TestAppContext) {
        let broken = "[plugins]\ndisabled = [\"monitor\", 3]\n";
        let (toggles, handle) = toggles(broken, cx);
        let refused = switch(&toggles, "containers", false, cx);
        assert!(matches!(refused, Err(EnablementError::Malformed { .. })));
        let raw = cx.update(|cx| handle.document(cx).to_table().to_string());
        assert!(raw.contains("disabled = [\"monitor\", 3]"), "{raw}");
    }
}
