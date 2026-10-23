//! Checked immutable routing; providers are composed explicitly by the application.
use crate::{ProjectConnectionError, ProjectTerminalResolve, ProjectTerminalSession};
use gpui_kit::App;
use nocterm_project_model::{ProjectTarget, ProviderId};
use std::{collections::BTreeMap, rc::Rc};
pub struct ProjectProviderRegistry {
    direct: Rc<dyn ProjectTerminalResolve>,
    providers: BTreeMap<ProviderId, Rc<dyn ProjectTerminalResolve>>,
}
impl ProjectProviderRegistry {
    pub fn new(
        direct: Rc<dyn ProjectTerminalResolve>,
        providers: impl IntoIterator<Item = (ProviderId, Rc<dyn ProjectTerminalResolve>)>,
    ) -> Result<Self, ProjectConnectionError> {
        let mut registered = BTreeMap::new();
        for (id, provider) in providers {
            if registered.insert(id.clone(), provider).is_some() {
                return Err(ProjectConnectionError::DuplicateProvider(id));
            }
        }
        Ok(Self {
            direct,
            providers: registered,
        })
    }
}
impl ProjectTerminalResolve for ProjectProviderRegistry {
    fn resolve(
        &self,
        location: &ProjectTarget,
        cx: &App,
    ) -> Result<Rc<dyn ProjectTerminalSession>, ProjectConnectionError> {
        match location {
            ProjectTarget::HostFolder { .. } => self.direct.resolve(location, cx),
            ProjectTarget::Provider { provider, .. } => self
                .providers
                .get(provider)
                .ok_or(ProjectConnectionError::ProviderUnavailable)?
                .resolve(location, cx),
        }
    }
}
