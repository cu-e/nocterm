//! Where a drop over the bar preview lands, worked out from where the pointer is against what
//! the preview painted last.
//!
//! The half of an entry the pointer is over puts the drop in front of the entry or after it.
//! A panel over a group goes among its members, so the outer halves of the group's first and
//! last members are its start and its end; a widget or a group over a group goes beside the
//! group. A panel over the middle of a panel on its own makes a group with it. Away from every
//! entry the nearest zone takes the drop, so a drop over the preview always lands somewhere,
//! and the preview marks that place while the item is held over it.

use gpui_kit::{Bounds, Context, Entity, InteractiveElement, Pixels, Point};
use nocterm_plugin_model::{ContributionId, GroupId, Zone};
use nocterm_window_layout::{Slot, Subject};

use crate::{
    commands::Commands,
    dnd::{Dragged, Kind, Landing},
};

/// What an entry of the preview holds, as far as a drop over it goes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Holds {
    /// A widget, a separator or a member of a group.
    Item,
    /// A panel that stands on its own.
    Alone(ContributionId),
    /// A group, with the members recorded under its own slot.
    Group(GroupId),
}

/// One entry of the preview: the entry at `index` of a zone, or a member of a group.
#[derive(Clone, Debug)]
pub(crate) struct Spot {
    place: Slot,
    index: usize,
    holds: Holds,
}

impl Spot {
    pub(crate) fn new(place: Slot, index: usize, holds: Holds) -> Self {
        Self {
            place,
            index,
            holds,
        }
    }

    /// The list and place a drag of this entry leaves.
    pub(crate) fn from(&self) -> (Slot, usize) {
        (self.place.clone(), self.index)
    }
}

/// The preview as it was last painted, and the drag held over it.
#[derive(Debug, Default)]
pub(crate) struct Drops {
    zones: Vec<(Zone, Bounds<Pixels>)>,
    spots: Vec<(Spot, Bounds<Pixels>)>,
    /// Where the item held over the preview would land, when that changes something.
    aimed: Option<Landing>,
    /// The item being dragged, which its place in the preview shows as moving.
    moving: Option<Subject>,
}

impl Drops {
    /// Forgets what was painted, before the preview is drawn again.
    pub(crate) fn repaint(&mut self) {
        self.zones.clear();
        self.spots.clear();
    }

    pub(crate) fn zone(&mut self, zone: Zone, bounds: Bounds<Pixels>) {
        self.zones.push((zone, bounds));
    }

    pub(crate) fn spot(&mut self, spot: Spot, bounds: Bounds<Pixels>) {
        self.spots.push((spot, bounds));
    }

    pub(crate) fn aimed(&self) -> Option<&Landing> {
        self.aimed.as_ref()
    }

    pub(crate) fn moving(&self) -> Option<&Subject> {
        self.moving.as_ref()
    }

    /// Where a drop of `dragged` at `at` lands; `None` before the preview was painted.
    pub(crate) fn landing(&self, at: Point<Pixels>, dragged: &Dragged) -> Option<Landing> {
        let x = at.x;
        let (zone, _) = self
            .zones
            .iter()
            .min_by(|(_, a), (_, b)| distance(a, x).total_cmp(&distance(b, x)))?;
        let place = Slot::Zone(*zone);
        let under = self
            .spots
            .iter()
            .find(|(spot, bounds)| spot.place == place && bounds.left() <= x && x < bounds.right());
        if let (Some((spot, bounds)), Kind::Panel) = (under, dragged.kind()) {
            match &spot.holds {
                Holds::Group(group) => {
                    let members = Slot::Group(group.clone());
                    let index = self.before(&members, x);
                    return Some(Landing::At(members, index));
                }
                Holds::Alone(panel) if middle(bounds, x) => {
                    return Some(Landing::With(panel.clone()));
                }
                _ => {}
            }
        }
        let index = self.before(&place, x);
        Some(Landing::At(place, index))
    }

    /// Follows the pointer of a drag of `dragged`; `at` is `None` while it is away from the
    /// preview. Returns whether what the preview shows changed.
    pub(crate) fn aim(&mut self, at: Option<Point<Pixels>>, dragged: &Dragged) -> bool {
        let aimed = at
            .and_then(|at| self.landing(at, dragged))
            .filter(|landing| landing.op(dragged).is_some());
        let moving = Some(dragged.subject().clone());
        let changed = aimed != self.aimed || moving != self.moving;
        self.aimed = aimed;
        self.moving = moving;
        changed
    }

    /// Forgets the drag, which ended. Returns whether what the preview shows changed.
    pub(crate) fn settle(&mut self) -> bool {
        let changed = self.aimed.is_some() || self.moving.is_some();
        self.aimed = None;
        self.moving = None;
        changed
    }

    /// How many entries of `place` are left of `x` by their middle: the index a drop at `x`
    /// takes.
    fn before(&self, place: &Slot, x: Pixels) -> usize {
        self.spots
            .iter()
            .filter(|(spot, bounds)| spot.place == *place && bounds.center().x < x)
            .count()
    }
}

/// How far `x` is from `bounds`, sideways.
fn distance(bounds: &Bounds<Pixels>, x: Pixels) -> f32 {
    let (left, right, x) = (
        f32::from(bounds.left()),
        f32::from(bounds.right()),
        f32::from(x),
    );
    (left - x).max(x - right).max(0.)
}

/// Whether `x` is in the middle half of `bounds`.
fn middle(bounds: &Bounds<Pixels>, x: Pixels) -> bool {
    let offset = f32::from(x - bounds.center().x).abs();
    offset <= f32::from(bounds.size.width) / 4.
}

/// Makes the whole preview take every drop of a dragged item, where `drops` says it lands, and
/// follow the pointer while one is held over it. A drop that changes nothing ends the drag all
/// the same, so no drop over the preview is refused.
pub(crate) fn target<E: InteractiveElement>(
    element: E,
    drops: Entity<Drops>,
    commands: Commands,
) -> E {
    let held = drops.clone();
    element
        .on_drag_move::<Dragged>(move |event, _, cx| {
            let at = event.event.position;
            let over = event.bounds.contains(&at).then_some(at);
            let dragged = event.drag(cx).clone();
            held.update(cx, |drops, cx| notify(drops.aim(over, &dragged), cx));
        })
        .on_drop(move |dragged: &Dragged, window, cx| {
            let at = window.mouse_position();
            let landing = drops.update(cx, |drops, cx| {
                let landing = drops.landing(at, dragged);
                notify(drops.settle(), cx);
                landing
            });
            let Some(landing) = landing else {
                return;
            };
            let dragged = dragged.clone();
            commands.run(window, cx, move |section, _, cx| {
                section.dropped(&dragged, &landing, cx);
            });
        })
}

fn notify(changed: bool, cx: &mut Context<Drops>) {
    if changed {
        cx.notify();
    }
}

#[cfg(test)]
mod tests {
    use gpui_kit::{point, px, size};

    use super::*;

    const LEFT: Slot = Slot::Zone(Zone::Left);

    fn dragged(kind: Kind, id: &'static str) -> Dragged {
        let subject = Subject::Contribution(ContributionId::new(id));
        Dragged::new(subject, kind, false, None)
    }

    fn widget() -> Dragged {
        dragged(Kind::Widget, "a.widget")
    }

    fn panel() -> Dragged {
        dragged(Kind::Panel, "a.panel")
    }

    fn span(left: f32, width: f32) -> Bounds<Pixels> {
        Bounds::new(point(px(left), px(0.)), size(px(width), px(28.)))
    }

    fn at(x: f32) -> Point<Pixels> {
        point(px(x), px(14.))
    }

    /// A left zone from 0 to 200 with a widget at 10..40, a group "g" at 50..130 holding two
    /// panels at 56..86 and 92..122, and a panel "p.alone" at 140..180; a right zone from 400
    /// to 500 with a widget at 410..440.
    fn drops() -> Drops {
        let mut drops = Drops::default();
        let group = || Slot::Group(GroupId::new("g"));
        drops.zone(Zone::Left, span(0., 200.));
        drops.zone(Zone::Right, span(400., 100.));
        drops.spot(Spot::new(LEFT, 0, Holds::Item), span(10., 30.));
        let pill = Holds::Group(GroupId::new("g"));
        drops.spot(Spot::new(LEFT, 1, pill), span(50., 80.));
        drops.spot(Spot::new(group(), 0, Holds::Item), span(56., 30.));
        drops.spot(Spot::new(group(), 1, Holds::Item), span(92., 30.));
        let alone = Holds::Alone(ContributionId::new("p.alone"));
        drops.spot(Spot::new(LEFT, 2, alone), span(140., 40.));
        let right = Slot::Zone(Zone::Right);
        drops.spot(Spot::new(right, 0, Holds::Item), span(410., 30.));
        drops
    }

    #[test]
    fn the_half_of_an_entry_under_the_pointer_puts_the_drop_in_front_of_it_or_after_it() {
        let drops = drops();

        assert_eq!(
            drops.landing(at(15.), &widget()),
            Some(Landing::At(LEFT, 0))
        );
        assert_eq!(
            drops.landing(at(35.), &widget()),
            Some(Landing::At(LEFT, 1))
        );
        assert_eq!(
            drops.landing(at(45.), &widget()),
            Some(Landing::At(LEFT, 1))
        );
    }

    #[test]
    fn a_panel_over_a_group_lands_among_its_members_from_its_start_to_its_end() {
        let drops = drops();
        let group = |index| Some(Landing::At(Slot::Group(GroupId::new("g")), index));

        assert_eq!(drops.landing(at(52.), &panel()), group(0), "the start edge");
        assert_eq!(drops.landing(at(60.), &panel()), group(0));
        assert_eq!(drops.landing(at(80.), &panel()), group(1));
        assert_eq!(drops.landing(at(118.), &panel()), group(2), "the end");
        assert_eq!(drops.landing(at(128.), &panel()), group(2), "the end edge");
    }

    #[test]
    fn a_widget_over_a_group_lands_beside_the_group() {
        let drops = drops();

        assert_eq!(
            drops.landing(at(60.), &widget()),
            Some(Landing::At(LEFT, 1))
        );
        assert_eq!(
            drops.landing(at(118.), &widget()),
            Some(Landing::At(LEFT, 2))
        );
        let group = dragged(Kind::Group, "unused");
        assert_eq!(drops.landing(at(118.), &group), Some(Landing::At(LEFT, 2)));
    }

    #[test]
    fn a_panel_over_the_middle_of_a_panel_on_its_own_groups_with_it_and_its_edges_go_beside() {
        let drops = drops();
        let with = Some(Landing::With(ContributionId::new("p.alone")));

        assert_eq!(drops.landing(at(160.), &panel()), with);
        assert_eq!(
            drops.landing(at(142.), &panel()),
            Some(Landing::At(LEFT, 2))
        );
        assert_eq!(
            drops.landing(at(178.), &panel()),
            Some(Landing::At(LEFT, 3))
        );
        assert_eq!(
            drops.landing(at(160.), &widget()),
            Some(Landing::At(LEFT, 2))
        );
    }

    #[test]
    fn away_from_every_entry_the_nearest_zone_takes_the_drop() {
        let drops = drops();
        let right = |index| Some(Landing::At(Slot::Zone(Zone::Right), index));

        assert_eq!(
            drops.landing(at(195.), &widget()),
            Some(Landing::At(LEFT, 3))
        );
        assert_eq!(
            drops.landing(at(250.), &widget()),
            Some(Landing::At(LEFT, 3))
        );
        assert_eq!(drops.landing(at(380.), &widget()), right(0));
        assert_eq!(drops.landing(at(560.), &widget()), right(1));
        assert_eq!(Drops::default().landing(at(10.), &widget()), None);
    }

    #[test]
    fn the_aim_shows_only_a_drop_that_changes_something_and_ends_with_the_drag() {
        let mut drops = drops();
        let placed = Subject::Contribution(ContributionId::new("a.widget"));
        let held = Dragged::new(placed.clone(), Kind::Widget, false, Some((LEFT, 0)));

        assert!(drops.aim(Some(at(15.)), &held), "the drag starts");
        assert_eq!(drops.aimed(), None, "in front of itself changes nothing");
        assert_eq!(drops.moving(), Some(&placed));
        assert!(drops.aim(Some(at(160.)), &held));
        assert_eq!(drops.aimed(), Some(&Landing::At(LEFT, 2)));
        assert!(!drops.aim(Some(at(158.)), &held), "nothing new to show");
        assert!(drops.aim(None, &held), "away from the preview");
        assert_eq!(drops.aimed(), None);
        assert!(drops.settle());
        assert_eq!(drops.moving(), None);
    }
}
