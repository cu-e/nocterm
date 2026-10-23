use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::{CatalogError, ParentConnectionKey, ProjectDefinition, ProjectDraft, ProjectId};

pub const MAX_CATALOG_BYTES: usize = 1024 * 1024;
pub const MAX_PROJECTS: usize = 256;
const VERSION: u32 = 2;

/// Bounded checked catalog; mutable transitions never publish a partial candidate.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Catalog {
    projects: BTreeMap<ProjectId, ProjectDefinition>,
}

/// Borrowed search results preserve current-parent membership and catalog identity.
#[derive(Debug)]
pub struct ProjectGroups<'a> {
    pub current: Vec<&'a ProjectDefinition>,
    pub other: Vec<&'a ProjectDefinition>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Document {
    version: u32,
    projects: Vec<ProjectDraft>,
}

#[derive(Serialize)]
struct BorrowedDocument<'a> {
    version: u32,
    projects: Vec<&'a ProjectDefinition>,
}

fn encode_bounded<'a>(
    projects: impl Iterator<Item = &'a ProjectDefinition>,
) -> Result<Vec<u8>, CatalogError> {
    let bytes = serde_json::to_vec(&BorrowedDocument {
        version: VERSION,
        projects: projects.collect(),
    })?;
    if bytes.len() > MAX_CATALOG_BYTES {
        return Err(CatalogError::TooLarge);
    }
    Ok(bytes)
}

#[derive(Deserialize)]
struct Version {
    version: u32,
}

impl Catalog {
    pub fn new(projects: Vec<ProjectDefinition>) -> Result<Self, CatalogError> {
        if projects.len() > MAX_PROJECTS {
            return Err(CatalogError::Capacity);
        }
        let mut catalog = Self::default();
        for project in projects {
            catalog.insert(project)?;
        }
        Ok(catalog)
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, CatalogError> {
        if bytes.len() > MAX_CATALOG_BYTES {
            return Err(CatalogError::TooLarge);
        }
        let version: Version = serde_json::from_slice(bytes)?;
        if !matches!(version.version, 1 | VERSION) {
            return Err(CatalogError::UnsupportedVersion(version.version));
        }
        let document: Document = serde_json::from_slice(bytes)?;
        if document.projects.len() > MAX_PROJECTS {
            return Err(CatalogError::Capacity);
        }
        let projects = document
            .projects
            .into_iter()
            .map(ProjectDefinition::try_from)
            .collect::<Result<Vec<_>, _>>()?;
        Self::new(projects)
    }

    pub fn encode(&self) -> Result<Vec<u8>, CatalogError> {
        encode_bounded(self.projects.values())
    }

    pub fn get(&self, id: ProjectId) -> Option<&ProjectDefinition> {
        self.projects.get(&id)
    }

    pub fn iter(&self) -> impl Iterator<Item = &ProjectDefinition> {
        self.projects.values()
    }

    pub fn partition(
        &self,
        parent: Option<&ParentConnectionKey>,
        query: &str,
    ) -> ProjectGroups<'_> {
        self.partition_by(parent, query, |target| {
            target
                .host_folder()
                .and_then(|(host, _)| host.parent_key().ok())
        })
    }

    /// Group known locations using an explicit pure projection supplied by their owner.
    pub fn partition_by(
        &self,
        parent: Option<&ParentConnectionKey>,
        query: &str,
        parent_for: impl Fn(&crate::ProjectTarget) -> Option<ParentConnectionKey>,
    ) -> ProjectGroups<'_> {
        let query = query.trim().to_lowercase();
        let mut current = Vec::new();
        let mut other = Vec::new();
        for project in self.projects.values() {
            let matches = query.is_empty()
                || project.name().to_lowercase().contains(&query)
                || matches!(project.target(), crate::ProjectTarget::Provider { provider, .. } if provider.as_str().contains(&query))
                || project.host_folder().is_some_and(|(host, root)| {
                    root.as_str().to_lowercase().contains(&query)
                        || host.address().is_some_and(|address| {
                            address.to_string().to_lowercase().contains(&query)
                        })
                });
            if !matches {
                continue;
            }
            if parent.is_some_and(|parent| {
                parent_for(project.target()).is_some_and(|key| &key == parent)
            }) {
                current.push(project);
            } else {
                other.push(project);
            }
        }
        current.sort_by_cached_key(|project| (project.name().to_lowercase(), project.id()));
        other.sort_by_cached_key(|project| (project.name().to_lowercase(), project.id()));
        ProjectGroups { current, other }
    }

    pub fn len(&self) -> usize {
        self.projects.len()
    }

    pub fn is_empty(&self) -> bool {
        self.projects.is_empty()
    }

    pub fn insert(&mut self, project: ProjectDefinition) -> Result<(), CatalogError> {
        let id = project.id();
        if self.projects.contains_key(&id) {
            return Err(CatalogError::DuplicateId(id));
        }
        if self.projects.len() == MAX_PROJECTS {
            return Err(CatalogError::Capacity);
        }
        encode_bounded(self.projects.values().chain(std::iter::once(&project)))?;
        self.projects.insert(id, project);
        Ok(())
    }

    pub fn replace(&mut self, project: ProjectDefinition) -> Result<(), CatalogError> {
        let id = project.id();
        if !self.projects.contains_key(&id) {
            return Err(CatalogError::Missing(id));
        }
        encode_bounded(self.projects.iter().map(|(existing_id, existing)| {
            if *existing_id == id {
                &project
            } else {
                existing
            }
        }))?;
        self.projects.insert(id, project);
        Ok(())
    }

    pub fn remove(&mut self, id: ProjectId) -> Result<ProjectDefinition, CatalogError> {
        self.projects.remove(&id).ok_or(CatalogError::Missing(id))
    }
}
