use crate::support::*;
use nocterm_project_model::*;
use nocterm_session::Target;

#[test]
fn renaming_and_changing_a_root_preserve_supplied_project_identity() {
    let original = definition(1);
    let mut draft = original.to_draft();
    draft.name = "Renamed".into();
    draft.root = RootPath::parse("/srv/renamed").unwrap();
    let renamed = ProjectDefinition::try_from(draft).unwrap();
    assert_eq!(original.id(), renamed.id());
    assert_eq!(
        ContextId::Project(original.id()),
        ContextId::Project(renamed.id())
    );
}

#[test]
fn equal_names_hosts_and_paths_do_not_merge_supplied_ids() {
    let first = definition(1);
    let second = definition(2);
    assert_eq!(first.name(), second.name());
    assert_eq!(first.root(), second.root());
    assert_ne!(first.id(), second.id());
}

#[test]
fn ids_round_trip_without_allocating_new_identity() {
    let id = project(0xab);
    assert_eq!(id.to_string(), "ab".repeat(16));
    assert_eq!(id.to_string().parse::<ProjectId>().unwrap(), id);
    assert_eq!(id.to_bytes(), [0xab; 16]);
    assert!(matches!(
        "not-an-id".parse::<ProjectId>(),
        Err(ProjectIdParseError::InvalidShape)
    ));
}

#[test]
fn context_wire_shape_is_explicit_and_unknown_contexts_are_refused() {
    assert_eq!(
        serde_json::to_string(&ContextId::General).unwrap(),
        r#"{"kind":"general"}"#
    );
    let value = serde_json::to_value(ContextId::Project(project(1))).unwrap();
    assert_eq!(value["kind"], "project");
    assert_eq!(value["id"], "01".repeat(16));
    for bad in [
        r#"{"kind":"future"}"#,
        r#"{"kind":"project"}"#,
        r#"{"kind":"general","extra":true}"#,
        r#"{"kind":"general","id":"01010101010101010101010101010101"}"#,
    ] {
        assert!(serde_json::from_str::<ContextId>(bad).is_err());
    }
}

#[test]
fn different_jump_routes_remain_different_hosts() {
    let host = |via| HostLocator::Ssh {
        profile: "profile-one".into(),
        target: Target::new("user", "server", 22),
        via: vec![Target::new("user", via, 22)],
    };
    assert_ne!(host("bastion-a"), host("bastion-b"));
    host("bastion-a").validate().unwrap();
}

#[test]
fn roots_reject_ambiguous_paths_and_preserve_literal_remote_names() {
    for value in ["", "relative", "/a/../b", "/a/./b", "/a//b", "/a/", "/a\0b"] {
        assert!(matches!(
            RootPath::parse(value),
            Err(ProjectValidationError::InvalidRoot)
        ));
    }
    let root = RootPath::parse("/srv/проект $(literal)\\name").unwrap();
    assert_eq!(root.as_str(), "/srv/проект $(literal)\\name");
}

#[test]
fn ordinary_folder_roots_include_home_account_and_filesystem_roots() {
    for path in ["/", "/home", "/root", "/home/user", "/home/user/.ssh"] {
        assert_eq!(RootPath::parse(path).unwrap().as_str(), path);
    }
}

#[test]
fn importing_another_project_with_the_same_name_keeps_old_ownership_distinct() {
    let old_scope = ContextId::Project(definition(1).id());
    let mut catalog = Catalog::new(vec![definition(1)]).unwrap();
    catalog.remove(project(1)).unwrap();
    catalog.insert(definition(2)).unwrap();
    assert_eq!(old_scope, ContextId::Project(project(1)));
    assert!(catalog.get(project(1)).is_none());
    assert_ne!(catalog.get(project(2)).unwrap().id(), project(1));
}

#[test]
fn invalid_names_host_routes_and_import_sources_keep_typed_validation_reasons() {
    let mut draft = definition(1).to_draft();
    draft.name = " ".into();
    assert_eq!(
        ProjectDefinition::try_from(draft),
        Err(ProjectValidationError::InvalidName)
    );
    let mut draft = definition(1).to_draft();
    draft.host = HostLocator::Ssh {
        profile: "p".into(),
        target: Target::new("user", "host", 0),
        via: vec![],
    };
    assert_eq!(
        ProjectDefinition::try_from(draft),
        Err(ProjectValidationError::InvalidHost)
    );
    let mut draft = definition(1).to_draft();
    draft.source = ProjectSource::Imported {
        provider: "coder".into(),
        deployment: "deployment-one".into(),
        account: "account-one".into(),
        external_id: "".into(),
    };
    assert_eq!(
        ProjectDefinition::try_from(draft),
        Err(ProjectValidationError::InvalidSource)
    );
}

#[test]
fn unknown_source_and_host_fields_are_refused_even_when_other_values_are_valid() {
    let mut value = serde_json::to_value(definition(1).to_draft()).unwrap();
    value["source"] = serde_json::json!({"kind":"imported", "provider":"coder", "deployment":"deployment-one", "account":"account-one", "external_id":"one", "unexpected":true});
    assert!(serde_json::from_value::<ProjectDraft>(value).is_err());
    let mut value = serde_json::to_value(definition(1).to_draft()).unwrap();
    value["host"] = serde_json::json!({"kind":"ssh", "profile":"one", "target":{"host":"server","user":"u","port":22,"unexpected":true},"via":[]});
    assert!(serde_json::from_value::<ProjectDraft>(value).is_err());
}

#[test]
fn imported_workspace_identity_includes_deployment_and_account_without_replacing_project_id() {
    let mut first = definition(1).to_draft();
    first.source = ProjectSource::Imported {
        provider: "coder".into(),
        deployment: "deployment-one".into(),
        account: "account-one".into(),
        external_id: "same-external-id".into(),
    };
    let first = ProjectDefinition::try_from(first).unwrap();
    for (next_deployment, next_account) in [
        ("deployment-two", "account-one"),
        ("deployment-one", "account-two"),
    ] {
        let mut other = first.to_draft();
        if let ProjectSource::Imported {
            deployment,
            account,
            ..
        } = &mut other.source
        {
            *deployment = next_deployment.into();
            *account = next_account.into();
        }
        let other = ProjectDefinition::try_from(other).unwrap();
        assert_ne!(first.source(), other.source());
        assert_eq!(first.id(), other.id());
    }
    for field in ["provider", "deployment", "account", "external_id"] {
        let mut wire = serde_json::to_value(first.to_draft()).unwrap();
        wire["source"].as_object_mut().unwrap().remove(field);
        assert!(serde_json::from_value::<ProjectDraft>(wire).is_err());
    }
}

#[test]
fn quick_ssh_parent_is_explicit_and_never_uses_an_empty_saved_profile() {
    let direct = HostLocator::SshDirect {
        target: Target::new("ops", "server", 2222),
        via: vec![Target::new("jump", "bastion", 22)],
    };
    direct.validate().unwrap();
    let wire = serde_json::to_value(&direct).unwrap();
    assert_eq!(wire["kind"], "ssh_direct");
    assert!(wire.get("profile").is_none());
    assert_eq!(serde_json::from_value::<HostLocator>(wire).unwrap(), direct);
    let empty_profile = HostLocator::Ssh {
        profile: String::new(),
        target: Target::new("ops", "server", 2222),
        via: vec![],
    };
    assert!(matches!(
        empty_profile.parent_key(),
        Err(ProjectValidationError::InvalidHost)
    ));
}

#[test]
fn grouping_uses_saved_profile_identity_while_exact_target_facts_stay_distinct() {
    let first = HostLocator::Ssh {
        profile: "saved-one".into(),
        target: Target::new("ops", "old", 22),
        via: vec![],
    };
    let edited = HostLocator::Ssh {
        profile: "saved-one".into(),
        target: Target::new("ops", "new", 2222),
        via: vec![],
    };
    let same_host_other_profile = HostLocator::Ssh {
        profile: "saved-two".into(),
        target: Target::new("ops", "old", 22),
        via: vec![],
    };
    assert_eq!(first.parent_key().unwrap(), edited.parent_key().unwrap());
    assert_ne!(first, edited);
    assert_ne!(
        first.parent_key().unwrap(),
        same_host_other_profile.parent_key().unwrap()
    );
}

#[test]
fn quick_ssh_grouping_distinguishes_every_route_and_explicit_local() {
    let direct = |via: &str| HostLocator::SshDirect {
        target: Target::new("ops", "localhost", 22),
        via: vec![Target::new("jump", via, 22)],
    };
    assert_ne!(
        direct("a").parent_key().unwrap(),
        direct("b").parent_key().unwrap()
    );
    assert_ne!(
        direct("a").parent_key().unwrap(),
        HostLocator::Local.parent_key().unwrap()
    );
    for value in [
        serde_json::json!({"kind":"ssh_direct","profile":"fake","target":{"host":"h","user":"u","port":22},"via":[]}),
        serde_json::json!({"kind":"ssh_direct","target":{"host":"h","user":"u","port":22,"trusted":true},"via":[]}),
    ] {
        assert!(serde_json::from_value::<HostLocator>(value).is_err());
    }
}
