//! Edits of the user's presets: pure operations on the `[layouts]` section.
//!
//! [`edit`] applies one [`EditOp`] to one preset and returns what the preset list looked like
//! before and after, so a caller can offer undo. It runs on the settings it is given, inside
//! the store's ordered update, and leaves them untouched when it fails.

mod settle;
mod structure;

use nocterm_plugin_model::{ContributionId, GroupId, PluginSet, Side, Zone};

use crate::{
    BuiltIn, LayoutPreset, LayoutSettings, MAX_GROUP_MEMBERS, MAX_PRESETS, MAX_ZONE_ITEMS,
    NameError, PresetId, name::unique_name, select, validate_name,
};

/// The preset an operation acts on.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EditTarget {
    /// The selected preset, found when the operation runs.
    Selected,
    /// A preset by id, which may be a built-in. Only `Duplicate` is accepted on a built-in that
    /// is not selected.
    Preset(PresetId),
}

/// Something the bar holds that an operation moves.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Subject {
    /// A widget or a panel.
    Contribution(ContributionId),
    /// A panel group in a zone.
    Group(GroupId),
}

impl std::fmt::Display for Subject {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Contribution(id) => write!(f, "{id}"),
            Self::Group(id) => write!(f, "{id}"),
        }
    }
}

/// A list of the bar an entry can be put in.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Slot {
    Zone(Zone),
    /// The members of a group that is in the bar.
    Group(GroupId),
}

/// What an edit does. Positions count the entries the editor shows, separators included.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EditOp {
    /// Moves a placed entry `by` positions within its zone or group, stopping at the ends.
    Move {
        subject: Subject,
        by: i32,
    },
    /// Moves a widget or a group to the end of a zone.
    ToZone {
        subject: Subject,
        zone: Zone,
    },
    /// Moves a panel to the end of a group; the group joins its panel's zone if the bar has
    /// none.
    ToGroup {
        panel: ContributionId,
        group: GroupId,
    },
    /// Puts a widget, a panel or a group in front of the entry at `index` of a zone or a group,
    /// or at its end when `index` is past the last. `index` counts the entries as the editor
    /// shows them, the moved one included, so the same list is not shifted by its own entry.
    ///
    /// A widget or a group goes to a zone, a panel to a group that is in the bar. A widget or a
    /// panel that waits in the tray comes out of it.
    MoveTo {
        subject: Subject,
        place: Slot,
        index: usize,
    },
    InsertSeparator {
        place: Slot,
        index: usize,
    },
    RemoveSeparator {
        place: Slot,
        index: usize,
    },
    /// Takes a widget or a panel out of the bar. A pinned widget stays.
    RemoveToTray {
        id: ContributionId,
    },
    /// Puts a contribution back where its hints say.
    Restore {
        id: ContributionId,
    },
    /// Puts a contribution of the tray into a zone: a widget at the end of it, a panel in its
    /// hinted group, which takes the zone when the bar has no such group yet. A contribution
    /// that is in the bar stays where it is.
    PlaceIn {
        id: ContributionId,
        zone: Zone,
    },
    /// Drops an id of the tray that the catalog does not know.
    Forget {
        id: ContributionId,
    },
    /// Makes a panel open (or not) with the layout; the other members of its group do not.
    SetOpen {
        panel: ContributionId,
        open: bool,
    },
    /// The side a group opens on; `None` follows its zone.
    SetSide {
        group: GroupId,
        side: Option<Side>,
    },
    Rename {
        name: String,
    },
    /// Copies the preset to a new one. `active` does not change.
    Duplicate,
    Delete,
    /// Empties the preset, so the hints arrange it again.
    Reset,
    /// Replaces the stored presets (undo and redo); `active` does not change.
    RestorePresets(Vec<LayoutPreset>),
}

/// The presets before and after an edit, and the one the editor should show: the preset that
/// was edited, the copy a `Duplicate` made, or the selected one after `Delete` and undo.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EditOutcome {
    pub before: Vec<LayoutPreset>,
    pub after: Vec<LayoutPreset>,
    pub selected: PresetId,
}

/// Which limit an edit would exceed.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum Limit {
    #[error("at most {MAX_PRESETS} layouts")]
    Presets,
    #[error("at most {MAX_ZONE_ITEMS} entries in a zone")]
    ZoneItems,
    #[error("at most {MAX_GROUP_MEMBERS} panels in a group")]
    GroupMembers,
}

/// Why an edit was refused. The settings are unchanged.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum EditError {
    #[error("the layout \"{0}\" no longer exists")]
    PresetGone(PresetId),
    #[error("the built-in layout \"{0}\" cannot be changed; duplicate it first")]
    BuiltinReadOnly(PresetId),
    #[error("\"{0}\" is always shown and cannot be removed")]
    Pinned(ContributionId),
    #[error(transparent)]
    Name(#[from] NameError),
    #[error("\"{0}\" is not known to this application")]
    Unknown(String),
    #[error("\"{0}\" is not in the status bar")]
    NotPlaced(Subject),
    #[error("\"{0}\" cannot be put there")]
    WrongKind(Subject),
    #[error("\"{0}\" is installed; remove it to the tray instead")]
    Installed(ContributionId),
    #[error("the entry is not a separator")]
    NotASeparator,
    #[error("too many entries: {0}")]
    Full(Limit),
}

/// Applies `op` to the preset `target` names.
///
/// - `Selected` and the id of the selected preset mean the same preset, found against the
///   document as it is when this runs.
/// - A built-in that is selected is copied first: the entry operations make a new preset called
///   "Custom" from it, select it and act on that, so the first edit changes what the window
///   shows. Two operations on `Selected` therefore land on the one copy.
/// - A built-in that is not selected accepts `Duplicate` only.
/// - `Rename`, `Delete` and `Reset` never copy: on a built-in they are refused.
/// - The entry operations first rewrite the preset the way it resolves with every plugin
///   enabled: unknown ids of the bar move to the tray, repeated or misplaced entries go, the
///   hinted contributions are written out. Disabled plugins keep their entries in place.
///
/// The catalog supplies kinds, hints and pinning.
pub fn edit(
    settings: &mut LayoutSettings,
    legacy_swapped: bool,
    catalog: &PluginSet,
    target: EditTarget,
    op: EditOp,
) -> Result<EditOutcome, EditError> {
    let before = settings.presets.clone();
    let mut next = settings.clone();
    let selected = run(&mut next, legacy_swapped, catalog, target, op)?;
    let after = next.presets.clone();
    *settings = next;
    Ok(EditOutcome {
        before,
        after,
        selected,
    })
}

fn run(
    next: &mut LayoutSettings,
    legacy_swapped: bool,
    catalog: &PluginSet,
    target: EditTarget,
    op: EditOp,
) -> Result<PresetId, EditError> {
    if let EditOp::RestorePresets(presets) = op {
        next.presets = presets;
        return Ok(select(next, legacy_swapped));
    }
    let current = select(next, legacy_swapped);
    let id = match target {
        EditTarget::Selected => current.clone(),
        EditTarget::Preset(id) => id,
    };
    let built_in = BuiltIn::of(&id);
    if built_in.is_none() && next.preset(&id).is_none() {
        return Err(EditError::PresetGone(id));
    }
    match op {
        EditOp::Duplicate => duplicate(next, &id, catalog),
        EditOp::Rename { name } => rename(next, &id, &name).map(|()| id),
        EditOp::Delete => delete(next, &id).map(|()| select(next, legacy_swapped)),
        EditOp::Reset => reset(next, &id).map(|()| id),
        op => {
            if built_in.is_some() && id != current {
                return Err(EditError::BuiltinReadOnly(id));
            }
            let id = match built_in {
                Some(built_in) => copy_on_write(next, built_in, catalog)?,
                None => id,
            };
            structure::apply(next, &id, catalog, op).map(|()| id)
        }
    }
}

/// The preset a built-in becomes on its first edit: named "Custom", written out, selected.
fn copy_on_write(
    next: &mut LayoutSettings,
    built_in: BuiltIn,
    catalog: &PluginSet,
) -> Result<PresetId, EditError> {
    let id = add_copy(next, "Custom", settle::written_out(built_in, catalog))?;
    next.active = Some(id.clone());
    Ok(id)
}

/// Adds a preset with these contents under a fresh id and a name derived from `name`.
fn add_copy(
    next: &mut LayoutSettings,
    name: &str,
    contents: LayoutPreset,
) -> Result<PresetId, EditError> {
    if next.presets.len() >= MAX_PRESETS {
        return Err(EditError::Full(Limit::Presets));
    }
    let id = next.next_id();
    let name = unique_name(&next.presets, name);
    next.presets.push(LayoutPreset {
        id: id.clone(),
        name,
        ..contents
    });
    Ok(id)
}

fn duplicate(
    next: &mut LayoutSettings,
    id: &PresetId,
    catalog: &PluginSet,
) -> Result<PresetId, EditError> {
    let (name, contents) = match BuiltIn::of(id) {
        Some(built_in) => (
            built_in.name().to_owned(),
            settle::written_out(built_in, catalog),
        ),
        None => match next.preset(id) {
            Some(preset) => (preset.name.clone(), preset.clone()),
            None => return Err(EditError::PresetGone(id.clone())),
        },
    };
    add_copy(next, &format!("{name} copy"), contents)
}

fn rename(next: &mut LayoutSettings, id: &PresetId, name: &str) -> Result<(), EditError> {
    if BuiltIn::of(id).is_some() {
        return Err(EditError::BuiltinReadOnly(id.clone()));
    }
    let name = validate_name(next, Some(id), name)?;
    if let Some(preset) = next.presets.iter_mut().find(|preset| preset.id == *id) {
        preset.name = name;
    }
    Ok(())
}

fn delete(next: &mut LayoutSettings, id: &PresetId) -> Result<(), EditError> {
    if BuiltIn::of(id).is_some() {
        return Err(EditError::BuiltinReadOnly(id.clone()));
    }
    next.presets.retain(|preset| preset.id != *id);
    if next.active.as_ref() == Some(id) {
        next.active = None;
    }
    Ok(())
}

fn reset(next: &mut LayoutSettings, id: &PresetId) -> Result<(), EditError> {
    if BuiltIn::of(id).is_some() {
        return Err(EditError::BuiltinReadOnly(id.clone()));
    }
    if let Some(preset) = next.presets.iter_mut().find(|preset| preset.id == *id) {
        let empty = BuiltIn::preset();
        preset.bar = empty.bar;
        preset.tray = empty.tray;
        preset.open = empty.open;
    }
    Ok(())
}
