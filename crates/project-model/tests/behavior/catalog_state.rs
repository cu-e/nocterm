use nocterm_project_model::{Catalog, CatalogError, CatalogState, ProjectDefinition, RootPath};

use crate::support::{definition, project};

#[test]
fn checked_catalog_edits_reject_a_stale_definition_without_mutating_state() {
    let original = definition(1);
    let mut state = CatalogState::new(Catalog::new(vec![original.clone()]).unwrap(), 0);
    let mut draft = original.to_draft();
    draft.name = "Changed".into();
    let changed = ProjectDefinition::try_from(draft).unwrap();
    state.save(changed.clone(), Some(&original)).unwrap();
    let before = state.clone();
    assert!(matches!(
        state.save(original.clone(), Some(&original)),
        Err(CatalogError::Conflict(_))
    ));
    assert_eq!(state, before);
    state.remove(&changed).unwrap();
    assert!(state.catalog().is_empty());
}

#[test]
fn rename_preserves_source_stamp_but_root_cycles_retain_intermediate_revisions() {
    let original = definition(1);
    let mut state = CatalogState::new(Catalog::new(vec![original.clone()]).unwrap(), 0);
    let first = state.stamp(project(1)).unwrap();
    let mut draft = original.to_draft();
    draft.name = "Renamed".into();
    let renamed = ProjectDefinition::try_from(draft).unwrap();
    state.save(renamed.clone(), Some(&original)).unwrap();
    assert_eq!(state.stamp(project(1)).unwrap(), first);
    let mut draft = renamed.to_draft();
    if let nocterm_project_model::ProjectTarget::HostFolder { root, .. } = &mut draft.target {
        *root = RootPath::parse("/srv/other").unwrap();
    }
    let other = ProjectDefinition::try_from(draft).unwrap();
    state.save(other.clone(), Some(&renamed)).unwrap();
    state.save(original.clone(), Some(&other)).unwrap();
    assert_eq!(state.catalog().get(project(1)).unwrap(), &original);
    assert_ne!(state.stamp(project(1)).unwrap(), first);
}

#[test]
fn removing_and_readding_a_supplied_id_never_renews_its_old_generation() {
    let original = definition(1);
    let mut state = CatalogState::new(Catalog::new(vec![original.clone()]).unwrap(), 0);
    let first = state.stamp(project(1)).unwrap();
    state.remove(&original).unwrap();
    assert_eq!(state.stamp(project(1)), None);
    state.save(original, None).unwrap();
    assert_ne!(
        state.stamp(project(1)).unwrap().generation(),
        first.generation()
    );
}

#[test]
fn generation_exhaustion_refuses_changed_candidates_but_allows_an_exact_noop() {
    let original = definition(1);
    let mut state = CatalogState::new(Catalog::new(vec![original.clone()]).unwrap(), u64::MAX);
    let before = state.clone();
    state.save(original.clone(), Some(&original)).unwrap();
    assert_eq!(state, before);
    assert!(matches!(
        state.remove(&original),
        Err(CatalogError::RevisionExhausted)
    ));
    assert_eq!(state, before);
}

#[test]
fn exact_ssh_route_changes_invalidate_only_the_affected_source_stamp() {
    use nocterm_project_model::HostLocator;
    use nocterm_session::Target;
    let original = definition(1);
    let mut state = CatalogState::new(
        Catalog::new(vec![original.clone(), definition(2)]).unwrap(),
        0,
    );
    let first = state.stamp(project(1)).unwrap();
    let retained = state.stamp(project(2)).unwrap();
    let mut draft = original.to_draft();
    if let nocterm_project_model::ProjectTarget::HostFolder { host, .. } = &mut draft.target {
        *host = HostLocator::Ssh {
            profile: "profile".into(),
            target: Target::new("user", "host", 22),
            via: vec![Target::new("user", "bastion", 22)],
        };
    }
    state
        .save(draft.try_into().unwrap(), Some(&original))
        .unwrap();
    assert_ne!(state.stamp(project(1)).unwrap(), first);
    assert_eq!(state.stamp(project(2)).unwrap(), retained);
}
