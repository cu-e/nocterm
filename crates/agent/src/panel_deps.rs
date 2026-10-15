//! Immutable view collaborators supplied by the application composition root.
use crate::ImageDeps;
use nocterm_store::SettingsHandle;
use nocterm_ui::LayoutHandle;

#[derive(Clone)]
pub struct PanelDeps {
    images: ImageDeps,
    layout: LayoutHandle,
    settings: SettingsHandle,
}
impl PanelDeps {
    pub fn new(images: ImageDeps, layout: LayoutHandle, settings: SettingsHandle) -> Self {
        Self {
            images,
            layout,
            settings,
        }
    }
    pub(crate) fn into_parts(self) -> (ImageDeps, LayoutHandle, SettingsHandle) {
        (self.images, self.layout, self.settings)
    }
    pub(crate) fn settings(&self) -> &SettingsHandle {
        &self.settings
    }
}
