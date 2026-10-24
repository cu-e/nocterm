//! The entries of each zone as lists, with the keys and the menus that move them.

use std::{rc::Rc, sync::Arc};

use gpui_kit::{
    AnyElement, App, ElementId, FocusHandle, IntoElement, RenderOnce, SharedString,
    TestSupportExt as _, Window,
    component::{ActiveTheme as _, Icon, Sizable as _, StyledExt as _, h_flex, v_flex},
    div,
    prelude::*,
};
use nocterm_plugin_model::{ContributionId, PluginCatalog, Side, Zone};
use nocterm_ui::{contribution_icon, form};
use nocterm_window_layout::{EditOp, ResolvedGroup, ResolvedItem, ResolvedMember, Slot, Subject};

use crate::{
    commands::Commands,
    dnd::{self, Dragged, Landing, ROW},
    keys,
    menu::ItemMenu,
    parts::{action, tag},
    row::ItemRow,
    snapshot::Snapshot,
};

/// The narrowest a zone's column is drawn. Columns that do not fit side by side wrap onto the
/// next line instead of squeezing each other.
const COLUMN_MIN_REMS: f32 = 11.;

/// What the keys of the lists do, for the line under them.
const HINT: &str = "Drag an item to move it. Keys: Alt+\u{2190}\u{2192} order, Alt+\u{2191}\u{2193} \
                    zone, G group, | separator, Delete hide.";

/// Three columns, Left, Centre and Right, in one focus scope that takes the editor's keys.
#[derive(IntoElement)]
pub(crate) struct Zones {
    snapshot: Rc<Snapshot>,
    catalog: Arc<PluginCatalog>,
    selected: Option<Subject>,
    focus: FocusHandle,
    commands: Commands,
}

impl Zones {
    pub(crate) fn new(
        snapshot: Rc<Snapshot>,
        catalog: Arc<PluginCatalog>,
        selected: Option<Subject>,
        focus: FocusHandle,
        commands: Commands,
    ) -> Self {
        Self {
            snapshot,
            catalog,
            selected,
            focus,
            commands,
        }
    }

    fn column(&self, zone: Zone, cx: &App) -> AnyElement {
        let (name, title) = match zone {
            Zone::Left => ("left", "Left"),
            Zone::Center => ("center", "Centre"),
            Zone::Right => ("right", "Right"),
        };
        let items = self.snapshot.layout().zone(zone);
        let rows: Vec<AnyElement> = items
            .iter()
            .enumerate()
            .map(|(index, item)| match item {
                ResolvedItem::Widget(id) => {
                    self.entry(id, false, Landing::in_zone(zone, index), cx)
                }
                ResolvedItem::Separator => self.separator(
                    Slot::Zone(zone),
                    name,
                    index,
                    Landing::in_zone(zone, index),
                    cx,
                ),
                ResolvedItem::Group(group) => self.group(group, zone, index, cx),
            })
            .collect();
        let theme = cx.theme();
        let column = v_flex()
            .id(SharedString::from(format!("layout-zone:{name}")))
            .test_support()
            .flex_1()
            .min_w(gpui_kit::rems(COLUMN_MIN_REMS))
            .gap_1()
            .p_1()
            .rounded(theme.radius)
            .border_1()
            .border_color(theme.border.opacity(0.))
            .child(
                div()
                    .px_2()
                    .text_xs()
                    .font_semibold()
                    .text_color(cx.theme().muted_foreground)
                    .child(title.to_uppercase()),
            )
            .when(rows.is_empty(), |column| {
                column.child(
                    div()
                        .px_2()
                        .text_sm()
                        .text_color(cx.theme().muted_foreground)
                        .child("Nothing here"),
                )
            })
            .children(rows);
        if !self.snapshot.editable() {
            return column.into_any_element();
        }
        let end = self.end(
            format!("layout-zone-end:{name}"),
            Landing::in_zone(zone, items.len()),
            cx,
        );
        dnd::target(
            column.child(end),
            Landing::zone_area(zone, items.len()),
            true,
            self.commands.clone(),
        )
        .into_any_element()
    }

    /// The empty line after the last entry of a list, where a drop goes last.
    fn end(&self, id: String, landing: Landing, cx: &App) -> AnyElement {
        let line = div()
            .id(SharedString::from(id.clone()))
            .test_support()
            .group(ROW)
            .relative()
            .w_full()
            .h_3()
            .child(dnd::marker(
                SharedString::from(format!("{id}-marker")),
                ROW,
                false,
                landing.clone(),
                cx,
            ));
        dnd::target(line, landing, false, self.commands.clone()).into_any_element()
    }

    fn group(&self, group: &ResolvedGroup, zone: Zone, index: usize, cx: &App) -> AnyElement {
        let subject = Subject::Group(group.id().clone());
        let title = self
            .snapshot
            .groups()
            .iter()
            .find(|spec| spec.id == *group.id())
            .map_or_else(
                || SharedString::from(group.id().as_str().to_owned()),
                |spec| SharedString::from(spec.title),
            );
        let title_text = title.clone();
        let opens = match group.side() {
            Side::Left => "opens left",
            Side::Right => "opens right",
        };
        let label = h_flex()
            .w_full()
            .min_w_0()
            .flex_wrap()
            .gap_x_2()
            .items_center()
            .child(
                div()
                    .min_w_0()
                    .truncate()
                    .text_sm()
                    .font_semibold()
                    .child(title),
            )
            .child(
                div()
                    .flex_shrink_0()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(opens),
            );
        let landing =
            Landing::in_zone(zone, index).and_group_end(group.id().clone(), group.members().len());
        let dragged = Dragged::group(group.id(), title_text, &self.snapshot);
        let header = self.row(
            format!("layout-item:group:{}", group.id()),
            subject,
            label,
            false,
            Some((dragged, landing)),
        );
        let name = group.id().to_string();
        let members = group
            .members()
            .iter()
            .enumerate()
            .map(|(index, member)| match member {
                ResolvedMember::Panel(id) => {
                    self.entry(id, true, Landing::in_group(group.id().clone(), index), cx)
                }
                ResolvedMember::Separator => self.separator(
                    Slot::Group(group.id().clone()),
                    &name,
                    index,
                    Landing::in_group(group.id().clone(), index),
                    cx,
                ),
            });
        let end = self.snapshot.editable().then(|| {
            self.end(
                format!("layout-group-end:{name}"),
                Landing::in_group(group.id().clone(), group.members().len()),
                cx,
            )
        });
        v_flex()
            .w_full()
            .gap_0p5()
            .child(header)
            .children(members)
            .children(end)
            .into_any_element()
    }

    fn entry(&self, id: &ContributionId, member: bool, landing: Landing, cx: &App) -> AnyElement {
        let Some(contribution) = self.catalog.contribution(id) else {
            return div().into_any_element();
        };
        let disabled = self.snapshot.is_disabled(id, &self.catalog);
        let label = h_flex()
            .w_full()
            .min_w_0()
            .flex_wrap()
            .gap_x_2()
            .items_center()
            .child(
                h_flex()
                    .min_w_0()
                    .gap_2()
                    .items_center()
                    .child(
                        div()
                            .flex_shrink_0()
                            .child(Icon::new(contribution_icon(contribution.icon)).small()),
                    )
                    .child(
                        div()
                            .min_w_0()
                            .truncate()
                            .text_sm()
                            .child(contribution.title),
                    ),
            )
            .when(contribution.pinned, |label| {
                label.child(tag(format!("layout-pinned:{id}"), "Always shown", cx))
            })
            .when(disabled, |label| {
                label.child(tag(format!("layout-disabled:{id}"), "Disabled", cx))
            })
            .when(disabled, |label| label.opacity(0.6));
        self.row(
            format!("layout-item:{id}"),
            Subject::Contribution(id.clone()),
            label,
            member,
            Some((Dragged::contribution(contribution, &self.snapshot), landing)),
        )
    }

    fn row(
        &self,
        id: String,
        subject: Subject,
        label: impl IntoElement,
        member: bool,
        drag: Option<(Dragged, Landing)>,
    ) -> AnyElement {
        let menu = ItemMenu::new(
            &subject,
            &self.snapshot,
            &self.catalog,
            self.commands.clone(),
        );
        let selected = self.selected.as_ref() == Some(&subject);
        ItemRow::new(id, subject, label, menu, self.commands.clone())
            .selected(selected)
            .when(member, ItemRow::indented)
            .when_some(
                drag.filter(|_| self.snapshot.editable()),
                |row, (dragged, landing)| row.draggable(dragged, landing),
            )
            .into_any_element()
    }

    fn separator(
        &self,
        place: Slot,
        scope: &str,
        index: usize,
        landing: Landing,
        cx: &App,
    ) -> AnyElement {
        let commands = self.commands.clone();
        let id: ElementId =
            SharedString::from(format!("layout-separator-remove:{scope}:{index}")).into();
        let line = h_flex()
            .id(SharedString::from(format!(
                "layout-separator:{scope}:{index}"
            )))
            .test_support()
            .group(ROW)
            .relative()
            .w_full()
            .px_2()
            .gap_2()
            .items_center()
            .child(div().flex_1().h(gpui_kit::px(1.)).bg(cx.theme().border))
            .child(action(
                id,
                "Remove separator",
                self.snapshot.editable(),
                move |window, cx| {
                    let place = place.clone();
                    commands.run(window, cx, move |section, _, cx| {
                        section.edit(EditOp::RemoveSeparator { place, index }, cx);
                    });
                },
            ));
        if !self.snapshot.editable() {
            return line.into_any_element();
        }
        let line = line.child(dnd::marker(
            SharedString::from(format!("layout-separator-marker:{scope}:{index}")),
            ROW,
            false,
            landing.clone(),
            cx,
        ));
        dnd::target(line, landing, false, self.commands.clone()).into_any_element()
    }
}

impl RenderOnce for Zones {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let commands = self.commands.clone();
        let columns = h_flex()
            .id("layout-zones")
            .test_support()
            .w_full()
            .flex_wrap()
            .gap_x_4()
            .gap_y_3()
            .items_start()
            .child(self.column(Zone::Left, cx))
            .child(self.column(Zone::Center, cx))
            .child(self.column(Zone::Right, cx));
        v_flex()
            .id("layout-lists")
            .test_support()
            .track_focus(&self.focus)
            .on_key_down(move |event, window, cx| {
                if let Some(command) = keys::command(&event.keystroke) {
                    cx.stop_propagation();
                    commands.run(window, cx, move |section, _, cx| section.key(command, cx));
                }
            })
            .w_full()
            .gap_2()
            .child(columns)
            .child(form::note(HINT, cx))
    }
}
