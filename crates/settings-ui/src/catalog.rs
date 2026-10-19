//! Validated immutable page descriptors; constructing a catalog never invokes a factory.
use nocterm_ui::SettingsPageSpec;
use std::{collections::HashSet, rc::Rc};

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Lifecycle {
    Eager,
    Lazy,
}

#[derive(Clone)]
pub struct SettingsPageRegistration {
    spec: SettingsPageSpec,
    lifecycle: Lifecycle,
}
impl SettingsPageRegistration {
    pub fn eager(spec: SettingsPageSpec) -> Self {
        Self {
            spec,
            lifecycle: Lifecycle::Eager,
        }
    }
    pub fn lazy(spec: SettingsPageSpec) -> Self {
        Self {
            spec,
            lifecycle: Lifecycle::Lazy,
        }
    }
    pub(crate) fn spec(&self) -> &SettingsPageSpec {
        &self.spec
    }
    pub(crate) fn lifecycle(&self) -> Lifecycle {
        self.lifecycle
    }
}

#[derive(Debug, PartialEq, Eq, thiserror::Error)]
pub enum SettingsCatalogError {
    #[error("settings page at index {index} has an empty ID")]
    EmptyId { index: usize },
    #[error("duplicate settings page ID: {id}")]
    DuplicateId { id: &'static str },
    #[error("settings default page is absent: {id}")]
    MissingDefault { id: &'static str },
}

#[derive(Clone)]
pub struct SettingsCatalog {
    pages: Rc<[SettingsPageRegistration]>,
    default_index: usize,
}
impl SettingsCatalog {
    pub fn new(
        pages: Vec<SettingsPageRegistration>,
        default_id: &'static str,
    ) -> Result<Self, SettingsCatalogError> {
        let mut ids = HashSet::new();
        for (index, page) in pages.iter().enumerate() {
            let id = page.spec.id;
            if id.is_empty() {
                return Err(SettingsCatalogError::EmptyId { index });
            }
            if !ids.insert(id) {
                return Err(SettingsCatalogError::DuplicateId { id });
            }
        }
        let default_index = pages
            .iter()
            .position(|page| page.spec.id == default_id)
            .ok_or(SettingsCatalogError::MissingDefault { id: default_id })?;
        Ok(Self {
            pages: pages.into(),
            default_index,
        })
    }
    pub(crate) fn pages(&self) -> &[SettingsPageRegistration] {
        &self.pages
    }
    pub(crate) fn default_index(&self) -> usize {
        self.default_index
    }
}
