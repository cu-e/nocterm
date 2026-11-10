//! Dragging the items of the bar: what a drag carries, what a drop asks for, how an item starts
//! a drag, and the tray, which takes what is dropped on it out of the bar.
//!
//! The rules stay in `nocterm-window-layout`: a landing only names the list and the place, and
//! the section sends the resulting [`EditOp`] like any other edit. Where a drop over the bar
//! preview lands is worked out by `drops`.

use gpui_kit::{
    App, Bounds, InteractiveElement, ParentElement, Pixels, StatefulInteractiveElement, Styled,
    Window, canvas, component::ActiveTheme as _, prelude::*, px,
};
use nocterm_plugin_model::{ContributionId, ContributionKind, ContributionRef, GroupId};
use nocterm_ui::DragSource;
use nocterm_window_layout::{EditOp, Slot, Subject};

use crate::{
    commands::Commands,
    face::{self, Look},
};

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

/// The payload of a drag: the item and where it came from.
#[derive(Clone, Debug)]
pub(crate) struct Dragged {
    subject: Subject,
    kind: Kind,
    pinned: bool,
    /// The list and place it leaves, when it is in the bar.
    from: Option<(Slot, usize)>,
}

impl Dragged {
    pub(crate) fn new(
        subject: Subject,
        kind: Kind,
        pinned: bool,
        from: Option<(Slot, usize)>,
    ) -> Self {
        Self {
            subject,
            kind,
            pinned,
            from,
        }
    }

    /// A widget or a panel, at `from` in the bar or waiting in the tray.
    pub(crate) fn contribution(
        contribution: ContributionRef<'_>,
        from: Option<(Slot, usize)>,
    ) -> Self {
        // The editor lists no toolbar; one would move like a widget.
        let kind = match contribution.kind() {
            ContributionKind::Widget | ContributionKind::Toolbar => Kind::Widget,
            ContributionKind::Panel => Kind::Panel,
        };
        let subject = Subject::Contribution(contribution.id().clone());
        Self::new(subject, kind, contribution.pinned(), from)
    }

    /// The group `id`, at `from` in the bar.
    pub(crate) fn group(id: &GroupId, from: (Slot, usize)) -> Self {
        Self::new(Subject::Group(id.clone()), Kind::Group, false, Some(from))
    }

    pub(crate) fn subject(&self) -> &Subject {
        &self.subject
    }

    pub(crate) fn kind(&self) -> Kind {
        self.kind
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

/// Where a drop puts the dragged item.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Landing {
    /// In front of the entry at `index` of a zone or a group, or last when `index` is its
    /// length.
    At(Slot, usize),
    /// Into a group with the panel that stands on its own here.
    With(ContributionId),
    /// Out of the bar.
    Tray,
}

impl Landing {
    /// The edit a drop of `dragged` here asks for; `None` when it changes nothing, because the
    /// item is already there or does not go there.
    pub(crate) fn op(&self, dragged: &Dragged) -> Option<EditOp> {
        match (self, &dragged.subject) {
            (Self::At(Slot::Group(_), _), _) if dragged.kind != Kind::Panel => None,
            (Self::At(place, index), subject) => {
                (!dragged.stays(place, *index)).then(|| EditOp::MoveTo {
                    subject: subject.clone(),
                    place: place.clone(),
                    index: *index,
                })
            }
            (Self::With(target), Subject::Contribution(panel))
                if dragged.kind == Kind::Panel && panel != target =>
            {
                Some(EditOp::GroupWith {
                    panel: panel.clone(),
                    target: target.clone(),
                })
            }
            (Self::Tray, Subject::Contribution(id)) if dragged.in_bar() => {
                Some(EditOp::RemoveToTray { id: id.clone() })
            }
            _ => None,
        }
    }

    /// Whether the drop goes in front of the entry at `index` of `place`.
    pub(crate) fn is_at(&self, place: &Slot, index: usize) -> bool {
        matches!(self, Self::At(at, before) if at == place && *before == index)
    }
}

/// Calls `painted` with the box of `element` each time it is painted. `border` is the width
/// of the element's border, which the measuring canvas, filling the padding box, leaves out.
pub(crate) fn measured<E: ParentElement>(
    element: E,
    border: f32,
    painted: impl FnOnce(Bounds<Pixels>, &mut Window, &mut App) + 'static,
) -> E {
    element.child(
        canvas(
            move |bounds, window, cx| painted(bounds.dilate(px(border)), window, cx),
            |_, _, _, _| {},
        )
        .absolute()
        .top_0()
        .left_0()
        .size_full(),
    )
}

/// Makes `element` start a drag of `dragged`. The ghost draws what `look` makes when the drag
/// starts, lifted, in the box `measured` took when `element` was last painted.
pub(crate) fn source<E>(
    element: E,
    dragged: Dragged,
    measured: DragSource,
    look: impl Fn() -> Look + 'static,
) -> E
where
    E: StatefulInteractiveElement,
{
    element.on_drag(dragged, move |_, _, _, cx| {
        cx.stop_propagation();
        let look = look();
        cx.new(|_| measured.preview(move |_, cx| face::ghost(&look, cx)))
    })
}

/// Makes the tray take what is dropped on it out of the bar. It is tinted while that would
/// happen, in the colour of a refusal for an item that is always shown. A drop that changes
/// nothing, like an item of the tray put back on it, is taken all the same and ends the drag.
pub(crate) fn tray<E: InteractiveElement>(element: E, commands: Commands) -> E {
    element
        .on_drop(move |dragged: &Dragged, window, cx| {
            let dragged = dragged.clone();
            commands.run(window, cx, move |section, _, cx| {
                section.dropped(&dragged, &Landing::Tray, cx);
            });
        })
        .drag_over::<Dragged>(|style, dragged, _, cx| {
            if Landing::Tray.op(dragged).is_none() {
                return style;
            }
            let theme = cx.theme();
            let color = if dragged.pinned {
                theme.danger
            } else {
                theme.primary
            };
            style.bg(color.opacity(0.1)).border_color(color)
        })
}

#[cfg(test)]
mod tests {
    use nocterm_plugin_model::{ContributionId, Zone};

    use super::*;

    fn dragged(kind: Kind, from: Option<(Slot, usize)>) -> Dragged {
        let subject = Subject::Contribution(ContributionId::new("a.b"));
        Dragged::new(subject, kind, false, from)
    }

    #[test]
    fn a_widget_lands_in_a_zone_and_a_panel_in_a_group_or_on_its_own_in_a_zone() {
        let widget = dragged(Kind::Widget, Some((Slot::Zone(Zone::Left), 0)));
        let panel = dragged(Kind::Panel, Some((Slot::Group(GroupId::new("g")), 0)));
        let zone = Landing::At(Slot::Zone(Zone::Right), 2);
        let member = Landing::At(Slot::Group(GroupId::new("h")), 1);

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
        assert!(member.op(&widget).is_none(), "a widget never joins a group");
    }

    #[test]
    fn a_panel_dropped_on_a_panel_of_its_own_groups_with_it_and_nothing_else_does() {
        let panel = dragged(Kind::Panel, Some((Slot::Zone(Zone::Left), 0)));
        let widget = dragged(Kind::Widget, Some((Slot::Zone(Zone::Left), 0)));
        let alone = Landing::With(ContributionId::new("c.d"));

        assert!(matches!(alone.op(&panel), Some(EditOp::GroupWith { .. })));
        assert!(
            alone.op(&widget).is_none(),
            "a widget does not make a group"
        );
        let itself = Landing::With(ContributionId::new("a.b"));
        assert!(itself.op(&panel).is_none(), "not with itself");
    }

    #[test]
    fn dropping_on_its_own_place_or_the_next_one_changes_nothing() {
        let widget = dragged(Kind::Widget, Some((Slot::Zone(Zone::Left), 2)));
        let at = |zone, index| Landing::At(Slot::Zone(zone), index);

        assert!(at(Zone::Left, 2).op(&widget).is_none());
        assert!(at(Zone::Left, 3).op(&widget).is_none());
        assert!(at(Zone::Left, 1).op(&widget).is_some());
        assert!(at(Zone::Left, 4).op(&widget).is_some());
        assert!(at(Zone::Right, 2).op(&widget).is_some());
    }

    #[test]
    fn the_tray_takes_what_is_in_the_bar_and_nothing_else() {
        let placed = dragged(Kind::Widget, Some((Slot::Zone(Zone::Left), 0)));
        let waiting = dragged(Kind::Widget, None);

        assert!(matches!(
            Landing::Tray.op(&placed),
            Some(EditOp::RemoveToTray { .. })
        ));
        assert!(Landing::Tray.op(&waiting).is_none());
    }

    #[test]
    fn a_panel_of_the_tray_dropped_on_a_zone_stands_there_on_its_own() {
        let waiting = dragged(Kind::Panel, None);

        assert!(matches!(
            Landing::At(Slot::Zone(Zone::Center), 0).op(&waiting),
            Some(EditOp::MoveTo {
                place: Slot::Zone(Zone::Center),
                index: 0,
                ..
            })
        ));
    }
}
