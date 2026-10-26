//! The table itself, and the rules every entry follows.

use alloc::collections::BTreeSet;
use core::time::Duration;

use thiserror::Error;

use crate::{API_VERSION, ApiVersion};

/// Whether a capability reads host state or asks the host to act.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Family {
    Observe,
    Effect,
}

/// What an effect may act on. Host objects are outside the model.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EffectTarget {
    /// The plugin's own contributions.
    OwnContribution,
    /// The plugin's own stored data.
    OwnData,
    /// How the host presents something the plugin said, such as a notice.
    HostPresentation,
}

/// A capability that still works but has a successor.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Deprecation {
    pub since: ApiVersion,
    pub successor: &'static str,
    pub note: &'static str,
}

/// One row of the table. Fields can be added later without breaking authors.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CapabilityEntry {
    /// `<domain>.<name>`, kebab-case.
    pub id: &'static str,
    pub family: Family,
    /// What it is, for authors.
    pub doc: &'static str,
    /// What the consent dialog says in the host's words.
    pub consent: &'static str,
    /// What the capability deliberately does not give.
    pub excluded: &'static str,
    /// Computed from a connection, host, project, file or other non-plugin user data, or from
    /// the user's locale or time zone.
    pub session_derived: bool,
    /// Set exactly for effects.
    pub target: Option<EffectTarget>,
    /// Needs a user gesture token.
    pub gesture: bool,
    /// At most this many uses in this time.
    pub rate: Option<(u16, Duration)>,
    /// Most live results per plugin.
    pub live_cap: Option<u8>,
    pub since: ApiVersion,
    pub deprecated: Option<Deprecation>,
}

const V1: ApiVersion = ApiVersion::new(0, 1);

/// Every capability of this API version.
pub const CAPABILITIES: &[CapabilityEntry] = &[
    CapabilityEntry {
        id: "clock.now",
        family: Family::Observe,
        doc: "The wall-clock time to the minute and the UTC offset of the user.",
        consent: "See the current time and your time zone.",
        excluded: "A timer finer than a minute.",
        session_derived: true,
        target: None,
        gesture: false,
        rate: None,
        live_cap: None,
        since: V1,
        deprecated: None,
    },
    CapabilityEntry {
        id: "workspace.kind",
        family: Family::Observe,
        doc: "Whether the window shows a local, remote, container or folder workspace, and whether it is connected.",
        consent: "See what kind of workspace a window shows and whether it is connected.",
        excluded: "Title, host, user, path and tab count.",
        session_derived: true,
        target: None,
        gesture: false,
        rate: None,
        live_cap: None,
        since: V1,
        deprecated: None,
    },
    CapabilityEntry {
        id: "monitor.usage",
        family: Family::Observe,
        doc: "Processor and memory use in percent, or unavailable when no built-in sampler runs.",
        consent: "See how busy this computer is.",
        excluded: "Processes and host names.",
        session_derived: true,
        target: None,
        gesture: false,
        rate: None,
        live_cap: None,
        since: V1,
        deprecated: None,
    },
    CapabilityEntry {
        id: "transfers.counts",
        family: Family::Observe,
        doc: "How many transfers run and how many failed.",
        consent: "See how many file transfers are running or failed.",
        excluded: "File names, paths and rates.",
        session_derived: true,
        target: None,
        gesture: false,
        rate: None,
        live_cap: None,
        since: V1,
        deprecated: None,
    },
    CapabilityEntry {
        id: "notifications.own",
        family: Family::Observe,
        doc: "The notices this plugin posted that are still shown.",
        consent: "See the notices this plugin posted.",
        excluded: "Notices of the host and of other plugins.",
        session_derived: false,
        target: None,
        gesture: false,
        rate: None,
        live_cap: None,
        since: V1,
        deprecated: None,
    },
    CapabilityEntry {
        id: "notifications.post",
        family: Family::Effect,
        doc: "Posts an informational or caution notice of up to 200 characters.",
        consent: "Show notices in the status bar.",
        excluded: "Actions, links and the error level.",
        session_derived: false,
        target: Some(EffectTarget::HostPresentation),
        gesture: false,
        rate: Some((1, Duration::from_secs(10))),
        live_cap: Some(3),
        since: V1,
        deprecated: None,
    },
    CapabilityEntry {
        id: "panels.set-open",
        family: Family::Effect,
        doc: "Opens or closes one of the plugin's own panels after a click.",
        consent: "Open or close its own panels when you click it.",
        excluded: "Other contributions and moving focus.",
        session_derived: false,
        target: Some(EffectTarget::OwnContribution),
        gesture: true,
        rate: None,
        live_cap: None,
        since: V1,
        deprecated: None,
    },
    CapabilityEntry {
        id: "panels.set-badge",
        family: Family::Effect,
        doc: "Sets or clears a number badge on one of the plugin's own contributions.",
        consent: "Show a number on its own panel or widget.",
        excluded: "Attention requests.",
        session_derived: false,
        target: Some(EffectTarget::OwnContribution),
        gesture: false,
        rate: None,
        live_cap: None,
        since: V1,
        deprecated: None,
    },
    CapabilityEntry {
        id: "storage.set",
        family: Family::Effect,
        doc: "Stores a value under a key in the plugin's own data, up to 64 KiB in total.",
        consent: "Keep a small amount of its own data.",
        excluded: "Data of other plugins.",
        session_derived: false,
        target: Some(EffectTarget::OwnData),
        gesture: false,
        rate: None,
        live_cap: None,
        since: V1,
        deprecated: None,
    },
    CapabilityEntry {
        id: "storage.remove",
        family: Family::Effect,
        doc: "Removes a key from the plugin's own data.",
        consent: "Remove its own stored data.",
        excluded: "Data of other plugins.",
        session_derived: false,
        target: Some(EffectTarget::OwnData),
        gesture: false,
        rate: None,
        live_cap: None,
        since: V1,
        deprecated: None,
    },
];

/// The entry with `id`.
pub fn capability(id: &str) -> Option<&'static CapabilityEntry> {
    CAPABILITIES.iter().find(|entry| entry.id == id)
}

/// A table the rules refuse.
#[derive(Clone, Debug, PartialEq, Eq, Error)]
pub enum TableError {
    #[error("capability `{0}` is not a kebab-case `domain.name`")]
    Id(&'static str),
    #[error("capability `{0}` is listed twice")]
    Duplicate(&'static str),
    #[error("capability `{0}` has a target exactly when it should not")]
    Target(&'static str),
    #[error("capability `{0}` has no consent or excluded text")]
    Text(&'static str),
    #[error("capability `{0}` is newer than this API version")]
    Future(&'static str),
    #[error("capability `{0}` is deprecated in favour of an entry the table does not hold")]
    Successor(&'static str),
}

/// Checks the rules of the table: kebab-case `<domain>.<name>` ids, unique ids, `target` set
/// exactly for effects, non-empty `consent` and `excluded`, no entry newer than this API, and
/// deprecated entries naming a successor in the table.
pub fn validate_table(table: &[CapabilityEntry]) -> Result<(), TableError> {
    let mut seen = BTreeSet::new();
    for entry in table {
        if !is_id(entry.id) {
            return Err(TableError::Id(entry.id));
        }
        if !seen.insert(entry.id) {
            return Err(TableError::Duplicate(entry.id));
        }
        if entry.target.is_some() != (entry.family == Family::Effect) {
            return Err(TableError::Target(entry.id));
        }
        if entry.consent.trim().is_empty() || entry.excluded.trim().is_empty() {
            return Err(TableError::Text(entry.id));
        }
        if entry.since > API_VERSION {
            return Err(TableError::Future(entry.id));
        }
    }
    for entry in table {
        if let Some(deprecation) = entry.deprecated
            && !seen.contains(deprecation.successor)
        {
            return Err(TableError::Successor(entry.id));
        }
    }
    Ok(())
}

fn is_id(id: &str) -> bool {
    let kebab = |part: &str| {
        !part.is_empty()
            && !part.starts_with('-')
            && !part.ends_with('-')
            && !part.contains("--")
            && part
                .bytes()
                .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
    };
    id.split_once('.')
        .is_some_and(|(domain, name)| kebab(domain) && kebab(name))
}
