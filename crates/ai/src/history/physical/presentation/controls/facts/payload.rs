//! Whole immutable controls still visit every independently guarded child owner.
use crate::{
    connection::AgentInfo,
    model::{
        controls::Modes,
        retention::{
            CensusPermit, RetentionError,
            document::Publication,
            graph::{Count, OwnerGraph},
        },
    },
};
pub(in crate::history::physical::presentation::controls) trait ControlPayload {
    fn count_children(&self, count: &mut Count<'_, '_>) -> Result<(), RetentionError>;
    fn cleanup_children(&self, permit: &CensusPermit<'_>);
    fn adopt_children(&self, publication: &mut Publication<'_>);
}
impl ControlPayload for AgentInfo {
    fn count_children(&self, count: &mut Count<'_, '_>) -> Result<(), RetentionError> {
        for value in &self.auth_methods {
            value.id.count(count)?;
        }
        Ok(())
    }
    fn cleanup_children(&self, permit: &CensusPermit<'_>) {
        for value in &self.auth_methods {
            value.id.cleanup(permit);
        }
    }
    fn adopt_children(&self, publication: &mut Publication<'_>) {
        for value in &self.auth_methods {
            value.id.adopt(publication);
        }
    }
}
impl ControlPayload for Modes {
    fn count_children(&self, count: &mut Count<'_, '_>) -> Result<(), RetentionError> {
        self.current.count(count)?;
        if let Some(value) = &self.extensions {
            value.count(count)?;
        }
        for value in &self.choices {
            value.id.count(count)?;
            if let Some(value) = &value.extensions {
                value.count(count)?;
            }
        }
        Ok(())
    }
    fn cleanup_children(&self, permit: &CensusPermit<'_>) {
        self.current.cleanup(permit);
        if let Some(value) = &self.extensions {
            value.cleanup(permit);
        }
        for value in &self.choices {
            value.id.cleanup(permit);
            if let Some(value) = &value.extensions {
                value.cleanup(permit);
            }
        }
    }
    fn adopt_children(&self, publication: &mut Publication<'_>) {
        self.current.adopt(publication);
        if let Some(value) = &self.extensions {
            value.adopt(publication);
        }
        for value in &self.choices {
            value.id.adopt(publication);
            if let Some(value) = &value.extensions {
                value.adopt(publication);
            }
        }
    }
}
