use super::*;
fn snippet() -> Snippet {
    Snippet {
        name: "Deploy".into(),
        content: "echo привет\nls -la\n".into(),
        ..Default::default()
    }
}
#[test]
fn exact_profile_or_group_matches_and_missing_bindings_stay_scoped() {
    let mut s = snippet();
    s.profiles.push("profile-1".into());
    s.groups.push("Work".into());
    assert!(s.matches(Some("profile-1"), None));
    assert!(s.matches(Some("profile-2"), Some("Work")));
    assert!(!s.matches(Some("profile-2"), Some("work")));
    assert!(!s.matches(None, Some("Work")));
    assert!(!s.matches(Some("missing"), None));
    assert!(!snippet().matches(Some("profile-1"), None));
}
#[test]
fn stale_edits_and_deletes_do_not_overwrite_current_content() {
    let original = snippet();
    let mut library = Library::default();
    library.save(original.clone(), None).unwrap();
    let mut changed = original.clone();
    changed.content = "updated".into();
    library.save(changed.clone(), Some(&original)).unwrap();
    assert!(library.save(original.clone(), Some(&original)).is_err());
    assert!(library.save(original.clone(), None).is_err());
    assert!(library.delete(&original).is_err());
    assert_eq!(library.snippets, [changed.clone()]);
    library.delete(&changed).unwrap();
    assert!(library.save(original.clone(), Some(&original)).is_err());
}
#[test]
fn validation_preserves_multiline_content_and_rejects_empty_fields() {
    let mut s = snippet();
    let content = s.content.clone();
    s.validate().unwrap();
    assert_eq!(content, s.content);
    s.name = "  ".into();
    assert_eq!(s.validate(), Err(ValidationError::Name));
    s.name = "OK".into();
    s.content = "\n  ".into();
    assert_eq!(s.validate(), Err(ValidationError::Content));
    s.content = "x".repeat(256 * 1024);
    assert_eq!(s.validate(), Err(ValidationError::Size));
}
#[test]
fn partition_filters_and_orders_context_before_other_snippets() {
    let mut bound = snippet();
    bound.profiles.push("current".into());
    let unbound = snippet();
    let library = Library {
        snippets: vec![unbound.clone(), bound.clone()],
    };
    let (current, other) = library.partition(Some("current"), None, "ПРИВЕТ");
    assert_eq!(current, [&bound]);
    assert_eq!(other, [&unbound]);
    assert!(library.partition(None, None, "absent").1.is_empty());
}
