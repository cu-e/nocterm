//! The published settings, typed access to their sections and a bounded,
//! ordered disk writer.
use gpui_kit::{App, Global, Subscription, Task};
use nocterm_settings::{SectionError, SettingsDocument, SettingsFile, SettingsSection};

mod writer;
pub use writer::edit_settings;
pub(crate) use writer::init;

/// Published settings only. Queued changes do not notify observers.
pub struct SettingsStore {
    document: SettingsDocument,
    file: Option<SettingsFile>,
    revision: u64,
}
impl SettingsStore {
    pub fn new(document: SettingsDocument, file: SettingsFile) -> Self {
        Self {
            document,
            file: Some(file),
            revision: 0,
        }
    }
    pub fn in_memory(document: SettingsDocument) -> Self {
        Self {
            document,
            file: None,
            revision: 0,
        }
    }
    /// A store holding `section`, for tests and previews.
    pub fn with<S: SettingsSection>(mut self, section: S) -> Self {
        self.document.set(section);
        self
    }
    pub fn is_persistent(&self) -> bool {
        self.file.is_some()
    }
    pub fn document(&self) -> &SettingsDocument {
        &self.document
    }
    /// Sections of the file that could not be read and have not been
    /// rewritten since, and tables no section claims.
    pub fn section_errors(&self) -> Vec<SectionError> {
        self.document.errors()
    }
    /// Advances every time a change is published.
    pub fn revision(&self) -> u64 {
        self.revision
    }
}
impl Global for SettingsStore {}

/// Reads section `S` from the file, once. The crate that owns a section
/// registers it when it is initialised; without a settings store this does
/// nothing.
pub fn register_setting<S: SettingsSection>(cx: &mut App) {
    if cx.has_global::<SettingsStore>()
        && !cx.global::<SettingsStore>().document.is_registered::<S>()
    {
        cx.global_mut::<SettingsStore>().document.register::<S>();
    }
}

/// Typed access to the published settings.
pub trait SettingsExt {
    /// Section `S` as last published.
    fn setting<S: SettingsSection>(&self) -> &S;

    /// Changes section `S` and saves it. Changes are applied in order to the
    /// settings current when each runs, so they never conflict.
    fn update_setting<S: SettingsSection>(
        &mut self,
        edit: impl FnOnce(&mut S) + 'static,
    ) -> Task<Result<u64, String>>;

    /// Calls `on_change` whenever a published change alters section `S`.
    /// Changes to other sections, and subscribing, do not call it.
    fn observe_setting<S: SettingsSection>(
        &mut self,
        on_change: impl FnMut(&S, &mut App) + 'static,
    ) -> Subscription;
}

impl SettingsExt for App {
    fn setting<S: SettingsSection>(&self) -> &S {
        self.global::<SettingsStore>().document.get::<S>()
    }

    fn update_setting<S: SettingsSection>(
        &mut self,
        edit: impl FnOnce(&mut S) + 'static,
    ) -> Task<Result<u64, String>> {
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

fn publish(document: SettingsDocument, cx: &mut App) -> u64 {
    let store = cx.global_mut::<SettingsStore>();
    store.document = document;
    store.revision += 1;
    store.revision
}

#[cfg(test)]
#[path = "settings/tests.rs"]
mod tests;
