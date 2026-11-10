//! `panels.set-open` and `panels.set-badge`.

use serde::{Deserialize, Serialize};

use super::{Capability, sealed};
use crate::LocalId;

/// Opens or closes one of the plugin's own panels. It never moves focus.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub struct SetOpen {
    pub local: LocalId,
    pub open: bool,
}

/// Marker of `panels.set-open`.
pub struct SetPanelOpen;

impl sealed::Sealed for SetPanelOpen {}

impl Capability for SetPanelOpen {
    const ID: &'static str = "panels.set-open";
    type Value = SetOpen;
}

/// Sets or clears the badge number on one of the plugin's own contributions.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub struct SetBadge {
    pub local: LocalId,
    pub badge: Option<u16>,
}

/// Marker of `panels.set-badge`.
pub struct SetPanelBadge;

impl sealed::Sealed for SetPanelBadge {}

impl Capability for SetPanelBadge {
    const ID: &'static str = "panels.set-badge";
    type Value = SetBadge;
}
