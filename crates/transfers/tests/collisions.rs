//! Pure collision, hostile-name and stream accounting rules.
use nocterm_transfers::*;
#[test]
fn collision_choices_are_explicit_and_reservations_serialize_replace() {
    assert_eq!(
        collision_decision(CollisionPolicy::Skip, false, true),
        CollisionDecision::Skip
    );
    assert_eq!(
        collision_decision(CollisionPolicy::Skip, true, false),
        CollisionDecision::Skip
    );
    assert_eq!(
        collision_decision(CollisionPolicy::Rename, false, true),
        CollisionDecision::Rename
    );
    assert_eq!(
        collision_decision(CollisionPolicy::Replace, true, false),
        CollisionDecision::Use
    );
    assert_eq!(
        collision_decision(CollisionPolicy::Replace, false, true),
        CollisionDecision::Wait
    );
}
#[test]
fn rename_preserves_extensions_hidden_names_and_parent() {
    assert_eq!(renamed("/a/archive.tar", 1), "/a/archive (1).tar");
    assert_eq!(renamed("/a/.hidden", 2), "/a/.hidden (2)");
    assert_eq!(renamed("/a/no-extension", 3), "/a/no-extension (3)");
}
