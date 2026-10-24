//! Which entries of a narrow bar go into the overflow menu.
//!
//! Pure arithmetic over widths: the planner hides the entries that matter least, one at a time,
//! until the rest fits the room.

use gpui_kit::Pixels;

/// What the planner may do with one entry.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Kind {
    /// A divider, which stays only between two entries that stay.
    Separator,
    /// Never hidden: the pinned notice line, an item that asks for the user, and an item whose
    /// size is not known yet.
    Fixed,
    /// May go into the menu. A lower rank goes first; among equal ranks the one later in the bar
    /// goes first.
    Hideable(u8),
}

/// One entry of a zone, as wide as it was measured.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct Slot {
    pub(super) width: Pixels,
    pub(super) kind: Kind,
}

/// How a row lays out its children.
#[derive(Clone, Copy, Debug)]
pub(super) struct Row {
    /// The space between two neighbouring children.
    pub(super) gap: Pixels,
    /// Children besides the entries and the button, such as the spacer of a flat bar.
    pub(super) spare: usize,
}

/// The entries of `zones` to hide, per zone, until `over` finds no zone left over.
///
/// `over` is given what is hidden so far and answers with the zones that hiding one more of
/// their entries would help: the ones that are too wide, and any zone whose entries give room to
/// them. The entry that goes next is the one of the lowest rank among those zones. Fixed entries
/// stay even if they do not fit; the zone then clips them.
pub(super) fn plan(zones: &[&[Slot]], over: impl Fn(&[Vec<bool>]) -> Vec<usize>) -> Vec<Vec<bool>> {
    let mut hidden: Vec<Vec<bool>> = zones.iter().map(|zone| vec![false; zone.len()]).collect();
    loop {
        let candidates = over(&hidden).into_iter().flat_map(|z| {
            let hidden = &hidden;
            (0..zones[z].len())
                .filter(move |&ix| !hidden[z][ix])
                .filter_map(move |ix| match zones[z][ix].kind {
                    Kind::Hideable(rank) => Some((rank, std::cmp::Reverse((z, ix)))),
                    _ => None,
                })
        });
        let Some((_, std::cmp::Reverse((z, ix)))) = candidates.min() else {
            return hidden;
        };
        hidden[z][ix] = true;
    }
}

/// The width the visible entries of `zones`, the spare children and the `button` take.
pub(super) fn footprint(
    zones: &[&[Slot]],
    hidden: &[Vec<bool>],
    row: Row,
    button: Option<Pixels>,
) -> Pixels {
    let mut children = row.spare;
    let mut width = Pixels::ZERO;
    for (zone, hidden) in zones.iter().zip(hidden) {
        for slot in visible(zone, hidden) {
            children += 1;
            width += slot.width;
        }
    }
    if let Some(button) = button {
        children += 1;
        width += button;
    }
    width + row.gap * children.saturating_sub(1) as f32
}

/// What a zone keeps, with the separators that no longer stand between two entries removed.
fn visible<'a>(zone: &'a [Slot], hidden: &'a [bool]) -> Vec<&'a Slot> {
    let mut kept: Vec<&Slot> = Vec::with_capacity(zone.len());
    let is_separator = |slot: &Slot| slot.kind == Kind::Separator;
    for (slot, _) in zone.iter().zip(hidden).filter(|(_, hidden)| !**hidden) {
        if !(is_separator(slot) && kept.last().is_none_or(|last| is_separator(last))) {
            kept.push(slot);
        }
    }
    while kept.last().is_some_and(|last| is_separator(last)) {
        kept.pop();
    }
    kept
}

#[cfg(test)]
mod tests;
