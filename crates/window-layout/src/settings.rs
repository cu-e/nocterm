//! The `[layouts]` section of `settings.toml`: the user's presets and which one is selected.

use std::collections::{BTreeSet, HashSet};

use nocterm_plugin_model::{GroupId, PluginCatalog, PluginId};
use nocterm_settings::{SettingsDocument, SettingsSection};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::{
    BarItem, BuiltIn, LayoutPreset, PresetId, ResolvedLayout, name::is_reserved, name::same_name,
    resolve,
};

/// Most presets a document keeps.
pub const MAX_PRESETS: usize = 32;
/// Most entries in one zone of a preset.
pub const MAX_ZONE_ITEMS: usize = 64;
/// Most panels in one group of a preset.
pub const MAX_GROUP_MEMBERS: usize = 32;
/// Longest id of a stored preset.
const MAX_ID_LEN: usize = 32;

/// The presets the user saved and the one that is selected.
///
/// The built-in layouts are never stored; they are always available by id.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct LayoutSettings {
    /// The selected layout, a preset id or a built-in id (`default`, `swapped`). Unset or
    /// unknown selects `default`, or `swapped` if the earlier swap-sides choice is still in force.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub active: Option<PresetId>,
    /// The saved presets, in the order the user keeps them.
    pub presets: Vec<LayoutPreset>,
}

impl SettingsSection for LayoutSettings {
    const KEY: &'static str = "layouts";

    fn sanitize(&mut self) {
        self.presets.truncate(MAX_PRESETS);
        for preset in &mut self.presets {
            limit(&mut preset.bar.left);
            limit(&mut preset.bar.center);
            limit(&mut preset.bar.right);
        }
        self.assign_ids();
        self.assign_names();
    }
}

fn limit(zone: &mut Vec<BarItem>) {
    zone.truncate(MAX_ZONE_ITEMS);
    for item in zone {
        if let BarItem::Group { members, .. } = item {
            members.truncate(MAX_GROUP_MEMBERS);
        }
    }
}

impl LayoutSettings {
    /// Why the document's `[layouts]` table could not be read, if it could not.
    ///
    /// Such a table runs on the defaults and stays as written until the section is changed, so
    /// changing it, even to select a layout, would replace what the user wrote.
    pub fn read_error(document: &SettingsDocument) -> Option<String> {
        document
            .errors()
            .into_iter()
            .find(|error| error.key == Self::KEY)
            .map(|error| error.error)
    }

    /// The stored preset with this id.
    pub fn preset(&self, id: &PresetId) -> Option<&LayoutPreset> {
        self.presets.iter().find(|preset| preset.id == *id)
    }

    /// Whether the id names a stored preset or a built-in layout.
    pub fn exists(&self, id: &PresetId) -> bool {
        BuiltIn::of(id).is_some() || self.preset(id).is_some()
    }

    /// The id a new preset would get: `p<n>` with n one more than the largest stored, or the
    /// smallest unused number once the largest is as high as a number goes.
    pub fn next_id(&self) -> PresetId {
        let used = numbers_in_use(self.presets.iter().map(|preset| &preset.id));
        fresh_id(free_number(&used))
    }

    /// The arrangement of a layout against the catalog, without the disabled plugins. An
    /// unknown id resolves as `default`.
    pub fn resolve(
        &self,
        id: &PresetId,
        catalog: &PluginCatalog,
        disabled: &BTreeSet<PluginId>,
    ) -> ResolvedLayout {
        if let Some(built_in) = BuiltIn::of(id) {
            return built_in.resolve(catalog, disabled);
        }
        match self.preset(id) {
            Some(preset) => resolve(preset, catalog, disabled),
            None => BuiltIn::Default.resolve(catalog, disabled),
        }
    }

    /// The saved presets' `primary` flags that `resolve` ignores, as (preset, group) pairs: the
    /// flag is set on a group other than the one the catalog makes primary.
    pub fn ignored_primary_flags(&self, catalog: &PluginCatalog) -> Vec<(PresetId, GroupId)> {
        let primary = catalog.groups().iter().find(|spec| spec.primary);
        let mut ignored = Vec::new();
        for preset in &self.presets {
            let zones = [&preset.bar.left, &preset.bar.center, &preset.bar.right];
            for item in zones.into_iter().flatten() {
                if let BarItem::Group {
                    id, primary: true, ..
                } = item
                    && primary.is_none_or(|spec| spec.id != *id)
                {
                    ignored.push((preset.id.clone(), id.clone()));
                }
            }
        }
        ignored
    }

    /// Gives a missing, invalid, reserved or repeated id a fresh `p<n>`.
    ///
    /// A selection that named a preset by an id that is replaced follows it; a selection that
    /// names a built-in or a preset that kept its id stays as it is.
    fn assign_ids(&mut self) {
        let mut seen = HashSet::new();
        let mut unusable = Vec::new();
        for (index, preset) in self.presets.iter().enumerate() {
            if !usable_id(&preset.id) || !seen.insert(preset.id.clone()) {
                unusable.push(index);
            }
        }
        let mut used = numbers_in_use(
            self.presets
                .iter()
                .enumerate()
                .filter(|(index, _)| !unusable.contains(index))
                .map(|(_, preset)| &preset.id),
        );
        let mut selection_follows = true;
        for index in unusable {
            let number = free_number(&used);
            used.insert(number);
            let fresh = fresh_id(number);
            let old = std::mem::replace(&mut self.presets[index].id, fresh.clone());
            let was_selected = self.active.as_ref() == Some(&old);
            if was_selected && selection_follows && !usable_id(&old) && BuiltIn::of(&old).is_none()
            {
                self.active = Some(fresh);
                selection_follows = false;
            }
        }
    }

    /// Trims and shortens names, and makes them differ from each other and from the built-ins.
    fn assign_names(&mut self) {
        let mut taken: HashSet<String> = HashSet::new();
        for preset in &mut self.presets {
            let name = fit_name(&preset.name, |name| {
                is_reserved(name) || taken.iter().any(|taken| same_name(taken, name))
            });
            taken.insert(name.clone());
            preset.name = name;
        }
    }
}

/// A stored id is `[a-z0-9-]`, at most 32 characters, and not a built-in's.
fn usable_id(id: &PresetId) -> bool {
    let text = id.as_str();
    !text.is_empty()
        && text.len() <= MAX_ID_LEN
        && text
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
        && BuiltIn::of(id).is_none()
}

/// The `n` of an id spelled `p<n>`.
fn number_of(id: &PresetId) -> Option<u32> {
    let digits = id.as_str().strip_prefix('p')?;
    if digits.is_empty() || !digits.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    digits.parse().ok()
}

/// The numbers of the ids spelled `p<n>`.
fn numbers_in_use<'a>(ids: impl Iterator<Item = &'a PresetId>) -> HashSet<u32> {
    ids.filter_map(number_of).collect()
}

/// One more than the largest number in use; once that does not fit, the smallest unused one.
fn free_number(used: &HashSet<u32>) -> u32 {
    used.iter()
        .max()
        .and_then(|largest| largest.checked_add(1))
        .or_else(|| (1..=u32::MAX).find(|number| !used.contains(number)))
        .unwrap_or(1)
}

fn fresh_id(number: u32) -> PresetId {
    PresetId::from_owned(format!("p{number}"))
}

/// The name shortened to 48 characters, trimmed, and numbered until `taken` refuses no more.
fn fit_name(name: &str, taken: impl Fn(&str) -> bool) -> String {
    let base: String = name
        .trim()
        .chars()
        .take(crate::name::MAX_NAME_CHARS)
        .collect();
    let base = if base.trim().is_empty() {
        "Layout".to_owned()
    } else {
        base.trim().to_owned()
    };
    if !taken(&base) {
        return base;
    }
    (2_u32..)
        .map(|number| {
            let suffix = format!(" {number}");
            let room = crate::name::MAX_NAME_CHARS - suffix.chars().count();
            let head: String = base.chars().take(room).collect();
            format!("{}{suffix}", head.trim_end())
        })
        .find(|candidate| !taken(candidate))
        .unwrap_or(base)
}

/// The layout the settings select.
///
/// `active` if it names a stored preset or a built-in; otherwise `swapped` while the legacy
/// swap-sides flag is set; otherwise `default`. Never writes: an unknown `active` stays in the
/// document.
pub fn select(settings: &LayoutSettings, legacy_swapped: bool) -> PresetId {
    match &settings.active {
        Some(active) if settings.exists(active) => active.clone(),
        _ if legacy_swapped => BuiltIn::Swapped.id(),
        _ => BuiltIn::Default.id(),
    }
}
