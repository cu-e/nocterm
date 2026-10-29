//! Actual independent normal, urgent and admitted-fact publication reserves.
use nocterm_cloud_model::{
    coordinator::{CommandReceiveBudget, IntakePoolLimits},
    journal::{CommandKind, PublicationBudget, PublicationWorkBudget},
};
use std::sync::Arc;

/// Explicit account/process DI. Normal/urgent publishers share their exact
/// receive pools; the finite cleanup reserve uses the same generation owner.
/// Snapshot construction has separate configured capacity and cannot consume
/// this reserve for observations of already admitted effects.
#[derive(Clone)]
pub struct CoordinatorPublicationBudgets {
    ordinary: Arc<dyn PublicationWorkBudget>,
    urgent: Arc<dyn PublicationWorkBudget>,
    cleanup: Arc<dyn PublicationWorkBudget>,
    receive: CommandReceiveBudget,
}
impl CoordinatorPublicationBudgets {
    pub fn new(receive: CommandReceiveBudget, cleanup: IntakePoolLimits) -> Self {
        Self {
            ordinary: Arc::new(PublicationBudget::ordinary(receive.clone())),
            urgent: Arc::new(PublicationBudget::urgent(receive.clone())),
            cleanup: Arc::new(PublicationBudget::maintenance(cleanup)),
            receive,
        }
    }
    pub fn receive(&self) -> CommandReceiveBudget {
        self.receive.clone()
    }
    pub fn ordinary(&self) -> &Arc<dyn PublicationWorkBudget> {
        &self.ordinary
    }
    pub fn urgent(&self) -> &Arc<dyn PublicationWorkBudget> {
        &self.urgent
    }
    pub fn cleanup(&self) -> &Arc<dyn PublicationWorkBudget> {
        &self.cleanup
    }
    pub(crate) fn for_command(&self, kind: Option<CommandKind>) -> &Arc<dyn PublicationWorkBudget> {
        if kind.is_some_and(Self::is_urgent) {
            self.urgent()
        } else {
            self.ordinary()
        }
    }
    pub(crate) fn is_urgent(kind: CommandKind) -> bool {
        kind.high_priority()
    }
}
