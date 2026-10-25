//! The names and limits that make up the API surface, for the generated `api-surface.json`.
//!
//! Everything here is a constant of the contract, listed once. Tests keep the key lists in step
//! with the DTOs they describe (names, which keys are required, and what each holds), so the
//! generated file cannot drift from what a package may write. A removed or renamed name, a
//! changed key type, a key that became required, or a tightened limit, is a breaking change.

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

/// What a manifest key holds, as the schema and the compatibility check see it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeyType {
    Text,
    /// A whole number the host reads within `min..=max`. The range is part of the contract: a
    /// narrower one refuses numbers a published package may hold.
    Integer {
        min: i64,
        max: i64,
    },
    Bool,
    TextList,
    /// A table of text values by name.
    TextMap,
    Table,
    /// An array of tables.
    TableList,
    /// One text of a named vocabulary: the key of a list in the surface (`zones`, `icons`, ...).
    Choice(&'static str),
    /// More than one form, for example a refresh request or a setting's default.
    Any,
}

impl KeyType {
    /// The name the surface file gives the type.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Text => "text",
            Self::Integer { .. } => "integer",
            Self::Bool => "bool",
            Self::TextList => "text-list",
            Self::TextMap => "text-map",
            Self::Table => "table",
            Self::TableList => "table-list",
            Self::Choice(_) => "choice",
            Self::Any => "any",
        }
    }
}

/// One key of a manifest table.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Key {
    /// As an author writes it (snake_case).
    pub name: &'static str,
    pub ty: KeyType,
    /// A package without it is refused.
    pub required: bool,
}

const fn optional(name: &'static str, ty: KeyType) -> Key {
    Key {
        name,
        ty,
        required: false,
    }
}

const fn required(name: &'static str, ty: KeyType) -> Key {
    Key {
        name,
        ty,
        required: true,
    }
}

use KeyType::{Any, Bool, Choice, Table, TableList, Text, TextList, TextMap};

const fn integer(min: i64, max: i64) -> KeyType {
    KeyType::Integer { min, max }
}

/// A number the host keeps in a `u8`.
const U8: KeyType = integer(0, u8::MAX as i64);
/// A number the host keeps in a `u16`.
const U16: KeyType = integer(0, u16::MAX as i64);
/// A number the host keeps in an `i32`; it may be negative.
const I32: KeyType = integer(i32::MIN as i64, i32::MAX as i64);
/// A step of a number setting: the host refuses zero and below.
const STEP: KeyType = integer(1, i32::MAX as i64);

/// The keys of each table of a package, by table path.
///
/// A lower bound in [`LIMITS`] starts with `min-`; the compatibility check relies on the prefix
/// to tell which way a limit may move.
pub const MANIFEST_KEYS: &[(&str, &[Key])] = &[
    (
        "manifest",
        &[
            required("api", Text),
            optional("runtime", Choice("runtimes")),
            required("publisher", Text),
            required("id", Text),
            required("name", Text),
            optional("description", Text),
            optional("capabilities", TextList),
            optional("optional_capabilities", TextList),
            optional("reasons", TextMap),
            optional("contribution", TableList),
            optional("settings", Table),
        ],
    ),
    (
        "contribution",
        &[
            required("local", Text),
            required("kind", Choice("kinds")),
            required("title", Text),
            optional("icon", Choice("icons")),
            optional("zone", Choice("zones")),
            optional("order", U16),
            optional("view", Table),
        ],
    ),
    (
        "contribution.view",
        &[optional("refresh", Any), required("nodes", TableList)],
    ),
    (
        "contribution.view.nodes",
        &[
            required("kind", Choice("node_kinds")),
            optional("parent", U16),
            optional("key", Text),
            optional("tone", Choice("tones")),
            optional("tone_when", Text),
            optional("text", Text),
            optional("icon", Choice("icons")),
            optional("value", Text),
            optional("each", Text),
        ],
    ),
    (
        "settings",
        &[optional("version", U16), optional("group", TableList)],
    ),
    (
        "settings.group",
        &[
            required("title", Text),
            optional("contribution", Text),
            optional("field", TableList),
        ],
    ),
    (
        "settings.group.field",
        &[
            required("key", Text),
            required("label", Text),
            optional("help", Text),
            required("kind", Choice("settings_kinds")),
            optional("renamed_from", Text),
            optional("default", Any),
            optional("options", TableList),
            optional("ordered", Bool),
            optional("max_chosen", U8),
            optional("min", I32),
            optional("max", I32),
            optional("step", STEP),
            optional("unit", Text),
            optional("max_len", U16),
            optional("placeholder", Text),
        ],
    ),
    (
        "settings.group.field.options",
        &[
            required("key", Text),
            required("label", Text),
            optional("help", Text),
        ],
    ),
];

/// The request fields of each effect capability, with the type each holds and whether a guest
/// may leave it out. A field added later has to be optional, and the compatibility check refuses
/// one that is not.
pub const EFFECT_REQUESTS: &[(&str, &[Key])] = &[
    (
        "notifications.post",
        &[
            required("tone", Choice("notice_tones")),
            required("text", Text),
            optional("notice_key", Text),
        ],
    ),
    (
        "panels.set-open",
        &[required("local", Text), required("open", Bool)],
    ),
    (
        "panels.set-badge",
        &[required("local", Text), optional("badge", U16)],
    ),
    (
        "storage.set",
        &[required("key", Text), required("value", Text)],
    ),
    ("storage.remove", &[required("key", Text)]),
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
