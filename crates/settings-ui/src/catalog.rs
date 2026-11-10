//! Validated immutable page descriptors; constructing a catalog never invokes a factory.
use nocterm_ui::SettingsPageSpec;
use std::{collections::HashSet, rc::Rc};

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Lifecycle {
    Eager,
    Lazy,
}

/// Where a page appears in the navigation.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Nav {
    /// An entry of the page list.
    Top,
    /// Hidden from the list; reached from `parent`, which keeps the highlight and shows a back
    /// link that reveals `anchor` there.
    Under {
        parent: &'static str,
        anchor: &'static str,
    },
}

#[derive(Clone)]
pub struct SettingsPageRegistration {
    spec: SettingsPageSpec,
    lifecycle: Lifecycle,
    nav: Nav,
}
impl SettingsPageRegistration {
    pub fn eager(spec: SettingsPageSpec) -> Self {
        Self {
            spec,
            lifecycle: Lifecycle::Eager,
            nav: Nav::Top,
        }
    }
    pub fn lazy(spec: SettingsPageSpec) -> Self {
        Self {
            spec,
            lifecycle: Lifecycle::Lazy,
            nav: Nav::Top,
        }
    }
    /// Nests the page under the top-level page `parent`: the navigation list omits it, and its
    /// back link returns to `parent` revealing `anchor`.
    pub fn under(mut self, parent: &'static str, anchor: &'static str) -> Self {
        self.nav = Nav::Under { parent, anchor };
        self
    }
    /// The id of the page.
    pub fn id(&self) -> &'static str {
        self.spec.id
    }
    /// The top-level page and the anchor this page is nested under, when it is nested.
    pub fn nested_under(&self) -> Option<(&'static str, &'static str)> {
        match self.nav {
            Nav::Top => None,
            Nav::Under { parent, anchor } => Some((parent, anchor)),
        }
    }
    pub(crate) fn spec(&self) -> &SettingsPageSpec {
        &self.spec
    }
    pub(crate) fn lifecycle(&self) -> Lifecycle {
        self.lifecycle
    }
    pub(crate) fn nav(&self) -> Nav {
        self.nav
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
    #[error("settings page {id} is nested under an unknown page: {parent}")]
    UnknownParent {
        id: &'static str,
        parent: &'static str,
    },
    #[error("settings page {id} is nested under {parent}, which is itself nested")]
    NestedParent {
        id: &'static str,
        parent: &'static str,
    },
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
        for page in &pages {
            let Nav::Under { parent, .. } = page.nav else {
                continue;
            };
            let id = page.spec.id;
            match pages.iter().find(|candidate| candidate.spec.id == parent) {
                None => return Err(SettingsCatalogError::UnknownParent { id, parent }),
                Some(found) if found.nav != Nav::Top => {
                    return Err(SettingsCatalogError::NestedParent { id, parent });
                }
                Some(_) => {}
            }
        }
        let default_index = pages
            .iter()
            .position(|page| page.spec.id == default_id && page.nav == Nav::Top)
            .ok_or(SettingsCatalogError::MissingDefault { id: default_id })?;
        Ok(Self {
            pages: pages.into(),
            default_index,
        })
    }
    /// The registered pages in navigation order, nested ones included.
    pub fn pages(&self) -> &[SettingsPageRegistration] {
        &self.pages
    }
    pub(crate) fn default_index(&self) -> usize {
        self.default_index
    }
}
