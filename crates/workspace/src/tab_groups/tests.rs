use super::*;

const PALETTE: usize = 6;

/// Applies `moves` the way the dock does: out of its place, then in.
fn apply(bars: &mut Bars<char, u8>, moves: &[Move<char, u8>]) {
    for step in moves {
        if let Some((bar, ix)) = locate(bars, step.panel) {
            bars[bar].1.remove(ix);
        }
        let bar = bars
            .iter()
            .position(|(node, _)| *node == step.node)
            .unwrap();
        bars[bar].1.insert(step.ix, step.panel);
    }
}

fn bars(layout: &[(u8, &str)]) -> Bars<char, u8> {
    layout
        .iter()
        .map(|(node, panels)| (*node, panels.chars().collect()))
        .collect()
}

fn shown(bars: &Bars<char, u8>) -> Vec<String> {
    bars.iter()
        .map(|(_, panels)| panels.iter().collect())
        .collect()
}

/// Groups `members` and gathers them once in `layout`, as the workspace
/// does after grouping, so later layouts are compared against it.
fn grouped(layout: &[(u8, &str)], members: &str) -> (TabGroups<char, u8>, Bars<char, u8>) {
    let mut groups = TabGroups::default();
    let mut members = members.chars();
    let group = groups.create(members.next().unwrap(), PALETTE);
    for member in members {
        groups.join(member, group);
    }
    let mut layout = bars(layout);
    let moves = groups.gather(&layout);
    apply(&mut layout, &moves);
    (groups, layout)
}

/// What the user did, then what gathering makes of it.
fn after(groups: &mut TabGroups<char, u8>, layout: &[(u8, &str)]) -> Vec<String> {
    let mut layout = bars(layout);
    let moves = groups.gather(&layout);
    apply(&mut layout, &moves);
    assert!(
        groups.gather(&layout).is_empty(),
        "a gathered layout is left alone"
    );
    shown(&layout)
}

#[test]
fn joining_brings_the_tab_to_its_group() {
    let (_, layout) = grouped(&[(1, "aXbcY")], "aY");
    assert_eq!(shown(&layout), ["aYXbc"]);
    let (_, layout) = grouped(&[(1, "XYa"), (2, "b")], "ab");
    assert_eq!(shown(&layout), ["XYab", ""]);
}

#[test]
fn dragging_any_member_drags_the_group() {
    let (mut groups, _) = grouped(&[(1, "abcXY")], "abc");
    // `c` dragged to the end of the bar.
    assert_eq!(after(&mut groups, &[(1, "abXYc")]), ["XYabc"]);
    // `a` dragged back to the front.
    assert_eq!(after(&mut groups, &[(1, "aXYbc")]), ["abcXY"]);
}

#[test]
fn moving_a_member_to_another_bar_takes_the_group_along() {
    let (mut groups, _) = grouped(&[(1, "XabY"), (2, "Z")], "ab");
    assert_eq!(
        after(&mut groups, &[(1, "XaY"), (2, "Zb")]),
        ["XY", "Zab"],
        "the group keeps its order"
    );
}

#[test]
fn a_tab_dropped_inside_a_group_is_moved_out_of_it() {
    let (mut groups, _) = grouped(&[(1, "abcX")], "abc");
    let layout = after(&mut groups, &[(1, "abXc")]);
    assert_eq!(layout, ["Xabc"]);
}

#[test]
fn reordering_inside_a_group_keeps_it_together() {
    let (mut groups, _) = grouped(&[(1, "abcX")], "abc");
    assert_eq!(after(&mut groups, &[(1, "cabX")]), ["cabX"]);
    assert_eq!(
        after(&mut groups, &[(1, "XcabY")]),
        ["XcabY"],
        "the new order stands"
    );
}

#[test]
fn a_group_gathered_into_another_pushes_that_one_aside() {
    let mut groups = TabGroups::default();
    let first = groups.create('a', PALETTE);
    groups.join('b', first);
    let second = groups.create('x', PALETTE);
    groups.join('y', second);
    let mut layout = bars(&[(1, "abxy")]);
    let moves = groups.gather(&layout);
    apply(&mut layout, &moves);
    // `b` dragged between `x` and `y`.
    let result = after(&mut groups, &[(1, "axby")]);
    let joined = result.concat();
    for pair in ["ab", "xy"] {
        assert!(joined.contains(pair), "{pair} together in {joined}");
    }
}

#[test]
fn closing_down_to_one_tab_ends_the_group() {
    let (mut groups, _) = grouped(&[(1, "abc")], "ab");
    let group = groups.group_of('a').unwrap();
    groups.gather(&bars(&[(1, "ac")]));
    assert_eq!(groups.group_of('a'), None);
    assert_eq!(groups.color(group), None);
}

#[test]
fn leaving_and_joining_move_a_tab_between_groups() {
    let (mut groups, _) = grouped(&[(1, "abc")], "abc");
    let group = groups.group_of('a').unwrap();
    groups.leave('c');
    assert_eq!(groups.group_of('c'), None);
    assert_eq!(groups.members(group).count(), 2);

    let other = groups.create('c', PALETTE);
    groups.join('b', other);
    assert_eq!(
        groups.group_of('a'),
        None,
        "the group left with one tab ends"
    );
    assert_eq!(groups.group_of('b'), Some(other));
}

#[test]
fn each_group_gets_a_color_no_other_has() {
    let mut groups: TabGroups<char, u8> = TabGroups::default();
    let first = groups.create('a', 2);
    let second = groups.create('b', 2);
    assert_ne!(groups.color(first), groups.color(second));
    groups.dissolve(first);
    let third = groups.create('c', 2);
    assert_eq!(groups.color(third), Some(0), "a freed color is reused");
}
