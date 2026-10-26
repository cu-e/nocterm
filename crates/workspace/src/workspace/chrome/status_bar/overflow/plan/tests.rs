use gpui_kit::px;

use super::*;

const GAP: f32 = 4.;
const BUTTON: f32 = 24.;

fn slot(width: f32, kind: Kind) -> Slot {
    Slot {
        width: px(width),
        kind,
    }
}

fn row(spare: usize) -> Row {
    Row {
        gap: px(GAP),
        spare,
    }
}

fn any(hidden: &[Vec<bool>]) -> bool {
    hidden.iter().flatten().any(|hidden| *hidden)
}

/// Plans `zones` as one row of `budget` that draws the button once something is hidden.
fn one_row(zones: &[&[Slot]], budget: f32, spare: usize) -> Vec<Vec<bool>> {
    plan(zones, |hidden| {
        let button = any(hidden).then_some(px(BUTTON));
        if footprint(zones, hidden, row(spare), button) > px(budget) {
            (0..zones.len()).collect()
        } else {
            Vec::new()
        }
    })
}

#[test]
fn entries_that_fit_are_all_kept() {
    let zone = [slot(24., Kind::Hideable(0)), slot(24., Kind::Hideable(2))];
    // Two entries and the gap between them.
    assert_eq!(one_row(&[&zone], 52., 0), [[false, false]]);
}

#[test]
fn a_bar_that_does_not_fit_hides_the_lowest_rank_first() {
    let zone = [
        slot(24., Kind::Hideable(2)),
        slot(60., Kind::Hideable(0)),
        slot(24., Kind::Hideable(1)),
    ];
    // The widget goes; the rest and the button fit.
    assert_eq!(one_row(&[&zone], 110., 0), [[false, true, false]]);
}

#[test]
fn among_equal_ranks_the_later_entry_goes_first() {
    let zone = [
        slot(40., Kind::Hideable(0)),
        slot(40., Kind::Hideable(0)),
        slot(40., Kind::Hideable(0)),
    ];
    assert_eq!(one_row(&[&zone], 112., 0), [[false, false, true]]);
    assert_eq!(one_row(&[&zone], 80., 0), [[false, true, true]]);
}

#[test]
fn later_zones_give_way_before_earlier_ones() {
    let left = [slot(40., Kind::Hideable(0))];
    let right = [slot(40., Kind::Hideable(0))];
    assert_eq!(one_row(&[&left, &right], 80., 1), [[false], [true]]);
}

#[test]
fn fixed_entries_stay_even_when_nothing_else_is_left() {
    let zone = [
        slot(300., Kind::Fixed),
        slot(24., Kind::Hideable(1)),
        slot(24., Kind::Hideable(0)),
    ];
    assert_eq!(one_row(&[&zone], 100., 0), [[false, true, true]]);
}

#[test]
fn the_button_is_counted_only_when_something_is_hidden() {
    let zone = [slot(50., Kind::Hideable(0)), slot(50., Kind::Fixed)];
    // 104 is exactly the two entries; no button is needed.
    assert_eq!(one_row(&[&zone], 104., 0), [[false, false]]);
    // One unit less hides the first, and the button takes its place in the count.
    assert_eq!(one_row(&[&zone], 103., 0), [[true, false]]);
}

#[test]
fn a_separator_beside_a_hidden_entry_no_longer_costs_room() {
    let zone = [
        slot(40., Kind::Hideable(1)),
        slot(1., Kind::Separator),
        slot(40., Kind::Hideable(0)),
    ];
    // All three: 40 + 1 + 40 + two gaps = 89. Without the last entry the separator is trailing
    // and drops out: 40 plus the button and a gap = 68.
    let hidden = one_row(&[&zone], 70., 0);
    assert_eq!(hidden, [[false, false, true]]);
    let kept = footprint(&[&zone], &hidden, row(0), Some(px(BUTTON)));
    assert_eq!(kept, px(68.));
}

#[test]
fn spare_children_take_a_gap_each() {
    let zone = [slot(40., Kind::Hideable(0))];
    // With a spacer the entry is the second child: 40 + 4.
    assert_eq!(one_row(&[&zone], 44., 1), [[false]]);
    assert_eq!(one_row(&[&zone], 43., 1), [[true]]);
}

#[test]
fn only_the_zones_that_would_help_give_up_entries() {
    let left = [slot(40., Kind::Hideable(2))];
    let right = [slot(40., Kind::Hideable(2)), slot(40., Kind::Hideable(3))];
    // Only the right zone is too wide: the left toggle keeps its place although its rank is
    // not higher than the right one's.
    let hidden = plan(&[&left, &right], |hidden| {
        let right_width = footprint(&[&right], &hidden[1..2], row(0), None);
        if right_width > px(50.) {
            vec![1]
        } else {
            Vec::new()
        }
    });
    assert_eq!(hidden, [vec![false], vec![true, false]]);
}

#[test]
fn an_empty_bar_plans_nothing() {
    let empty: [Slot; 0] = [];
    assert_eq!(one_row(&[&empty], 0., 0), [Vec::<bool>::new()]);
}
