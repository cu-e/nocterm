//! The User section of the Plugins page: the installed user plugins, the install and update
//! consent, enabling, placement, removal and the audit log.
//!
//! The section owns what the user sees and decides. What a package is, how it is checked,
//! stored and run belongs to the application behind [`UserPlugins`].

mod consent;
mod consent_view;
mod port;
mod section;
mod section_view;

pub use port::{
    AuditLine, Candidate, CapabilityLine, FieldLine, InstallRequest, Replaced, RowState,
    UserPluginError, UserPluginRow, UserPlugins,
};
pub use section::UserPluginsSection;

#[cfg(test)]
mod tests;
