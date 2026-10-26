//! Dragging the items of the bar: what a drag carries, where a drop on an element lands, and how
//! a target looks while something is over it.
//!
//! The rules stay in `nocterm-window-layout`: a landing only names the list and the place, and
//! the section sends the resulting [`EditOp`] like any other edit.

use std::any::Any;

use gpui_kit::{
    AnyElement, App, ElementId, InteractiveElement, IntoElement, ParentElement, SharedString,
    StatefulInteractiveElement, Styled, TestSupportExt as _,
    base::ElementExt as _,
    component::{ActiveTheme as _, Icon, Sizable as _, h_flex},
    div,
    prelude::*,
    px,
};
use nocterm_plugin_model::{ContributionId, ContributionKind, ContributionRef, GroupId, Zone};
use nocterm_ui::{DragSource, IconName, contribution_icon};
use nocterm_window_layout::{EditOp, Slot, Subject};

use crate::{commands::Commands, snapshot::Snapshot};

/// The hover group of a chip of the bar preview.
pub(crate) const CHIP: &str = "layout-drop-chip";
/// The hover group of a group of the bar preview.
pub(crate) const PILL: &str = "layout-drop-pill";
/// The hover group of the gap in front of an entry of a zone.
pub(crate) const GAP: &str = "layout-drop-gap";
/// How wide the gap between the entries of a zone is.
pub(crate) const GAP_WIDTH: f32 = 6.;
/// How thick the line that marks a place is.
const MARK: f32 = 2.;
/// How tall a chip of the bar preview is.
pub(crate) const CHIP_HEIGHT: f32 = 22.;

/// What kind of thing is dragged, which decides where it may land.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Kind {
    /// A status bar widget: lands in a zone.
    Widget,
    /// A panel: lands in a group, beside a panel on its own, or on its own in a zone.
    Panel,
    /// A panel group: lands in a zone.
    Group,
}

/// The payload of a drag: the item, enough to draw its ghost, and where it came from.
#[derive(Clone, Debug)]
pub(crate) struct Dragged {
    subject: Subject,
    kind: Kind,
    title: SharedString,
    icon: Option<IconName>,
    pinned: bool,
    /// The list and place it leaves, when it is in the bar.
    from: Option<(Slot, usize)>,
}

impl Dragged {
    pub(crate) fn contribution(contribution: ContributionRef<'_>, snapshot: &Snapshot) -> Self {
        let subject = Subject::Contribution(contribution.id().clone());
        Self {
            kind: match contribution.kind() {
                ContributionKind::Widget => Kind::Widget,
                ContributionKind::Panel => Kind::Panel,
            },
            title: contribution.title().to_owned().into(),
            icon: Some(contribution_icon(contribution.icon())),
            pinned: contribution.pinned(),
            from: snapshot.before(&subject),
            subject,
        }
    }

    pub(crate) fn group(id: &GroupId, title: SharedString, snapshot: &Snapshot) -> Self {
        let subject = Subject::Group(id.clone());
        Self {
            kind: Kind::Group,
            title,
            icon: None,
            pinned: false,
            from: snapshot.before(&subject),
            subject,
        }
    }

    fn in_bar(&self) -> bool {
        self.from.is_some()
    }

    /// Whether dropping at `index` of `place` leaves it where it is: on its own place, or right
    /// after it.
    fn stays(&self, place: &Slot, index: usize) -> bool {
        matches!(&self.from, Some((held, at)) if held == place && (index == *at || index == at + 1))
    }
}

/// Where a drop on one element puts the dragged item. An element names a place for what goes
/// into a zone, one for a panel that goes into a group, a panel on its own that a dropped panel
/// makes a group with, or the tray.
#[derive(Clone, Debug, Default)]
pub(crate) struct Landing {
    zone: Option<(Zone, usize)>,
    group: Option<(GroupId, usize)>,
    beside: Option<ContributionId>,
    tray: bool,
    /// The element is the whole zone, so it fills while a widget or a group is over it.
    zone_fills: bool,
    /// The element is the whole group, so it fills while a panel is over it.
    group_fills: bool,
}

impl Landing {
    /// In front of the entry at `index` of a zone, or last when `index` is its length.
    pub(crate) fn in_zone(zone: Zone, index: usize) -> Self {
        Self {
            zone: Some((zone, index)),
            ..Self::default()
        }
    }

    /// In front of the member at `index` of a group, or last when `index` is its length.
    pub(crate) fn in_group(group: GroupId, index: usize) -> Self {
        Self {
            group: Some((group, index)),
            ..Self::default()
        }
    }

    /// A panel dropped here makes a group with `panel`, which is on its own; anything else lands
    /// at `zone` in front of it.
    pub(crate) fn beside(panel: ContributionId, zone: Zone, index: usize) -> Self {
        Self {
            beside: Some(panel),
            ..Self::in_zone(zone, index)
        }
    }

    /// Out of the bar.
    pub(crate) fn tray() -> Self {
        Self {
            tray: true,
            ..Self::default()
        }
    }

    /// The end of a zone, as the whole area of the zone.
    pub(crate) fn zone_area(zone: Zone, len: usize) -> Self {
        Self {
            zone_fills: true,
            ..Self::in_zone(zone, len)
        }
    }

    /// A panel dropped here goes to the end of `group`, whose `members` it has.
    pub(crate) fn and_group_end(mut self, group: GroupId, members: usize) -> Self {
        self.group = Some((group, members));
        self.group_fills = true;
        self
    }

    /// The edit a drop of `dragged` here asks for; `None` when it does not fit this place or
    /// would leave it where it is.
    pub(crate) fn op(&self, dragged: &Dragged) -> Option<EditOp> {
        if let (Kind::Panel, Some(target), Subject::Contribution(panel)) =
            (dragged.kind, &self.beside, &dragged.subject)
        {
            return (panel != target).then(|| EditOp::GroupWith {
                panel: panel.clone(),
                target: target.clone(),
            });
        }
        let in_zone = self.zone.map(|(zone, index)| (Slot::Zone(zone), index));
        let wanted = match dragged.kind {
            Kind::Widget | Kind::Group => in_zone,
            Kind::Panel => self
                .group
                .clone()
                .map(|(group, index)| (Slot::Group(group), index))
                .or(in_zone),
        };
        if let Some((place, index)) = wanted {
            return (!dragged.stays(&place, index)).then(|| EditOp::MoveTo {
                subject: dragged.subject.clone(),
                place,
                index,
            });
        }
        let Subject::Contribution(id) = &dragged.subject else {
            return None;
        };
        (self.tray && dragged.in_bar()).then(|| EditOp::RemoveToTray { id: id.clone() })
    }

    fn accepts(&self, value: &dyn Any) -> bool {
        value
            .downcast_ref::<Dragged>()
            .is_some_and(|dragged| self.op(dragged).is_some())
    }

    /// Whether a drop of `value` here goes in front of the element, which a line shows: it
    /// fits, and the element as a whole does not take it.
    fn marks(&self, value: &dyn Any) -> bool {
        value
            .downcast_ref::<Dragged>()
            .is_some_and(|dragged| self.op(dragged).is_some() && !self.fills(dragged))
    }

    /// Whether the element as a whole is where `dragged` would go, and so fills.
    fn fills(&self, dragged: &Dragged) -> bool {
        match dragged.kind {
            Kind::Panel => {
                self.group_fills || self.tray || self.zone_fills || self.beside.is_some()
            }
            Kind::Widget => self.zone_fills || self.tray,
            Kind::Group => self.zone_fills,
        }
    }
}

/// Makes `element` start a drag of `dragged`, with a ghost that looks like the item.
pub(crate) fn source<E>(element: E, dragged: Dragged) -> E
where
    E: StatefulInteractiveElement + ParentElement,
{
    let measured = DragSource::default();
    let capture = measured.clone();
    element
        .on_prepaint(move |bounds, window, _| capture.capture(bounds, window))
        .on_drag(dragged.clone(), move |_, _, _, cx| {
            cx.stop_propagation();
            let dragged = dragged.clone();
            cx.new(|_| measured.preview(move |_, cx| ghost(&dragged, cx)))
        })
}

/// Makes `element` take a drop at `landing`. With `fill`, it is tinted while something that
/// goes into it as a whole is over it; a pinned item over the tray shows as refused.
pub(crate) fn target<E: InteractiveElement>(
    element: E,
    landing: Landing,
    fill: bool,
    commands: Commands,
) -> E {
    let (fits, shown, dropped) = (landing.clone(), landing.clone(), landing);
    let element = element
        .can_drop(move |value, _, _| fits.accepts(value))
        .on_drop(move |dragged: &Dragged, window, cx| {
            let (dragged, landing) = (dragged.clone(), dropped.clone());
            commands.run(window, cx, move |section, _, cx| {
                section.dropped(&dragged, &landing, cx);
            });
        });
    if !fill {
        return element;
    }
    element.drag_over::<Dragged>(move |style, dragged, _, cx| {
        if !shown.fills(dragged) {
            return style;
        }
        let theme = cx.theme();
        let refused = shown.tray && dragged.pinned;
        let color = if refused { theme.danger } else { theme.primary };
        style.bg(color.opacity(0.1)).border_color(color)
    })
}

/// A line in front of an element that shows where a drop would land. It follows the hover group
/// `group`, which the element it is placed in must carry, and shows only for a drag that
/// `landing` puts in front of the element rather than into it.
pub(crate) fn marker(
    id: impl Into<ElementId>,
    group: &'static str,
    landing: Landing,
    cx: &App,
) -> AnyElement {
    let color = cx.theme().primary;
    div()
        .id(id)
        .test_support()
        .absolute()
        .invisible()
        .cursor_default()
        .top_0()
        .bottom_0()
        .left(px(-(GAP_WIDTH + MARK) / 2.))
        .w(px(MARK))
        .rounded_full()
        .can_drop(move |value, _, _| landing.marks(value))
        .group_drag_over::<Dragged>(group, move |style| style.visible().bg(color))
        .into_any_element()
}

/// The room in front of an entry of a zone, or after its last one, that takes a drop at
/// `landing`: a panel dropped here stands on its own rather than joining its neighbour.
pub(crate) fn gap(
    id: impl Into<ElementId>,
    landing: Landing,
    commands: Commands,
    cx: &App,
) -> AnyElement {
    let color = cx.theme().primary;
    let fits = landing.clone();
    let line = div()
        .absolute()
        .invisible()
        .top_0()
        .bottom_0()
        .left(px((GAP_WIDTH - MARK) / 2.))
        .w(px(MARK))
        .rounded_full()
        .can_drop(move |value, _, _| fits.accepts(value))
        .group_drag_over::<Dragged>(GAP, move |style| style.visible().bg(color));
    let gap = div()
        .id(id)
        .test_support()
        .group(GAP)
        .relative()
        .flex_none()
        .w(px(GAP_WIDTH))
        .h(px(CHIP_HEIGHT))
        .child(line);
    target(gap, landing, false, commands).into_any_element()
}

/// The ghost that follows the pointer: the item as a small chip.
fn ghost(dragged: &Dragged, cx: &App) -> AnyElement {
    let theme = cx.theme();
    h_flex()
        .id("layout-drag-ghost")
        .test_support()
        .gap_1p5()
        .px_2()
        .py_1()
        .items_center()
        .rounded(theme.radius)
        .border_1()
        .border_color(theme.primary)
        .bg(theme.popover)
        .shadow_md()
        .when_some(dragged.icon, |chip, icon| {
            chip.child(Icon::new(icon).small())
        })
        .child(div().text_sm().child(dragged.title.clone()))
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use nocterm_plugin_model::ContributionId;

    use super::*;

    fn dragged(kind: Kind, from: Option<(Slot, usize)>) -> Dragged {
        Dragged {
            subject: Subject::Contribution(ContributionId::new("a.b")),
            kind,
            title: "Item".into(),
            icon: None,
            pinned: false,
            from,
        }
    }

    #[test]
    fn a_widget_lands_in_a_zone_and_a_panel_in_a_group_or_on_its_own_in_a_zone() {
        let widget = dragged(Kind::Widget, Some((Slot::Zone(Zone::Left), 0)));
        let panel = dragged(Kind::Panel, Some((Slot::Group(GroupId::new("g")), 0)));
        let zone = Landing::in_zone(Zone::Right, 2);
        let member = Landing::in_group(GroupId::new("h"), 1);

        assert!(zone.op(&widget).is_some());
        assert!(matches!(
            zone.op(&panel),
            Some(EditOp::MoveTo {
                place: Slot::Zone(Zone::Right),
                index: 2,
                ..
            })
        ));
        assert!(matches!(
            member.op(&panel),
            Some(EditOp::MoveTo {
                place: Slot::Group(_),
                index: 1,
                ..
            })
        ));
        assert!(
            member.op(&widget).is_none(),
            "the group around it takes a widget"
        );
    }

    #[test]
    fn a_panel_dropped_on_a_panel_of_its_own_groups_with_it_and_a_widget_lands_in_front() {
        let panel = dragged(Kind::Panel, Some((Slot::Zone(Zone::Left), 0)));
        let widget = dragged(Kind::Widget, Some((Slot::Zone(Zone::Left), 0)));
        let alone = Landing::beside(ContributionId::new("c.d"), Zone::Left, 3);

        assert!(matches!(alone.op(&panel), Some(EditOp::GroupWith { .. })));
        assert!(matches!(
            alone.op(&widget),
            Some(EditOp::MoveTo { index: 3, .. })
        ));
        let itself = Landing::beside(ContributionId::new("a.b"), Zone::Left, 0);
        assert!(itself.op(&panel).is_none(), "not on itself");
    }

    #[test]
    fn dropping_on_its_own_place_or_the_next_one_changes_nothing() {
        let widget = dragged(Kind::Widget, Some((Slot::Zone(Zone::Left), 2)));

        assert!(Landing::in_zone(Zone::Left, 2).op(&widget).is_none());
        assert!(Landing::in_zone(Zone::Left, 3).op(&widget).is_none());
        assert!(Landing::in_zone(Zone::Left, 1).op(&widget).is_some());
        assert!(Landing::in_zone(Zone::Left, 4).op(&widget).is_some());
        assert!(Landing::in_zone(Zone::Right, 2).op(&widget).is_some());
    }

    #[test]
    fn the_tray_takes_what_is_in_the_bar_and_nothing_else() {
        let placed = dragged(Kind::Widget, Some((Slot::Zone(Zone::Left), 0)));
        let waiting = dragged(Kind::Widget, None);

        assert!(matches!(
            Landing::tray().op(&placed),
            Some(EditOp::RemoveToTray { .. })
        ));
        assert!(Landing::tray().op(&waiting).is_none());
    }

    #[test]
    fn a_panel_of_the_tray_dropped_on_a_zone_stands_there_on_its_own() {
        let waiting = dragged(Kind::Panel, None);

        assert!(matches!(
            Landing::zone_area(Zone::Center, 0).op(&waiting),
            Some(EditOp::MoveTo {
                place: Slot::Zone(Zone::Center),
                index: 0,
                ..
            })
        ));
    }
}
