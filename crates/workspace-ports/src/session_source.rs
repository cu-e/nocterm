//! Original session source: descriptive partition plus an actual captured guard.
use nocterm_hubs_model::source_identity::LocalAccountPartition;
use nocterm_session::{ConnectionSource, ConnectionSourceError};
#[derive(Clone, Debug, Default)]
pub struct SessionSource {
    partition: Option<LocalAccountPartition>,
    connection: ConnectionSource,
}
impl SessionSource {
    /// Supplied by the source owner. Metadata alone grants no authority.
    pub fn new(partition: Option<LocalAccountPartition>, connection: ConnectionSource) -> Self {
        Self {
            partition,
            connection,
        }
    }
    pub fn this_computer() -> Self {
        Self::default()
    }
    pub fn partition(&self) -> Option<&LocalAccountPartition> {
        self.partition.as_ref()
    }
    pub fn connection(&self) -> &ConnectionSource {
        &self.connection
    }
    pub fn check_current(&self) -> Result<(), ConnectionSourceError> {
        self.connection.check()
    }
    pub fn same_as(&self, other: &Self) -> bool {
        self.partition == other.partition && self.connection.same_owner(&other.connection)
    }
}
