//! The guest SDK for Rust code plugins: the Plugin trait, a view builder, typed reads of
//! observations and settings, effect constructors, the export! macro and testing::Host.
//!
//! Owns: guest-side dispatch of ABI calls and the test host that drives it.
//! Does not own: the ABI types (nocterm-plugin-abi), contract types (nocterm-plugin-api),
//!       running modules (nocterm-plugin-wasm).
//! Ports: none. Settings: none. Globals: none in this crate; export! expands one single-threaded
//!       slot in the author's crate, the only unsafe code on the guest side.
//! Invariants: no_std + alloc (feature `testing` adds std and toml); never in the app's
//!       dependency closure (xtask guest_only).
//!
//! A plugin is a type that implements [`Plugin`]. [`export!`] turns it into a WebAssembly
//! module that carries the manifest and the four exports of the ABI. Everything else is safe
//! code: [`Cx`] reads the snapshot of one call, [`ViewBuilder`] draws, [`effect`] builds the
//! effects a reply carries, and `testing::Host` (feature `testing`) runs the same dispatch the
//! module runs, natively and with the real encoding.
//!
//! ```ignore
//! use nocterm_plugin_sdk::{Cx, Plugin, ViewBuilder, api::{Effects, Event, View}};
//!
//! #[derive(Default)]
//! struct Hello;
//!
//! impl Plugin for Hello {
//!     fn render(&mut self, _cx: &mut Cx<'_>) -> View {
//!         let mut view = ViewBuilder::new();
//!         view.text(None, "hello");
//!         view.build()
//!     }
//! }
//!
//! nocterm_plugin_sdk::export!(Hello, concat!(env!("CARGO_MANIFEST_DIR"), "/plugin.toml"));
//! ```
#![no_std]

extern crate alloc;
#[cfg(any(test, feature = "testing"))]
extern crate std;

mod cx;
mod dispatch;
pub mod effect;
mod export;
pub mod observe;
mod view;

#[cfg(any(test, feature = "testing"))]
pub mod testing;

pub use cx::Cx;
pub use observe::{Observable, ObserveError};
pub use view::{NodeId, ViewBuilder};

#[doc(hidden)]
pub use dispatch::__private;

/// The contract types an author names, so a plugin depends on this crate alone.
pub mod api {
    pub use nocterm_plugin_api::{
        API_VERSION, ContributionContext, EffectOutcome, EffectRequest, Effects, Event, HostError,
        LimitError, LocalId, MAX_BADGE, MAX_BUTTON, MAX_DEPTH, MAX_EFFECT_KEY, MAX_EFFECTS,
        MAX_LIST_ITEMS, MAX_NODE_KEY, MAX_NODES, MAX_TEXT, MIN_REFRESH_MS, Node, NodeKind,
        Observed, PluginIcon, Refresh, Snapshot, Tone, View,
        capability::{
            clock::ClockNow,
            monitor::Usage,
            notifications::{NoticeTone, OwnNotice, OwnNotices},
            transfers::Counts,
            workspace::{WorkspaceKind, WorkspaceState},
        },
    };
    pub use nocterm_plugin_model::SettingValue;
}

use api::{Effects, Event, View};

/// A code plugin. The host makes one value per instance with `Default` and calls it for every
/// contribution of the package; [`Cx::local`] says which. A trap or a fault drops the value, and
/// the next call starts again from `Default`.
pub trait Plugin: Default {
    /// Draws the contribution `cx.local()`.
    fn render(&mut self, cx: &mut Cx<'_>) -> View;

    /// Answers an event of the contribution `cx.local()` with effects. More than eight are
    /// refused as a whole.
    fn on_event(&mut self, cx: &mut Cx<'_>, event: &Event) -> Effects {
        let _ = (cx, event);
        Effects::default()
    }
}

#[cfg(test)]
mod tests;
