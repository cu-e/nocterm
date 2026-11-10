//! Current builtin projections are optional; unknown providers remain saved.
use nocterm_project_model::{HostLocator, ProjectTarget, RootPath};
use nocterm_project_providers::ContainerLocation;
pub(crate) fn host(target: &ProjectTarget) -> Option<HostLocator> {
    match target {
        ProjectTarget::HostFolder { host, .. } => Some(host.clone()),
        ProjectTarget::Provider { .. } => ContainerLocation::from_target(target)
            .ok()
            .map(|location| location.host().clone()),
    }
}
pub(crate) fn root(target: &ProjectTarget) -> Option<RootPath> {
    match target {
        ProjectTarget::HostFolder { root, .. } => Some(root.clone()),
        ProjectTarget::Provider { .. } => ContainerLocation::from_target(target)
            .ok()
            .map(|location| location.root().clone()),
    }
}
pub(crate) fn label(target: &ProjectTarget) -> String {
    root(target)
        .map(|root| root.as_str().to_owned())
        .unwrap_or_else(|| match target {
            ProjectTarget::Provider { provider, .. } => provider.to_string(),
            ProjectTarget::HostFolder { .. } => "Workspace".to_owned(),
        })
}
