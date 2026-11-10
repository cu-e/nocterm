use crate::support::*;
use nocterm_project_model::*;

#[test]
fn version_one_folder_catalog_migrates_without_changing_identity_or_route() {
    let wire = serde_json::json!({"version":1,"projects":[{"id":project(1),"name":"Legacy","host":{"kind":"local"},"root":"/old folder","source":{"kind":"manual"},"updated":123}]});
    let catalog = Catalog::decode(&serde_json::to_vec(&wire).unwrap()).unwrap();
    let saved = catalog.get(project(1)).unwrap();
    assert_eq!(saved.host_folder().unwrap().1.as_str(), "/old folder");
    let updated: serde_json::Value = serde_json::from_slice(&catalog.encode().unwrap()).unwrap();
    assert_eq!(updated["version"], 2);
    assert!(updated["projects"][0].get("host").is_none());
    assert_eq!(updated["projects"][0]["target"]["kind"], "host_folder");
}
#[test]
fn unknown_provider_keeps_opaque_reference_and_history_identity_through_unrelated_edits() {
    let reference = ProviderResourceRef::new(
        serde_json::json!({"endpoint":"region","resource":{"id":"opaque"},"future":[1,2]}),
    )
    .unwrap();
    let mut draft = definition(1).to_draft();
    draft.target = ProjectTarget::Provider {
        provider: ProviderId::parse("example.vm").unwrap(),
        resource_version: 99,
        reference: reference.clone(),
    };
    let record = ProjectDefinition::try_from(draft).unwrap();
    assert!(record.host_folder().is_none());
    let mut catalog = Catalog::decode(
        &Catalog::new(vec![record.clone(), definition(2)])
            .unwrap()
            .encode()
            .unwrap(),
    )
    .unwrap();
    let mut other = definition(2).to_draft();
    other.name = "Renamed".into();
    catalog.replace(other.try_into().unwrap()).unwrap();
    assert_eq!(catalog.get(project(1)).unwrap(), &record);
    assert_eq!(
        ContextId::Project(record.id()),
        ContextId::Project(project(1))
    );
    assert!(!format!("{reference:?}").contains("endpoint"));
}
#[test]
fn provider_envelope_rejects_unqualified_ids_scalars_excess_depth_and_size() {
    for id in ["", "docker", "a..b", "Example.vm", "example.vm/path"] {
        assert!(ProviderId::parse(id).is_err());
    }
    assert!(ProviderResourceRef::new(serde_json::json!([1])).is_err());
    assert!(
        ProviderResourceRef::new(serde_json::json!({"large":"a".repeat(MAX_RESOURCE_BYTES)}))
            .is_err()
    );
    let mut value = serde_json::json!({});
    for _ in 0..33 {
        value = serde_json::json!({"child":value});
    }
    assert!(ProviderResourceRef::new(value).is_err());
    let mut draft = definition(1).to_draft();
    draft.target = ProjectTarget::Provider {
        provider: ProviderId::parse("example.vm").unwrap(),
        resource_version: 0,
        reference: ProviderResourceRef::new(serde_json::json!({})).unwrap(),
    };
    assert!(matches!(
        ProjectDefinition::try_from(draft),
        Err(ProjectValidationError::InvalidResource)
    ));
}
#[test]
fn provider_and_resource_changes_invalidate_bindings_but_renames_preserve_them() {
    let mut draft = definition(1).to_draft();
    draft.target = ProjectTarget::Provider {
        provider: ProviderId::parse("example.vm").unwrap(),
        resource_version: 1,
        reference: ProviderResourceRef::new(serde_json::json!({"id":"one"})).unwrap(),
    };
    let original = ProjectDefinition::try_from(draft).unwrap();
    let mut state = CatalogState::new(Catalog::new(vec![original.clone()]).unwrap(), 0);
    let initial = state.stamp(original.id());
    let mut renamed = original.to_draft();
    renamed.name = "Other".into();
    let renamed = ProjectDefinition::try_from(renamed).unwrap();
    state.save(renamed.clone(), Some(&original)).unwrap();
    assert_eq!(state.stamp(original.id()), initial);
    let mut changed = renamed.to_draft();
    if let ProjectTarget::Provider { reference, .. } = &mut changed.target {
        *reference = ProviderResourceRef::new(serde_json::json!({"id":"two"})).unwrap();
    }
    state
        .save(changed.try_into().unwrap(), Some(&renamed))
        .unwrap();
    assert_ne!(state.stamp(original.id()), initial);
}

#[test]
fn invalid_or_null_envelope_cannot_fall_back_to_legacy_host_fields() {
    let base = serde_json::json!({"id":project(1),"name":"Example","host":{"kind":"local"},"root":"/","source":{"kind":"manual"},"updated":1});
    for target in [
        serde_json::Value::Null,
        serde_json::json!({"kind":"unknown"}),
    ] {
        let mut value = base.clone();
        value["target"] = target;
        assert!(serde_json::from_value::<ProjectDraft>(value).is_err());
    }
}
