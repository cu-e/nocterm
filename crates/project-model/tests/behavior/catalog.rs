use crate::support::*;
use nocterm_project_model::*;

#[test]
fn checked_catalog_round_trips_folder_definitions() {
    let catalog = Catalog::new(vec![definition(1), definition(2)]).unwrap();
    let bytes = catalog.encode().unwrap();
    let decoded = Catalog::decode(&bytes).unwrap();
    assert_eq!(decoded.get(project(1)), catalog.get(project(1)));
    assert_eq!(decoded.len(), 2);
    let value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    let fields = value["projects"][0].as_object().unwrap();
    assert_eq!(fields.len(), 6);
    for field in ["id", "name", "host", "root", "source", "updated"] {
        assert!(fields.contains_key(field), "missing {field}");
    }
}

#[test]
fn unsupported_and_unknown_catalog_data_are_never_treated_as_empty_or_general() {
    assert!(matches!(
        Catalog::decode(br#"{"version":2,"projects":[]}"#),
        Err(CatalogError::UnsupportedVersion(2))
    ));
    assert!(matches!(
        Catalog::decode(br#"{"version":1,"projects":[],"unexpected":true}"#),
        Err(CatalogError::Codec(_))
    ));
    assert!(matches!(
        Catalog::decode(br#"{"projects":[]}"#),
        Err(CatalogError::Codec(_))
    ));
}

#[test]
fn legacy_catalog_metadata_preserves_ids_and_is_omitted_from_new_encoding() {
    let mut value = serde_json::json!({"version":1,"projects":[definition(1)]});
    value["projects"][0]["policy"] = serde_json::json!({"version":99,"obsolete":true});
    let decoded = Catalog::decode(&serde_json::to_vec(&value).unwrap()).unwrap();
    assert_eq!(decoded.get(project(1)).unwrap(), &definition(1));
    assert_eq!(
        ContextId::Project(decoded.get(project(1)).unwrap().id()),
        ContextId::Project(project(1))
    );
    let encoded: serde_json::Value = serde_json::from_slice(&decoded.encode().unwrap()).unwrap();
    assert!(encoded["projects"][0].get("policy").is_none());
}

#[test]
fn duplicate_ids_and_oversized_documents_are_rejected_before_publication() {
    assert!(matches!(
        Catalog::new(vec![definition(1), definition(1)]),
        Err(CatalogError::DuplicateId(_))
    ));
    assert!(matches!(
        Catalog::decode(&vec![b' '; MAX_CATALOG_BYTES + 1]),
        Err(CatalogError::TooLarge)
    ));
    let projects = (0..=MAX_PROJECTS)
        .map(|n| {
            let mut draft = definition(1).to_draft();
            let mut bytes = [0; 16];
            bytes[..8].copy_from_slice(&(n as u64).to_be_bytes());
            draft.id = ProjectId::from_bytes(bytes);
            ProjectDefinition::try_from(draft).unwrap()
        })
        .collect();
    assert!(matches!(
        Catalog::new(projects),
        Err(CatalogError::Capacity)
    ));
}

#[test]
fn failed_mutations_leave_the_existing_catalog_intact() {
    let mut catalog = Catalog::new(vec![definition(1)]).unwrap();
    assert!(matches!(
        catalog.insert(definition(1)),
        Err(CatalogError::DuplicateId(_))
    ));
    assert!(matches!(
        catalog.remove(project(2)),
        Err(CatalogError::Missing(_))
    ));
    assert_eq!(catalog.len(), 1);
    assert_eq!(catalog.get(project(1)).unwrap(), &definition(1));
}

#[test]
fn malformed_json_keeps_its_concrete_original_source() {
    use std::error::Error;
    let error = Catalog::decode(b"{").unwrap_err();
    let shared = error.source().unwrap();
    assert!(
        shared
            .source()
            .unwrap()
            .downcast_ref::<serde_json::Error>()
            .is_some()
    );
}

#[test]
fn byte_capacity_refuses_a_candidate_before_mutating_the_catalog() {
    let mut catalog = Catalog::default();
    let original = catalog.encode().unwrap();
    let long_name = "\"".repeat(250);
    let mut rejected = false;
    for value in 0..MAX_PROJECTS {
        let mut draft = definition(value as u8).to_draft();
        draft.root = RootPath::parse(format!("/srv/{}", "\"".repeat(3800))).unwrap();
        draft.source = ProjectSource::Imported {
            provider: "provider".into(),
            deployment: "deployment-one".into(),
            account: "account-one".into(),
            external_id: long_name.clone(),
        };
        let candidate = ProjectDefinition::try_from(draft).unwrap();
        let before = catalog.encode().unwrap();
        if let Err(CatalogError::TooLarge) = catalog.insert(candidate) {
            assert_eq!(catalog.encode().unwrap(), before);
            rejected = true;
            break;
        }
    }
    assert!(rejected);
    assert_ne!(catalog.encode().unwrap(), original);
}

#[test]
fn replacing_a_definition_retains_identity_and_refuses_an_absent_project() {
    let mut catalog = Catalog::new(vec![definition(1)]).unwrap();
    let mut draft = definition(1).to_draft();
    draft.name = "Renamed".into();
    catalog
        .replace(ProjectDefinition::try_from(draft).unwrap())
        .unwrap();
    assert_eq!(catalog.get(project(1)).unwrap().name(), "Renamed");
    assert!(matches!(
        catalog.replace(definition(2)),
        Err(CatalogError::Missing(_))
    ));
    assert_eq!(catalog.len(), 1);
}

#[test]
fn current_connection_workspaces_precede_all_other_saved_workspaces() {
    use nocterm_session::Target;
    let saved = |id, profile: &str, name: &str| {
        let mut draft = definition(id).to_draft();
        draft.name = name.into();
        draft.host = HostLocator::Ssh {
            profile: profile.into(),
            target: Target::new("ops", "server", 22),
            via: vec![],
        };
        ProjectDefinition::try_from(draft).unwrap()
    };
    let first = saved(1, "current", "Zulu");
    let current = first.host().parent_key().unwrap();
    let catalog = Catalog::new(vec![
        first,
        saved(2, "other", "Other"),
        saved(3, "current", "Alpha"),
    ])
    .unwrap();
    let groups = catalog.partition(Some(&current), "");
    assert_eq!(
        groups
            .current
            .iter()
            .map(|row| row.name())
            .collect::<Vec<_>>(),
        ["Alpha", "Zulu"]
    );
    assert_eq!(
        groups
            .other
            .iter()
            .map(|row| row.name())
            .collect::<Vec<_>>(),
        ["Other"]
    );
    let offline = catalog.partition(None, "");
    assert!(offline.current.is_empty());
    assert_eq!(offline.other.len(), 3);
}

#[test]
fn workspace_search_filters_both_groups_without_changing_membership_or_catalog() {
    let mut second = definition(2).to_draft();
    second.name = "Other workspace".into();
    second.host = HostLocator::SshDirect {
        target: nocterm_session::Target::new("ops", "other-server", 22),
        via: vec![],
    };
    let catalog = Catalog::new(vec![
        definition(1),
        ProjectDefinition::try_from(second).unwrap(),
    ])
    .unwrap();
    let before = catalog.encode().unwrap();
    let local = HostLocator::Local.parent_key().unwrap();
    let groups = catalog.partition(Some(&local), " OTHER-SERVER ");
    assert!(groups.current.is_empty());
    assert_eq!(groups.other.len(), 1);
    assert_eq!(groups.other[0].id(), project(2));
    assert_eq!(catalog.encode().unwrap(), before);
}
