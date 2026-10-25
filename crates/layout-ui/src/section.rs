//! The editor entity: which layout and item are shown, the writes it sends and what they led to.

use std::{rc::Rc, sync::Arc};

use gpui_kit::{
    App, AppContext as _, Context, Entity, FocusHandle, Focusable, InteractiveElement as _,
    IntoElement, ParentElement as _, Render, SharedString, Styled as _, Subscription,
    TestSupportExt as _, Window,
    component::{h_flex, v_flex},
};
use nocterm_plugin_enablement::PluginEnablement;
use nocterm_plugin_model::{ContributionKind, PluginSet, Zone};
use nocterm_store::SettingsHandle;
use nocterm_ui::{
    FormFrame, FormScope, PluginSettingsDirectory, PluginSettingsForm, PluginSource, SettingsPage,
};
use nocterm_window_layout::{
    EditError, EditOp, EditOutcome, LayoutSettings, PresetId, Subject, select,
};
use nocterm_workspace::{LayoutEditError, edit_layouts};

use crate::{
    commands::Commands,
    dnd::{Dragged, Landing},
    fit::Fit,
    history::{History, Step},
    inspector::Inspector,
    keys::KeyCommand,
    presets::Presets,
    preview::Preview,
    rename::RenameDraft,
    selection::Selection,
    snapshot::Snapshot,
    tray::Tray,
    zones::Zones,
};

/// What the section is asking the user about, apart from the layout itself.
pub(crate) enum Mode {
    Idle,
    /// A new name is being typed for the layout.
    Renaming(RenameDraft),
    /// The layout shown is about to be deleted.
    ConfirmingDelete,
}

/// A line under the picker about what just happened.
pub(crate) struct Notice {
    text: SharedString,
    failed: bool,
    /// Whether the line offers to undo the change.
    undoable: bool,
}

/// The "Status bar and panels" section of Appearance: the arrangement of the status bar and the
/// panel groups, edited in place, with the settings of the selected item beside it.
pub struct LayoutEditorSection {
    settings: SettingsHandle,
    /// Where the plugins come from, which changes while the page is open when the user installs
    /// or removes a plugin.
    source: Rc<dyn PluginSource>,
    catalog: Arc<PluginSet>,
    legacy_swapped: bool,
    selection: Selection,
    history: History,
    plugin_settings: PluginSettingsDirectory,
    /// The form of the selected item, when its plugin declares settings for it.
    form: Option<Entity<PluginSettingsForm>>,
    /// Writes sent and not yet settled.
    pending: usize,
    mode: Mode,
    notice: Option<Notice>,
    focus: FocusHandle,
    /// The width the bar preview was drawn with, which decides how much of it is shown.
    fit: Fit,
    _subscriptions: Vec<Subscription>,
}

impl LayoutEditorSection {
    pub fn new(
        settings: SettingsHandle,
        source: Rc<dyn PluginSource>,
        plugin_settings: PluginSettingsDirectory,
        legacy_swapped: bool,
        cx: &mut Context<Self>,
    ) -> Self {
        settings.register::<LayoutSettings>(cx);
        settings.register::<PluginEnablement>(cx);
        let history = History::new(settings.setting::<LayoutSettings>(cx).presets.clone());
        let weak = cx.weak_entity();
        let changed = settings.observe(cx, move |cx| {
            let _ = weak.update(cx, |section, cx| section.stored_changed(cx));
        });
        let catalog = source.current(cx);
        let weak = cx.weak_entity();
        let plugins = source.observe(
            Box::new(move |cx| {
                let _ = weak.update(cx, |section, cx| section.plugins_changed(cx));
            }),
            cx,
        );
        Self {
            settings,
            source,
            catalog,
            legacy_swapped,
            selection: Selection::default(),
            history,
            plugin_settings,
            form: None,
            pending: 0,
            mode: Mode::Idle,
            notice: None,
            focus: cx.focus_handle(),
            fit: Fit::default(),
            _subscriptions: vec![changed, plugins],
        }
    }

    pub(crate) fn snapshot(&self, cx: &App) -> Snapshot {
        Snapshot::new(
            self.settings.setting::<LayoutSettings>(cx),
            self.settings.setting::<PluginEnablement>(cx),
            self.legacy_swapped,
            self.selection.shown(),
            &self.catalog,
        )
    }

    pub(crate) fn catalog_arc(&self) -> Arc<PluginSet> {
        self.catalog.clone()
    }

    pub(crate) fn fit(&self) -> Fit {
        self.fit.clone()
    }

    pub(crate) fn focus_handle_ref(&self) -> &FocusHandle {
        &self.focus
    }

    pub(crate) fn item(&self) -> Option<&Subject> {
        self.selection.item()
    }

    pub(crate) fn form(&self) -> Option<&Entity<PluginSettingsForm>> {
        self.form.as_ref()
    }

    pub(crate) fn mode(&self) -> &Mode {
        &self.mode
    }

    pub(crate) fn notice(&self) -> Option<&Notice> {
        self.notice.as_ref()
    }

    /// Whether an undo is possible now. A step restores the whole list the history kept, so it
    /// is not offered while a write is on its way: it would restore a list that the write is
    /// about to replace.
    pub(crate) fn can_undo(&self) -> bool {
        self.pending == 0 && self.history.can_undo()
    }

    /// Whether a redo is possible now, on the same terms as an undo.
    pub(crate) fn can_redo(&self) -> bool {
        self.pending == 0 && self.history.can_redo()
    }

    fn in_use(&self, cx: &App) -> PresetId {
        select(
            self.settings.setting::<LayoutSettings>(cx),
            self.legacy_swapped,
        )
    }

    /// Selects an item of the layout, or nothing, and shows the settings it has.
    pub(crate) fn pick(
        &mut self,
        item: Option<Subject>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.flush_form(cx);
        self.selection.pick(item);
        self.form = self.form_for_item(window, cx);
        window.focus(&self.focus, cx);
        cx.notify();
    }

    fn form_for_item(
        &self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<Entity<PluginSettingsForm>> {
        let Some(Subject::Contribution(id)) = self.selection.item() else {
            return None;
        };
        let plugin = self.catalog.plugin_of(id)?;
        let (schema, source) = self.plugin_settings.get(plugin.id(), cx)?;
        schema.for_contribution(id).next()?;
        let (plugin, scope) = (plugin.id().clone(), FormScope::Contribution(id.clone()));
        Some(cx.new(|cx| {
            PluginSettingsForm::new(source, plugin, schema, scope, FormFrame::Host, window, cx)
        }))
    }

    fn flush_form(&self, cx: &mut Context<Self>) {
        if let Some(form) = &self.form {
            form.update(cx, |form, cx| form.flush(cx));
        }
    }

    /// Shows another layout in the editor. It does not change the layout in use.
    pub(crate) fn show(&mut self, id: PresetId, window: &mut Window, cx: &mut Context<Self>) {
        self.flush_form(cx);
        let in_use = self.in_use(cx);
        self.selection.show(id, &in_use);
        self.form = None;
        self.mode = Mode::Idle;
        self.notice = None;
        window.focus(&self.focus, cx);
        cx.notify();
    }

    /// Sends one operation on the layout shown.
    pub(crate) fn edit(&mut self, op: EditOp, cx: &mut Context<Self>) {
        self.send(op, Step::Edit, cx);
    }

    /// Puts a dragged item where it was dropped, by the same rules as every other edit.
    pub(crate) fn dropped(&mut self, dragged: &Dragged, landing: &Landing, cx: &mut Context<Self>) {
        if !self.snapshot(cx).editable() {
            return;
        }
        if let Some(op) = landing.op(dragged) {
            self.edit(op, cx);
        }
    }

    pub(crate) fn undo(&mut self, cx: &mut Context<Self>) {
        if !self.can_undo() {
            return;
        }
        if let Some(presets) = self.history.previous().cloned() {
            self.send(EditOp::RestorePresets(presets), Step::Undo, cx);
        }
    }

    pub(crate) fn redo(&mut self, cx: &mut Context<Self>) {
        if !self.can_redo() {
            return;
        }
        if let Some(presets) = self.history.next().cloned() {
            self.send(EditOp::RestorePresets(presets), Step::Redo, cx);
        }
    }

    fn send(&mut self, op: EditOp, step: Step, cx: &mut Context<Self>) {
        // The notice names the layout as it is before the write: a deleted one is not found
        // after it.
        let name = self.snapshot(cx).name().to_owned();
        let target = self.selection.target(&self.in_use(cx));
        let written = edit_layouts(
            &self.settings,
            self.catalog.clone(),
            self.legacy_swapped,
            target,
            op.clone(),
            cx,
        );
        self.pending += 1;
        cx.spawn(async move |section, cx| {
            let result = written.await;
            let _ = section.update(cx, |section, cx| {
                section.settled(op, step, &name, result, cx)
            });
        })
        .detach();
    }

    fn settled(
        &mut self,
        op: EditOp,
        step: Step,
        name: &str,
        result: Result<Option<EditOutcome>, LayoutEditError>,
        cx: &mut Context<Self>,
    ) {
        self.pending = self.pending.saturating_sub(1);
        match result {
            Ok(Some(outcome)) => self.stored(&op, step, name, &outcome, cx),
            Ok(None) => {}
            Err(error) => self.refused(&op, &error),
        }
        self.stored_changed(cx);
        cx.notify();
    }

    fn stored(
        &mut self,
        op: &EditOp,
        step: Step,
        name: &str,
        outcome: &EditOutcome,
        cx: &mut Context<Self>,
    ) {
        self.history.record(step, outcome);
        let in_use = self.in_use(cx);
        let settings = self.settings.setting::<LayoutSettings>(cx).clone();
        self.selection.followed(op, outcome, &in_use, |id| {
            nocterm_window_layout::BuiltIn::of(id).is_some() || settings.exists(id)
        });
        self.notice = match op {
            EditOp::Delete => Some(Notice::undoable(format!("Deleted \u{201c}{name}\u{201d}."))),
            EditOp::Reset => Some(Notice::undoable(format!("Reset \u{201c}{name}\u{201d}."))),
            _ => None,
        };
        if matches!(op, EditOp::Rename { .. } | EditOp::Delete) {
            self.mode = Mode::Idle;
        }
    }

    fn refused(&mut self, op: &EditOp, error: &LayoutEditError) {
        let text: SharedString = match error {
            LayoutEditError::Refused(EditError::Pinned(id)) => {
                let title = self
                    .catalog
                    .contribution(id)
                    .map_or(id.as_str(), |it| it.title());
                format!("\u{201c}{title}\u{201d} is always shown and cannot be hidden.").into()
            }
            LayoutEditError::Store(other) => format!("Could not save the layout: {other}").into(),
            refusal => refusal.to_string().into(),
        };
        match (op, &mut self.mode) {
            (EditOp::Rename { .. }, Mode::Renaming(draft)) => draft.refuse(text),
            _ => self.notice = Some(Notice::failed(text)),
        }
    }

    /// Reads the plugins again after one was installed, replaced or removed. An item that is no
    /// longer in the set leaves the selection.
    fn plugins_changed(&mut self, cx: &mut Context<Self>) {
        self.catalog = self.source.current(cx);
        if let Some(Subject::Contribution(id)) = self.selection.item()
            && self.catalog.contribution(id).is_none()
        {
            self.selection.clear();
            self.form = None;
        }
        cx.notify();
    }

    /// Forgets the history when the layouts changed by someone else, once no write of ours is
    /// on its way.
    fn stored_changed(&mut self, cx: &mut Context<Self>) {
        if self.pending == 0 {
            let presets = self.settings.setting::<LayoutSettings>(cx).presets.clone();
            if self.history.observe(&presets) {
                self.notice = None;
            }
        }
        cx.notify();
    }

    pub(crate) fn dismiss_notice(&mut self, cx: &mut Context<Self>) {
        self.notice = None;
        cx.notify();
    }

    pub(crate) fn begin_rename(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let snapshot = self.snapshot(cx);
        if snapshot.built_in() {
            return;
        }
        self.notice = None;
        self.mode = Mode::Renaming(RenameDraft::new(
            snapshot.shown().clone(),
            snapshot.name().to_owned(),
            window,
            cx,
        ));
        cx.notify();
    }

    pub(crate) fn commit_rename(&mut self, _: &mut Window, cx: &mut Context<Self>) {
        let settings = self.settings.setting::<LayoutSettings>(cx).clone();
        let Mode::Renaming(draft) = &mut self.mode else {
            return;
        };
        if let Some(name) = draft.checked(&settings, cx) {
            self.edit(EditOp::Rename { name }, cx);
        }
        cx.notify();
    }

    pub(crate) fn clear_rename_error(&mut self, cx: &mut Context<Self>) {
        if let Mode::Renaming(draft) = &mut self.mode {
            draft.clear_error();
            cx.notify();
        }
    }

    pub(crate) fn cancel_rename(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if matches!(self.mode, Mode::Renaming(_)) {
            self.mode = Mode::Idle;
            window.focus(&self.focus, cx);
            cx.notify();
        }
    }

    pub(crate) fn ask_delete(&mut self, cx: &mut Context<Self>) {
        let snapshot = self.snapshot(cx);
        if !snapshot.built_in() {
            self.notice = None;
            self.mode = Mode::ConfirmingDelete;
            cx.notify();
        }
    }

    pub(crate) fn cancel_delete(&mut self, cx: &mut Context<Self>) {
        self.mode = Mode::Idle;
        cx.notify();
    }

    pub(crate) fn confirm_delete(&mut self, cx: &mut Context<Self>) {
        if matches!(self.mode, Mode::ConfirmingDelete) {
            self.edit(EditOp::Delete, cx);
        }
    }

    /// Runs a key of the editor on the selected item.
    pub(crate) fn key(&mut self, command: KeyCommand, cx: &mut Context<Self>) {
        match command {
            KeyCommand::Undo => return self.undo(cx),
            KeyCommand::Redo => return self.redo(cx),
            _ => {}
        }
        let snapshot = self.snapshot(cx);
        let Some(subject) = self
            .selection
            .item()
            .cloned()
            .filter(|_| snapshot.editable())
        else {
            return;
        };
        if let Some(op) = self.op_for(command, &subject, &snapshot) {
            self.edit(op, cx);
        }
    }

    fn op_for(
        &self,
        command: KeyCommand,
        subject: &Subject,
        snapshot: &Snapshot,
    ) -> Option<EditOp> {
        match command {
            KeyCommand::Move(by) => Some(EditOp::Move {
                subject: subject.clone(),
                by,
            }),
            KeyCommand::Zone(step) => {
                let moved = self.movable(subject, snapshot)?;
                let zones = [Zone::Left, Zone::Center, Zone::Right];
                let at = zones
                    .iter()
                    .position(|zone| Some(*zone) == snapshot.zone_of(&moved))?;
                let to = at.checked_add_signed(step as isize)?;
                Some(EditOp::ToZone {
                    subject: moved,
                    zone: *zones.get(to)?,
                })
            }
            KeyCommand::NextGroup => {
                let Subject::Contribution(panel) = subject else {
                    return None;
                };
                if self.catalog.contribution(panel)?.kind() != ContributionKind::Panel {
                    return None;
                }
                let groups = &snapshot.groups();
                let current = snapshot.group_of(panel).map(|group| group.id().clone());
                let at = groups
                    .iter()
                    .position(|group| Some(&group.id) == current.as_ref());
                let next = groups.get(at.map_or(0, |at| (at + 1) % groups.len()))?;
                Some(EditOp::ToGroup {
                    panel: panel.clone(),
                    group: next.id.clone(),
                })
            }
            KeyCommand::Separator => {
                let (place, index) = snapshot.after(subject)?;
                Some(EditOp::InsertSeparator { place, index })
            }
            KeyCommand::Remove => match subject {
                Subject::Contribution(id) => Some(EditOp::RemoveToTray { id: id.clone() }),
                Subject::Group(_) => None,
            },
            KeyCommand::Undo | KeyCommand::Redo => None,
        }
    }

    /// What moves between zones when `subject` is asked to: a widget, or the group of a panel.
    fn movable(&self, subject: &Subject, snapshot: &Snapshot) -> Option<Subject> {
        match subject {
            Subject::Group(_) => Some(subject.clone()),
            Subject::Contribution(id) => match self.catalog.contribution(id)?.kind() {
                ContributionKind::Widget => Some(subject.clone()),
                ContributionKind::Panel => snapshot
                    .group_of(id)
                    .map(|group| Subject::Group(group.id().clone())),
            },
        }
    }
}

impl Notice {
    pub(crate) fn text(&self) -> &SharedString {
        &self.text
    }

    pub(crate) fn is_failure(&self) -> bool {
        self.failed
    }

    pub(crate) fn offers_undo(&self) -> bool {
        self.undoable
    }

    fn undoable(text: String) -> Self {
        Self {
            text: text.into(),
            failed: false,
            undoable: true,
        }
    }

    fn failed(text: SharedString) -> Self {
        Self {
            text,
            failed: true,
            undoable: false,
        }
    }
}

impl SettingsPage for LayoutEditorSection {
    fn flush(&mut self, _: &mut Window, cx: &mut Context<Self>) {
        self.flush_form(cx);
    }

    fn on_deactivate(&mut self, _: &mut Window, cx: &mut Context<Self>) {
        self.flush_form(cx);
        self.mode = Mode::Idle;
    }

    /// Selects the contribution or panel group whose id is `anchor`.
    fn reveal(&mut self, anchor: &str, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let subject = self
            .catalog
            .entries()
            .find(|entry| entry.id().as_str() == anchor)
            .map(|entry| Subject::Contribution(entry.id().clone()))
            .or_else(|| {
                self.catalog
                    .group(anchor)
                    .map(|group| Subject::Group(group.id.clone()))
            });
        let known = subject.is_some();
        if known {
            self.pick(subject, window, cx);
        }
        known
    }
}

impl Focusable for LayoutEditorSection {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

/// The narrowest the lists are drawn: two zone columns side by side.
const LISTS_MIN_REMS: f32 = 24.;

impl Render for LayoutEditorSection {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let snapshot = Rc::new(self.snapshot(cx));
        let commands = Commands::new(cx.weak_entity());
        let selected = self.item().cloned();
        let catalog = self.catalog_arc();
        let inspector = Inspector::new(
            snapshot.clone(),
            catalog.clone(),
            selected.clone(),
            self.form().cloned(),
            commands.clone(),
        );
        // The bar has the full width on its own line. Below it the lists and the inspector sit
        // side by side while each has its minimum width, and wrap into one column when not.
        v_flex()
            .id("layout-editor")
            .test_support()
            .w_full()
            .min_w_0()
            .gap_4()
            .child(Presets::new(self, snapshot.clone(), commands.clone()))
            .child(Preview::new(
                snapshot.clone(),
                catalog.clone(),
                selected.clone(),
                self.fit(),
                commands.clone(),
            ))
            .child(
                h_flex()
                    .id("layout-body")
                    .test_support()
                    .w_full()
                    .flex_wrap()
                    .gap_x_4()
                    .gap_y_4()
                    .items_start()
                    .child(
                        v_flex()
                            .id("layout-lists-column")
                            .test_support()
                            .flex_1()
                            .min_w(gpui_kit::rems(LISTS_MIN_REMS))
                            .gap_4()
                            .child(Zones::new(
                                snapshot.clone(),
                                catalog.clone(),
                                selected.clone(),
                                self.focus_handle_ref().clone(),
                                commands.clone(),
                            ))
                            .child(Tray::new(snapshot, catalog, selected, commands)),
                    )
                    .child(inspector),
            )
    }
}
