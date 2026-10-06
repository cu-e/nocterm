use super::*;
fn server(id: &str, name: &str, group: Option<&str>) -> ConnectionSummary {
    ConnectionSummary {
        id: id.to_owned().into(),
        name: name.to_owned().into(),
        group: group.map(|group| group.to_owned().into()),
        description: String::new().into(),
        target: nocterm_session::Target::new("me", "host.test", 22),
        icon: None,
        flag: None,
    }
}
#[test]
fn projection_preserves_empty_folders_and_searches_hosts_and_groups() {
    let servers = vec![
        server("one", "Production", Some("Work")),
        server("two", "Personal", None),
    ];
    let groups = vec![SharedString::from("Work"), SharedString::from("Empty")];
    let folders = project(&servers, &groups, "");
    assert_eq!(folders.len(), 3);
    assert!(folders[0].servers.is_empty());
    let folders = project(&servers, &groups, "work");
    assert_eq!(folders.len(), 1);
    assert_eq!(folders[0].servers[0].id.as_ref(), "one");
    assert_eq!(
        project(&servers, &groups, "host.test")
            .iter()
            .map(|folder| folder.servers.len())
            .sum::<usize>(),
        2
    );
    assert!(project(&servers, &groups, "absent").is_empty());
}

#[test]
fn a_group_named_ungrouped_has_a_distinct_folder_identity() {
    let servers = vec![
        server("one", "Grouped", Some("ungrouped")),
        server("two", "Unassigned", None),
    ];
    let folders = project(&servers, &[], "");
    assert_eq!(folders.len(), 2);
    assert_ne!(
        folder_id(folders[0].name.as_deref()),
        folder_id(folders[1].name.as_deref())
    );
}
