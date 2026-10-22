use std::collections::BTreeMap;

use crate::{Catalog, CatalogError, ProjectDefinition, ProjectId};

/// A supplied in-memory source generation, independent of persisted identity.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ProjectStamp(u64);

impl ProjectStamp {
    pub fn generation(self) -> u64 {
        self.0
    }
}

/// Checked edits preserve rename bindings and invalidate changed folder/host paths.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CatalogState {
    catalog: Catalog,
    generation: u64,
    stamps: BTreeMap<ProjectId, ProjectStamp>,
}

impl CatalogState {
    pub fn new(catalog: Catalog, generation: u64) -> Self {
        let stamps = catalog
            .iter()
            .map(|project| (project.id(), ProjectStamp(generation)))
            .collect();
        Self {
            catalog,
            generation,
            stamps,
        }
    }

    pub fn catalog(&self) -> &Catalog {
        &self.catalog
    }

    pub fn into_catalog(self) -> Catalog {
        self.catalog
    }

    pub fn generation(&self) -> u64 {
        self.generation
    }

    pub fn stamp(&self, id: ProjectId) -> Option<ProjectStamp> {
        self.stamps.get(&id).copied()
    }

    /// Expected values are checked against the actual ordered durable base.
    pub fn save(
        &mut self,
        candidate: ProjectDefinition,
        expected: Option<&ProjectDefinition>,
    ) -> Result<ProjectStamp, CatalogError> {
        let id = candidate.id();
        let original = self.catalog.get(id);
        if original != expected {
            return Err(CatalogError::Conflict(id));
        }
        if original == Some(&candidate) {
            return self.stamp(id).ok_or(CatalogError::Missing(id));
        }
        let generation = self.next_generation()?;
        let stamp = match (original, self.stamp(id)) {
            (Some(original), Some(stamp)) if same_location(original, &candidate) => stamp,
            _ => ProjectStamp(generation),
        };
        let mut catalog = self.catalog.clone();
        if original.is_some() {
            catalog.replace(candidate)?;
        } else {
            catalog.insert(candidate)?;
        }
        self.catalog = catalog;
        self.stamps.insert(id, stamp);
        self.generation = generation;
        Ok(stamp)
    }

    pub fn remove(&mut self, expected: &ProjectDefinition) -> Result<(), CatalogError> {
        let id = expected.id();
        if self.catalog.get(id) != Some(expected) {
            return Err(CatalogError::Conflict(id));
        }
        let generation = self.next_generation()?;
        self.catalog.remove(id)?;
        self.stamps.remove(&id);
        self.generation = generation;
        Ok(())
    }

    fn next_generation(&self) -> Result<u64, CatalogError> {
        self.generation
            .checked_add(1)
            .ok_or(CatalogError::RevisionExhausted)
    }
}

fn same_location(first: &ProjectDefinition, second: &ProjectDefinition) -> bool {
    first.host() == second.host() && first.root() == second.root()
}
