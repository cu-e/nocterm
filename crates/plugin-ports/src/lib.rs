//! The ports through which a user plugin reaches the application.
//!
//! A plugin only ever sees a [`Snapshot`](nocterm_plugin_api::Snapshot) the host assembled and
//! only ever asks for [`Effects`](nocterm_plugin_api::Effects) the host applies. This crate
//! names the two trait families behind that: [`Observe`] reads one observation capability and
//! [`Apply`] performs one effect capability. A window's implementations are gathered in
//! [`PluginProviders`], an immutable record built with `with_*` methods in the application's
//! composition and handed to the host with each call.
//!
//! Owns: `Observe<C>`, `Apply<C>`, the `Surface` check of an effect's target contribution, and
//! `PluginProviders` with its dispatch from a capability id to the provider; in-memory providers
//! behind `test-support`.
//! Does not own: any provider (they live in the crate whose state they read), the broker, grants,
//! rate limits, records, or any view.
//! Ports used/provided: provides `Observe`, `Apply` and `Surface`; uses none.
//! Settings section: none. Globals: none.
//! Invariants: a provider can be sampled while any entity is updating, so it reads no entity at
//! sample time; a provider is passive, so it reads only state that already exists for the user's
//! own UI and never creates, keeps alive or widens a sampler, watch, subscription or connection;
//! a missing provider answers `unavailable`; only the application's composition calls
//! `PluginProviders::with_*`, and only the crates the isolation policy lists implement these
//! traits.

mod providers;
#[cfg(any(test, feature = "test-support"))]
pub mod test_support;

use gpui::App;
use nocterm_plugin_api::{Capability, HostError, LocalId};
use nocterm_plugin_model::PluginId;

pub use providers::PluginProviders;

/// Reads one observation capability for a plugin.
///
/// An implementation belongs to one window and answers from state that already exists for the
/// user's own UI. It answers `HostError::Unavailable` when that state does not exist.
///
/// The host samples from inside its own update, which a window can run from inside a render or a
/// click handler of any entity. An implementation therefore never reads an entity that may be
/// updating at that moment (GPUI panics on it): it answers from a value its owner pushed earlier,
/// for example through an observer.
pub trait Observe<C: Capability> {
    fn sample(&self, plugin: &PluginId, cx: &App) -> Result<C::Value, HostError>;
}

/// Performs one effect capability for a plugin, after the host checked the grant, the rate and
/// the gesture.
pub trait Apply<C: Capability> {
    fn apply(&self, plugin: &PluginId, request: C::Value, cx: &mut App) -> Result<(), HostError>;
}

/// Whether a plugin's own contribution may be acted on in this window now.
///
/// The host asks just before it applies an effect, once for the contribution whose event queued
/// it and once more for the contribution the effect names, if that is another. It runs from the
/// deferred apply, outside any entity update, so an implementation may read entities. It
/// answers `Unavailable` when the contribution is not registered in the window, its plugin is
/// disabled or the user did not approve placing it. The effect's own provider applies the window's
/// rules again when it performs the effect (a panel that asks for the user is `Denied` there),
/// since the window may change between the check and the apply.
pub trait Surface {
    fn permit(&self, plugin: &PluginId, local: &LocalId, cx: &App) -> Result<(), HostError>;
}

#[cfg(test)]
mod tests;
