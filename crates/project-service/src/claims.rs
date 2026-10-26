use std::collections::BTreeMap;

use gpui::{Context, EntityId, WindowId};
use nocterm_project_model::{Catalog, ProjectId};

use crate::ProjectsError;

/// One original owning window for a saved folder workspace.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ProjectClaim {
    project: ProjectId,
    window: WindowId,
    generation: u64,
    owner: EntityId,
}

impl ProjectClaim {
    pub fn project(self) -> ProjectId {
        self.project
    }

    pub fn window(self) -> WindowId {
        self.window
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ClaimOutcome {
    Acquired(ProjectClaim),
    Retained(ProjectClaim),
    FocusOwner {
        project: ProjectId,
        window: WindowId,
    },
}

pub(crate) struct ProjectClaims {
    claims: BTreeMap<ProjectId, ProjectClaim>,
    generation: u64,
    closing: bool,
    identity: EntityId,
}

impl ProjectClaims {
    pub(crate) fn new(cx: &mut Context<Self>) -> Self {
        Self {
            claims: BTreeMap::new(),
            generation: 0,
            closing: false,
            identity: cx.entity_id(),
        }
    }

    pub(crate) fn claim(
        &mut self,
        project: ProjectId,
        window: WindowId,
        cx: &mut Context<Self>,
    ) -> Result<ClaimOutcome, ProjectsError> {
        if self.closing {
            return Err(ProjectsError::Closing);
        }
        if let Some(original) = self.claims.get(&project) {
            return Ok(if original.window == window {
                ClaimOutcome::Retained(*original)
            } else {
                ClaimOutcome::FocusOwner {
                    project,
                    window: original.window,
                }
            });
        }
        let generation = self
            .generation
            .checked_add(1)
            .ok_or(ProjectsError::RevisionExhausted)?;
        let claim = ProjectClaim {
            project,
            window,
            generation,
            owner: self.identity,
        };
        self.generation = generation;
        self.claims.insert(project, claim);
        cx.notify();
        Ok(ClaimOutcome::Acquired(claim))
    }

    pub(crate) fn release(
        &mut self,
        claim: ProjectClaim,
        cx: &mut Context<Self>,
    ) -> Result<(), ProjectsError> {
        if self.claims.get(&claim.project) != Some(&claim) {
            return Err(ProjectsError::StaleClaim(claim.project));
        }
        self.claims.remove(&claim.project);
        cx.notify();
        Ok(())
    }

    pub(crate) fn check(&self, claim: ProjectClaim) -> Result<(), ProjectsError> {
        if self.closing {
            return Err(ProjectsError::Closing);
        }
        if self.claims.get(&claim.project) != Some(&claim) {
            return Err(ProjectsError::StaleClaim(claim.project));
        }
        Ok(())
    }

    pub(crate) fn owner(
        &self,
        project: ProjectId,
        window: WindowId,
    ) -> Result<ProjectClaim, ProjectsError> {
        let claim = self
            .claims
            .get(&project)
            .copied()
            .ok_or(ProjectsError::StaleClaim(project))?;
        self.check(claim)?;
        if claim.window != window {
            return Err(ProjectsError::StaleClaim(project));
        }
        Ok(claim)
    }

    pub(crate) fn published(&mut self, catalog: &Catalog, cx: &mut Context<Self>) {
        let previous = self.claims.len();
        self.claims
            .retain(|project, _| catalog.get(*project).is_some());
        if self.claims.len() != previous {
            cx.notify();
        }
    }

    pub(crate) fn close(&mut self, cx: &mut Context<Self>) {
        self.closing = true;
        self.claims.clear();
        cx.notify();
    }
}
