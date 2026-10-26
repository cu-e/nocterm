//! Captured original terminal destinations and provider contracts below the shell.
use crate::{ExpectedSession, TerminalCapabilities};
use gpui::{AnyEntity, AnyView, App, Entity, Window};
use nocterm_project_model::ProjectTarget;
use std::rc::Rc;

mod error;
pub use error::{ContextSelectionError, ProjectConnectionError};

/// A captured destination whose original authority can be checked without shell types.
pub trait ProjectTerminalDestination: 'static {
    fn validate(&self, window: &Window, cx: &App) -> Result<(), ContextSelectionError>;
}

/// Retains a captured destination and its type-bound validator.
/// Shell admission separately checks its own concrete destination type.
pub struct ProjectTerminalTarget {
    destination: AnyEntity,
    validate: fn(&AnyEntity, &Window, &App) -> Result<(), ContextSelectionError>,
}
impl ProjectTerminalTarget {
    pub fn new<T: ProjectTerminalDestination>(destination: Entity<T>) -> Self {
        Self {
            destination: destination.into(),
            validate: |destination, window, cx| {
                destination
                    .clone()
                    .downcast::<T>()
                    .map_err(|_| ContextSelectionError::StaleSurface)?
                    .read(cx)
                    .validate(window, cx)
            },
        }
    }
    pub fn validate(&self, window: &Window, cx: &App) -> Result<(), ContextSelectionError> {
        (self.validate)(&self.destination, window, cx)
    }
    /// Opaque captured identity for the supplying shell's typed admission bridge.
    pub fn destination_entity(&self) -> AnyEntity {
        self.destination.clone()
    }
}

/// Captures an ordinary parent view, its advertised operations and exact session epoch.
pub trait ProjectTerminalOpen: 'static {
    fn capture(
        &self,
        parent: AnyView,
        expected: ExpectedSession,
        capabilities: TerminalCapabilities,
        location: &ProjectTarget,
        cx: &App,
    ) -> Result<Rc<dyn ProjectTerminalSession>, ContextSelectionError>;
}

/// A captured connection launch configuration, independent of parent tab lifetime.
pub trait ProjectTerminalSession: 'static {
    fn open(
        &self,
        target: ProjectTerminalTarget,
        window: &mut Window,
        cx: &mut App,
    ) -> Result<(), ContextSelectionError>;
}

/// Resolve a checked location without opening another connection tab.
pub trait ProjectTerminalResolve: 'static {
    fn resolve(
        &self,
        location: &ProjectTarget,
        cx: &App,
    ) -> Result<Rc<dyn ProjectTerminalSession>, ProjectConnectionError>;
}
