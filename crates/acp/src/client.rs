use std::sync::Arc;

use nocterm_ai::AgentConnector;

/// Runtime-neutral ACP connector. Each live subprocess has one protocol driver.
#[derive(Clone)]
pub struct AcpConnector {
    helper: Option<Arc<std::path::PathBuf>>,
    rules: Option<Arc<crate::managed::DiagnosticRedactor>>,
}
impl AcpConnector {
    pub fn managed(helper: std::path::PathBuf) -> Self {
        Self {
            helper: Some(Arc::new(helper)),
            rules: None,
        }
    }
    pub fn unmanaged() -> Self {
        Self {
            helper: None,
            rules: None,
        }
    }
    pub fn try_managed(helper: std::path::PathBuf) -> Result<Self, crate::DiagnosticBuildError> {
        Ok(Self {
            helper: Some(Arc::new(helper)),
            rules: Some(Arc::new(crate::managed::DiagnosticRedactor::new()?)),
        })
    }
    pub fn try_unmanaged() -> Result<Self, crate::DiagnosticBuildError> {
        Ok(Self {
            helper: None,
            rules: Some(Arc::new(crate::managed::DiagnosticRedactor::new()?)),
        })
    }
    pub fn connect_managed(
        &self,
        request: nocterm_ai::connection::ManagedConnectRequest,
    ) -> Result<
        nocterm_ai::history::source::ManagedSourceFuture<
            Result<
                nocterm_ai::connection::ManagedAgentConnection,
                nocterm_ai::history::source::ManagedCommandError,
            >,
        >,
        nocterm_ai::model::json::deferred::NativeAllocationError,
    > {
        crate::managed::ConnectionPreparation::new(self.helper.clone(), self.rules.clone())
            .operation(request)
    }
}

impl AgentConnector for AcpConnector {
    fn connect_managed(
        &self,
        request: nocterm_ai::connection::ManagedConnectRequest,
    ) -> Result<
        nocterm_ai::history::source::ManagedSourceFuture<
            Result<
                nocterm_ai::connection::ManagedAgentConnection,
                nocterm_ai::history::source::ManagedCommandError,
            >,
        >,
        nocterm_ai::model::json::deferred::NativeAllocationError,
    > {
        AcpConnector::connect_managed(self, request)
    }
}
