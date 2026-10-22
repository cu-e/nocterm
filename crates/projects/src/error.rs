#[derive(Clone, Debug, thiserror::Error)]
pub enum ProjectOpenError {
    #[error(transparent)]
    Connection(#[from] nocterm_workspace::ProjectConnectionError),
    #[error(transparent)]
    Parent(#[from] nocterm_workspace_ports::ProjectParentError),
    #[error(transparent)]
    Service(#[from] nocterm_project_service::ProjectsError),
    #[error(transparent)]
    Surface(#[from] nocterm_workspace::ContextSelectionError),
    #[error(transparent)]
    Validation(#[from] nocterm_project_model::ProjectValidationError),
    #[error("This workspace definition is no longer saved")]
    Removed,
}
