//! The names and limits that make up the API surface, for the generated `api-surface.json`.
//!
//! Everything here is a constant of the contract, listed once. Tests keep the key lists in step
//! with the DTOs they describe, so the generated file cannot drift from what a package may
//! write. A removed or renamed name, or a tightened limit, is a breaking change.

use nocterm_plugin_model::{
    MAX_FIELDS, MAX_GROUPS, MAX_HELP, MAX_KEY, MAX_LABEL, MAX_OPTIONS, MAX_TEXT_LEN,
};

use crate::{
    MAX_BADGE, MAX_BUTTON, MAX_CONTRIBUTIONS, MAX_DECLARED, MAX_DEPTH, MAX_DESCRIPTION,
    MAX_EFFECT_KEY, MAX_EFFECTS, MAX_LIST_ITEMS, MAX_NAME, MAX_NODE_KEY, MAX_NODES,
    MAX_PACKAGE_BYTES, MAX_REASON, MAX_TEXT, MIN_REFRESH_MS,
    capability::{
        notifications::{MAX_LIVE_NOTICES, MAX_NOTICE_TEXT},
        storage::{MAX_KEY as MAX_STORAGE_KEY, MAX_TOTAL, MAX_VALUE},
    },
    template::{MAX_PLACEHOLDERS, MAX_ROUND, MAX_TEMPLATE_LEN},
};

/// The keys of each table of a package, by table path. Kebab-case, as an author writes them.
pub const MANIFEST_TABLES: &[(&str, &[&str])] = &[
    (
        "manifest",
        &[
            "api",
            "runtime",
            "publisher",
            "id",
            "name",
            "description",
            "capabilities",
            "optional_capabilities",
            "reasons",
            "contribution",
            "settings",
        ],
    ),
    (
        "contribution",
        &["local", "kind", "title", "icon", "zone", "order", "view"],
    ),
    ("contribution.view", &["refresh", "nodes"]),
    (
        "contribution.view.nodes",
        &[
            "kind",
            "parent",
            "key",
            "tone",
            "tone_when",
            "text",
            "icon",
            "value",
            "each",
        ],
    ),
    ("settings", &["version", "group"]),
    ("settings.group", &["title", "contribution", "field"]),
    (
        "settings.group.field",
        &[
            "key",
            "label",
            "help",
            "kind",
            "renamed_from",
            "default",
            "options",
            "ordered",
            "max_chosen",
            "min",
            "max",
            "step",
            "unit",
            "max_len",
            "placeholder",
        ],
    ),
    ("settings.group.field.options", &["key", "label", "help"]),
];

/// The request keys of each effect capability.
pub const EFFECT_REQUESTS: &[(&str, &[&str])] = &[
    ("notifications.post", &["tone", "text", "notice_key"]),
    ("panels.set-open", &["local", "open"]),
    ("panels.set-badge", &["local", "badge"]),
    ("storage.set", &["key", "value"]),
    ("storage.remove", &["key"]),
];

/// The kinds of setting a package may declare.
pub const SETTINGS_KINDS: [&str; 5] = ["toggle", "choice", "multi-choice", "number", "text"];

/// The runtimes a package may name.
pub const RUNTIMES: [&str; 2] = ["declarative", "wasm"];

/// The template filters.
pub const FILTERS: [&str; 4] = ["round(n)", "percent", "hh-mm", "threshold(low, high)"];

/// The events a plugin receives.
pub const EVENTS: [&str; 4] = [
    "activated",
    "toggled",
    "visibility-changed",
    "settings-changed",
];

/// The errors the host reports to a plugin.
pub const HOST_ERRORS: [&str; 4] = ["not-granted", "denied", "rate-limited", "unavailable"];

/// The refresh requests a view may make.
pub const REFRESHES: [&str; 3] = ["next-second", "next-minute", "after-ms"];

/// Every limit of the contract, by name.
pub const LIMITS: &[(&str, u64)] = &[
    ("badge-chars", MAX_BADGE as u64),
    ("button-chars", MAX_BUTTON as u64),
    ("capabilities-per-package", MAX_DECLARED as u64),
    ("contributions-per-package", MAX_CONTRIBUTIONS as u64),
    ("description-chars", MAX_DESCRIPTION as u64),
    ("effect-key-chars", MAX_EFFECT_KEY as u64),
    ("effects-per-call", MAX_EFFECTS as u64),
    ("list-items", MAX_LIST_ITEMS as u64),
    ("live-notices", MAX_LIVE_NOTICES as u64),
    ("min-refresh-ms", MIN_REFRESH_MS as u64),
    ("name-chars", MAX_NAME as u64),
    ("node-key-chars", MAX_NODE_KEY as u64),
    ("nodes-per-view", MAX_NODES as u64),
    ("notice-chars", MAX_NOTICE_TEXT as u64),
    ("package-bytes", MAX_PACKAGE_BYTES),
    ("placeholders-per-view", MAX_PLACEHOLDERS as u64),
    ("reason-chars", MAX_REASON as u64),
    ("round-decimals", MAX_ROUND as u64),
    ("setting-fields", MAX_FIELDS as u64),
    ("setting-groups", MAX_GROUPS as u64),
    ("setting-help-chars", MAX_HELP as u64),
    ("setting-key-chars", MAX_KEY as u64),
    ("setting-label-chars", MAX_LABEL as u64),
    ("setting-options", MAX_OPTIONS as u64),
    ("setting-text-chars", MAX_TEXT_LEN as u64),
    ("storage-key-chars", MAX_STORAGE_KEY as u64),
    ("storage-total-bytes", MAX_TOTAL as u64),
    ("storage-value-bytes", MAX_VALUE as u64),
    ("template-chars", MAX_TEMPLATE_LEN as u64),
    ("text-chars", MAX_TEXT as u64),
    ("view-depth", MAX_DEPTH as u64),
];
