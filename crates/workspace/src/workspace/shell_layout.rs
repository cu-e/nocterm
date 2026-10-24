//! The arrangement of the window: the selected layout resolved against the plugin catalog.
//!
//! The layouts are the app-wide `[layouts]` settings, and the plugins the user disabled are the
//! app-wide `[plugins]` settings: a disabled plugin's contributions drop out of the arrangement. Every window keeps the document it last
//! saw, so a change tells it whether the user switched layouts, edited the selected one or did
//! something it does not show. The legacy `workspace.swapped` flag is read once, when the window
//! is composed: nothing sets it any more, but it still picks `swapped` for a user who never
//! chose a layout.

use std::{
    collections::{BTreeSet, HashSet},
    sync::Arc,
};

use gpui_kit::{App, Subscription, Task};
use nocterm_plugin_enablement::PluginEnablement;
use nocterm_plugin_model::{ContributionId, PluginCatalog, PluginId, Side};
use nocterm_store::{SettingsHandle, StoreError};
use nocterm_ui::LayoutHandle;
use nocterm_window_layout::{
    BuiltIn, EditOp, EditOutcome, EditTarget, LayoutSettings, LayoutTransition, PresetId,
    ResolvedGroup, ResolvedLayout, select, transition,
};

use super::layout_edit::{LayoutEditError, edit_layouts};
use super::layout_switch::{LayoutChoice, LayoutSwitchError, MalformedLayouts, Removed};
use super::panel_groups::GroupKey;

/// What a change of the layout settings asks of the window.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum LayoutChange {
    /// Nothing the window shows changed.
    Unchanged,
    /// The arrangement changed; the panels that are open and still placed stay open.
    Rearranged,
    /// The user chose a saved layout, which names the panels that show.
    Opened(Vec<ContributionId>),
}

/// Logs each `primary` flag of a saved preset that the layout ignores, unless `before` already
/// had it: the primary group is the catalog's, and a user who sets the flag should learn that.
fn report_ignored_primary(
    layouts: &LayoutSettings,
    before: Option<&LayoutSettings>,
    catalog: &PluginCatalog,
) {
    let known = before.map(|before| before.ignored_primary_flags(catalog));
    for flag in layouts.ignored_primary_flags(catalog) {
        if known.as_ref().is_some_and(|known| known.contains(&flag)) {
            continue;
        }
        let (preset, group) = flag;
        tracing::warn!(
            %preset,
            %group,
            "the primary flag of this group in [layouts] is ignored: the sidebar group is fixed"
        );
    }
}

pub(super) struct ShellLayout {
    settings: SettingsHandle,
    catalog: Arc<PluginCatalog>,
    legacy_swapped: bool,
    seen: LayoutSettings,
    /// The plugins the arrangement leaves out: the stored ones the catalog allows to be off.
    disabled: BTreeSet<PluginId>,
    selected: PresetId,
    resolved: ResolvedLayout,
    /// The panels this window has already opened because the layout names them.
    opened: HashSet<ContributionId>,
    subscription: Option<Subscription>,
}

impl ShellLayout {
    pub(super) fn new(
        flags: &LayoutHandle,
        settings: SettingsHandle,
        catalog: Arc<PluginCatalog>,
        cx: &mut App,
    ) -> Self {
        settings.register::<LayoutSettings>(cx);
        settings.register::<PluginEnablement>(cx);
        let legacy_swapped = super::layout_edit::legacy_layout_swapped(flags, cx);
        let seen = settings.setting::<LayoutSettings>(cx).clone();
        let disabled = settings
            .setting::<PluginEnablement>(cx)
            .effective_disabled(&catalog);
        let selected = select(&seen, legacy_swapped);
        let resolved = seen.resolve(&selected, &catalog, &disabled);
        report_ignored_primary(&seen, None, &catalog);
        Self {
            settings,
            catalog,
            legacy_swapped,
            seen,
            disabled,
            selected,
            resolved,
            opened: HashSet::new(),
            subscription: None,
        }
    }

    /// Calls `on_change` whenever the layout settings are published, for as long as the window
    /// lives.
    pub(super) fn follow(&mut self, cx: &mut App, on_change: impl FnMut(&mut App) + 'static) {
        self.subscription = Some(self.settings.observe(cx, on_change));
    }

    /// Reads the settings again and tells how the change affects the window.
    ///
    /// Enabling or disabling a plugin is an edit of the arrangement, never a switch: panels that
    /// stay keep their state and no `open` set applies.
    pub(super) fn refresh(&mut self, cx: &App) -> LayoutChange {
        let next = self.settings.setting::<LayoutSettings>(cx);
        let disabled = self
            .settings
            .setting::<PluginEnablement>(cx)
            .effective_disabled(&self.catalog);
        let layouts_changed = *next != self.seen;
        let enablement_changed = disabled != self.disabled;
        if !layouts_changed && !enablement_changed {
            return LayoutChange::Unchanged;
        }
        let change = if layouts_changed {
            transition(&self.seen, next, self.legacy_swapped)
        } else {
            LayoutTransition::None
        };
        if layouts_changed {
            report_ignored_primary(next, Some(&self.seen), &self.catalog);
            self.seen = next.clone();
        }
        self.disabled = disabled;
        self.selected = select(&self.seen, self.legacy_swapped);
        self.resolved = self
            .seen
            .resolve(&self.selected, &self.catalog, &self.disabled);
        match change {
            LayoutTransition::None if enablement_changed => LayoutChange::Rearranged,
            LayoutTransition::None => LayoutChange::Unchanged,
            LayoutTransition::Switch => match self.open_on_switch() {
                Some(open) => LayoutChange::Opened(open.to_vec()),
                None => LayoutChange::Rearranged,
            },
            LayoutTransition::Edit => LayoutChange::Rearranged,
        }
    }

    /// Whether the contribution may exist: core contributions always, a plugin's while the user
    /// has not disabled the plugin.
    pub(super) fn is_enabled(&self, id: &ContributionId) -> bool {
        self.catalog
            .plugin_of(id)
            .is_none_or(|plugin| !self.disabled.contains(&plugin.id))
    }

    /// The layouts to choose from: the built-ins, then the saved presets in their order.
    pub(super) fn choices(&self) -> Vec<LayoutChoice> {
        BuiltIn::ALL
            .into_iter()
            .map(|built_in| (built_in.id(), built_in.name().to_owned()))
            .chain(
                self.seen
                    .presets
                    .iter()
                    .map(|preset| (preset.id.clone(), preset.name.clone())),
            )
            .map(|(id, name)| {
                let selected = id == self.selected;
                LayoutChoice::new(id, name, selected)
            })
            .collect()
    }

    /// The layout `offset` places after the selected one in menu order, wrapping.
    pub(super) fn neighbour(&self, offset: isize) -> Option<PresetId> {
        let choices = self.choices();
        let current = choices.iter().position(LayoutChoice::is_selected)?;
        let target = (current as isize + offset).rem_euclid(choices.len() as isize) as usize;
        Some(choices[target].id().clone())
    }

    /// Writes the selection for every window. The write goes through the ordered settings
    /// queue, so the selection changes only once it is published; a failed write leaves it as
    /// it was.
    ///
    /// A `[layouts]` table that could not be read is refused: writing the selection would
    /// replace it with the defaults and lose what the user wrote.
    pub(super) fn select_layout(
        &self,
        id: PresetId,
        cx: &mut App,
    ) -> Task<Result<(), LayoutSwitchError>> {
        if let Some(reason) = LayoutSettings::read_error(self.settings.document(cx)) {
            return Task::ready(Err(LayoutSwitchError::Malformed(MalformedLayouts::new(
                reason,
            ))));
        }
        if !self.settings.setting::<LayoutSettings>(cx).exists(&id) {
            return Task::ready(Err(LayoutSwitchError::Unknown(id)));
        }
        let write = self.settings.edit_checked(cx, move |document| {
            document.register::<LayoutSettings>();
            if let Some(reason) = LayoutSettings::read_error(document) {
                return Err(StoreError::edit(MalformedLayouts::new(reason)));
            }
            if !document.get::<LayoutSettings>().exists(&id) {
                return Err(StoreError::edit(Removed(id)));
            }
            document.update::<LayoutSettings>(|layouts| layouts.active = Some(id));
            Ok(())
        });
        cx.spawn(async move |_| write.await.map(drop).map_err(LayoutSwitchError::from))
    }

    /// Applies `op` to the layout `target` names for every window.
    pub(super) fn edit(
        &self,
        target: EditTarget,
        op: EditOp,
        cx: &mut App,
    ) -> Task<Result<Option<EditOutcome>, LayoutEditError>> {
        edit_layouts(
            &self.settings,
            self.catalog.clone(),
            self.legacy_swapped,
            target,
            op,
            cx,
        )
    }

    /// Whether this window should open the registered panel now: the selected saved layout names
    /// it and this window has not opened it yet. A panel that registers again, as when its plugin
    /// is enabled once more, is not reopened.
    pub(super) fn claim_open(&mut self, id: &ContributionId) -> bool {
        let named = self.open_on_switch().is_some_and(|open| open.contains(id));
        named && self.opened.insert(id.clone())
    }

    /// The arrangement in force.
    pub(super) fn current(&self) -> &ResolvedLayout {
        &self.resolved
    }

    /// The panels a switch to the selected layout opens. A saved preset names them; a built-in
    /// leaves the panels as they are, because it has no opinion about them.
    pub(super) fn open_on_switch(&self) -> Option<&[ContributionId]> {
        self.seen
            .preset(&self.selected)
            .map(|_| self.resolved.open())
    }

    /// The side of the content a group opens on. A group the arrangement does not place opens
    /// where its panels hint.
    pub(super) fn side(&self, group: GroupKey) -> Side {
        self.resolved
            .groups()
            .find(|placed| placed.id().as_str() == group.id())
            .map(ResolvedGroup::side)
            .unwrap_or_else(|| group.side())
    }

    /// The shown groups with their sides, in bar order: groups the arrangement places come
    /// first, in its order.
    pub(super) fn columns(
        &self,
        shown: impl IntoIterator<Item = GroupKey>,
    ) -> Vec<(GroupKey, Side)> {
        let mut shown: Vec<_> = shown.into_iter().collect();
        shown.sort_by_key(|group| {
            self.resolved
                .groups()
                .position(|placed| placed.id().as_str() == group.id())
                .unwrap_or(usize::MAX)
        });
        shown
            .into_iter()
            .map(|group| (group, self.side(group)))
            .collect()
    }
}
