//! Compatibility access to explicitly installed service-layer settings.
use gpui_kit::{App, Global, Subscription, Task};
use nocterm_settings::{SectionError, SettingsDocument, SettingsFile, SettingsSection};
use nocterm_store::{SettingsConfig, SettingsHandle, StoreError};

/// Existing UI global identity; documents, revisions and workers live in store.
pub struct SettingsStore {
    initial: Option<SettingsConfig>,
    installed: Option<SettingsHandle>,
}
impl SettingsStore {
    pub fn new(document: SettingsDocument, file: SettingsFile) -> Self {
        Self::from_config(SettingsConfig::new(document, file))
    }
    pub fn in_memory(document: SettingsDocument) -> Self {
        Self::from_config(SettingsConfig::in_memory(document))
    }
    pub fn from_config(initial: SettingsConfig) -> Self {
        Self {
            initial: Some(initial),
            installed: None,
        }
    }
    /// Wrap an explicitly installed service during the UI-global migration.
    pub fn from_handle(installed: SettingsHandle) -> Self {
        Self {
            initial: None,
            installed: Some(installed),
        }
    }
    pub fn with<S: SettingsSection>(mut self, section: S) -> Self {
        self.initial = self.initial.take().map(|config| config.with(section));
        self
    }
    pub fn is_persistent(&self) -> bool {
        self.installed.as_ref().map_or_else(
            || {
                self.initial
                    .as_ref()
                    .is_some_and(SettingsConfig::is_persistent)
            },
            SettingsHandle::is_persistent,
        )
    }
    pub fn document<'a>(&'a self, cx: &'a App) -> &'a SettingsDocument {
        match &self.installed {
            Some(handle) => handle.document(cx),
            None => self
                .initial
                .as_ref()
                .expect("settings are configured")
                .document(),
        }
    }
    pub fn section_errors(&self, cx: &App) -> Vec<SectionError> {
        self.document(cx).errors()
    }
    pub fn revision(&self, cx: &App) -> u64 {
        self.installed
            .as_ref()
            .map_or(0, |handle| handle.revision(cx))
    }
    pub fn handle(&self) -> Option<&SettingsHandle> {
        self.installed.as_ref()
    }
}
impl Global for SettingsStore {}

pub(crate) fn init(cx: &mut App) -> SettingsHandle {
    let (initial, installed) = {
        let store = cx.global_mut::<SettingsStore>();
        (store.initial.take(), store.installed.clone())
    };
    let handle = installed.unwrap_or_else(|| {
        initial
            .expect("settings are configured before UI install")
            .install(cx)
    });
    cx.global_mut::<SettingsStore>().installed = Some(handle.clone());
    handle
        .observe(cx, |cx| {
            let _ = cx.global_mut::<SettingsStore>();
        })
        .detach();
    handle
}

pub fn register_setting<S: SettingsSection>(cx: &mut App) {
    let handle = cx
        .try_global::<SettingsStore>()
        .and_then(|store| store.handle().cloned());
    if let Some(handle) = handle {
        handle.register::<S>(cx);
    } else if cx.has_global::<SettingsStore>()
        && let Some(initial) = &mut cx.global_mut::<SettingsStore>().initial
    {
        initial.register::<S>();
    }
}

pub fn edit_settings(
    cx: &mut App,
    edit: impl FnOnce(&mut SettingsDocument) + 'static,
) -> Task<Result<u64, StoreError>> {
    let handle = cx
        .try_global::<SettingsStore>()
        .and_then(|store| store.handle().cloned());
    match handle {
        Some(handle) => handle.edit(cx, edit),
        None => Task::ready(Err(StoreError::CompletionDropped)),
    }
}

pub trait SettingsExt {
    fn setting<S: SettingsSection>(&self) -> &S;
    fn update_setting<S: SettingsSection>(
        &mut self,
        edit: impl FnOnce(&mut S) + 'static,
    ) -> Task<Result<u64, StoreError>>;
    fn observe_setting<S: SettingsSection>(
        &mut self,
        on_change: impl FnMut(&S, &mut App) + 'static,
    ) -> Subscription;
}
impl SettingsExt for App {
    fn setting<S: SettingsSection>(&self) -> &S {
        self.global::<SettingsStore>().document(self).get::<S>()
    }
    fn update_setting<S: SettingsSection>(
        &mut self,
        edit: impl FnOnce(&mut S) + 'static,
    ) -> Task<Result<u64, StoreError>> {
        edit_settings(self, move |document| {
            document.update::<S>(edit);
        })
    }
    fn observe_setting<S: SettingsSection>(
        &mut self,
        mut on_change: impl FnMut(&S, &mut App) + 'static,
    ) -> Subscription {
        let mut seen = self.setting::<S>().clone();
        self.observe_global::<SettingsStore>(move |cx| {
            let now = cx.setting::<S>();
            if *now != seen {
                seen = now.clone();
                on_change(&seen, cx);
            }
        })
    }
}

#[cfg(test)]
#[path = "settings/tests.rs"]
mod tests;
