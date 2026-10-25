//! The contract between the host and plugins installed from a package.
//!
//! A plugin is a function from a snapshot of what it was allowed to see to a view, and from an
//! event to at most eight effects. This crate holds that data and the rules around it, and
//! nothing that runs a plugin or reads a file.
//!
//! Owns: the capability table (`CAPABILITIES`) with its sealed markers, `Snapshot`, `View` and
//! its arena rules, `Effects`, `Event`, `HostError`, `sanitize_display`, `PluginIcon` and its map
//! to the host icon, the author words `Zone` and `Kind`, `API_VERSION`, manifest DTOs and their
//! validation (ids, names, the clamp, the `[settings]` mapping onto `SettingsSchema`), grants
//! bound to `(PluginId, sha256)`, the declarative template language and its interpreter
//! (`DeclarativeRuntime`), the code package format (`package_format`: kind detection and the
//! bounded scan for the manifest section of a WebAssembly module), `Runtimes` (which picks the
//! declarative or the injected code runtime by the manifest), and the `PluginPackages` and
//! `PluginRuntime` ports with their errors.
//! Does not own: file access, TOML parsing, hashing, the broker, rate limits, providers, views
//! or any GPUI type.
//! Ports used/provided: provides `PluginPackages` (implemented by `nocterm-plugin-packages`)
//! and `PluginRuntime` (implemented here for declarative packages and by `nocterm-plugin-wasm`
//! for code); uses none.
//! Settings section: none; user plugin settings live in the host's records, not in
//! `settings.toml`. Globals: none.
//! Invariants: the table has unique kebab-case ids and an effect target exactly for effects;
//! every guest string is sanitized and bounded before it is shown; a view is an arena with
//! `parent < index`, unique keys and depth at most 8; a package cannot express a built-in
//! privilege; a template value is inserted as literal text and never parsed again.
//! `no_std` with `alloc`; the isolation check keeps it free of I/O and `unsafe`.
//! Feature `test-support`: constructors for tests of the host (`HostContext::with_stand_in_runtime`);
//! only dev-dependencies and the host's own `test-support` enable it, and `cargo xtask architecture`
//! refuses it in the normal build.
#![no_std]

extern crate alloc;
#[cfg(test)]
extern crate std;

pub mod capability;
mod contribution;
mod declarative;
mod effects;
mod error;
mod grant;
mod icon;
mod identity;
mod manifest;
mod package_format;
mod ports;
mod runtimes;
mod sanitize;
mod snapshot;
mod surface;
pub mod template;
mod version;
mod view;

pub use capability::{
    CAPABILITIES, Capability, CapabilityEntry, Deprecation, EffectTarget, Family, TableError,
    validate_table,
};
pub use contribution::{ContributionContext, Kind, Zone};
pub use declarative::DeclarativeRuntime;
pub use effects::{
    Effect, EffectOutcome, EffectRequest, Effects, EffectsError, Event, MAX_EFFECT_KEY, MAX_EFFECTS,
};
pub use error::{HostError, LimitError};
pub use grant::{Additions, Grant, GrantError};
pub use icon::PluginIcon;
pub use identity::{ContentHash, HashError, LocalId};
pub use manifest::{
    ChoiceOptionDto, ContributionDto, ContributionError, DeclareError, DefaultDto, FieldDto,
    FieldKindName, GroupDto, HostContext, MAX_CONTRIBUTIONS, MAX_DECLARED, MAX_DESCRIPTION,
    MAX_NAME, MAX_REASON, Manifest, ManifestDto, ManifestError, ManifestHeader, OptionalCapability,
    Runtime, SettingsDto, SettingsError,
};
pub use package_format::{MANIFEST_SECTION, PackageKind, SectionError, kind, manifest_section};
pub use ports::{
    Cause, MAX_MODULE_BYTES, MAX_PACKAGE_BYTES, ModuleError, Package, PackageError, PluginInstance,
    PluginPackages, PluginRuntime, ProtocolError, RuntimeError,
};
pub use runtimes::Runtimes;
pub use sanitize::sanitize_display;
pub use snapshot::{Observed, Snapshot};
pub use surface::{
    EFFECT_REQUESTS, EVENTS, FILTERS, HOST_ERRORS, LIMITS, MANIFEST_TABLES, REFRESHES, RUNTIMES,
    SETTINGS_KINDS,
};
pub use version::{API_VERSION, ApiVersion, VersionError};
pub use view::{
    MAX_BADGE, MAX_BUTTON, MAX_DEPTH, MAX_LIST_ITEMS, MAX_NODE_KEY, MAX_NODES, MAX_TEXT,
    MIN_REFRESH_MS, Node, NodeKind, Refresh, Tone, View, ViewError,
};

#[cfg(test)]
mod tests;
