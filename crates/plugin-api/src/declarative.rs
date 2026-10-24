//! The runtime of declarative packages: views are templates, and there is no code.

use alloc::{boxed::Box, collections::BTreeMap, string::String};

use crate::{
    Effects, Event, LocalId, Package, PluginInstance, PluginRuntime, RuntimeError, Snapshot, View,
    template::ViewTemplate,
};

/// Interprets the templates of a package.
#[derive(Clone, Copy, Debug, Default)]
pub struct DeclarativeRuntime;

struct Declarative {
    views: BTreeMap<String, ViewTemplate>,
}

impl PluginRuntime for DeclarativeRuntime {
    fn instantiate(&self, package: &Package) -> Result<Box<dyn PluginInstance>, RuntimeError> {
        let views = package
            .manifest()
            .views()
            .map(|(local, view)| (String::from(local), view.clone()))
            .collect();
        Ok(Box::new(Declarative { views }))
    }
}

impl PluginInstance for Declarative {
    fn render(&self, local: &LocalId, snapshot: &Snapshot) -> Result<View, RuntimeError> {
        let template = self
            .views
            .get(local.as_str())
            .ok_or_else(|| RuntimeError::UnknownContribution(local.clone()))?;
        template.render(snapshot).map_err(|error| match error {
            crate::template::RenderError::Invalid(view) => RuntimeError::View(view),
            crate::template::RenderError::TooManyNodes => {
                RuntimeError::View(crate::ViewError::TooManyNodes)
            }
        })
    }

    /// A declarative package has no code, so no event produces an effect.
    fn on_event(
        &self,
        local: &LocalId,
        _event: Event,
        _snapshot: &Snapshot,
    ) -> Result<Effects, RuntimeError> {
        if self.views.contains_key(local.as_str()) {
            Ok(Effects::default())
        } else {
            Err(RuntimeError::UnknownContribution(local.clone()))
        }
    }
}
