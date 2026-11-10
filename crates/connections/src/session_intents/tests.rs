use super::*;
use nocterm_connections_model::jump_route::JumpHostEntry;
#[test]
fn saved_profile_keeps_description_and_session_overrides_when_opened() {
    let profile = Profile {
        id: nocterm_connections_model::test_support::profile_id_for(
            "crates/connections/src/model/tests.rs:2",
        ),
        name: "Legacy build".into(),
        description: "Windows-1251 machine\nUses a private proxy".into(),
        target: nocterm_session::Target::new("ci", "build.example", 22),
        auth: nocterm_session::Auth::Auto,
        group: None,
        credential: None,
        launch: None,
        icon: None,
        icon_color: None,
        country: None,
        options: nocterm_session::SessionOptions {
            term: Some("screen-256color".into()),
            charset: Some(nocterm_session::Charset::Windows1251),
            proxy: Some(nocterm_session::ProxyConfig::HttpConnect {
                host: "127.0.0.1".into(),
                port: 8080,
            }),
            logging: Some(nocterm_session::LoggingOptions::default()),
        },
        jump_hosts: Vec::new(),
    };
    let serialized = toml::to_string(&profile).unwrap();
    let restored: Profile = toml::from_str(&serialized).unwrap();
    assert_eq!(restored.description, profile.description);
    assert_eq!(
        spec_for_profile(&restored, &Default::default())
            .unwrap()
            .options,
        profile.options
    );
}

fn saved(name: &str, host: &str) -> Profile {
    Profile {
        id: nocterm_connections_model::test_support::profile_id_for(name),
        name: name.into(),
        description: String::new(),
        target: nocterm_session::Target::new("ops", host, 22),
        auth: nocterm_session::Auth::Password,
        group: None,
        credential: Some(nocterm_session::CredentialId::from_bytes([3; 16])),
        launch: None,
        icon: None,
        icon_color: None,
        country: None,
        options: Default::default(),
        jump_hosts: Vec::new(),
    }
}

#[test]
fn a_profile_hop_brings_its_target_authentication_and_saved_credential() {
    let bastion = saved("Bastion", "bastion.example");
    let mut inner = saved("Inner", "inner.example");
    inner.credential = None;
    inner.jump_hosts = vec![JumpHostEntry::Profile(bastion.id)];
    let mut profiles = Profiles::default();
    profiles.upsert(bastion.clone());
    profiles.upsert(inner.clone());

    let spec = spec_for_profile(&inner, &profiles).unwrap();

    assert_eq!(spec.target, inner.target);
    assert_eq!(spec.jumps.len(), 1);
    assert_eq!(spec.jumps[0].host.target, bastion.target);
    assert_eq!(spec.jumps[0].host.auth, bastion.auth);
    assert_eq!(spec.jumps[0].credential, bastion.credential);
}

#[test]
fn removing_the_hop_connection_makes_the_route_unresolvable() {
    let bastion = saved("Bastion", "bastion.example");
    let mut inner = saved("Inner", "inner.example");
    inner.jump_hosts = vec![JumpHostEntry::Profile(bastion.id)];
    let mut profiles = Profiles::default();
    profiles.upsert(bastion.clone());
    profiles.upsert(inner.clone());
    profiles.remove(bastion.id);

    assert!(matches!(
        spec_for_profile(&inner, &profiles),
        Err(JumpRouteError::MissingProfile { missing, .. }) if missing == bastion.id
    ));
}

#[test]
fn a_typed_address_hop_signs_in_automatically_without_a_credential() {
    let mut inner = saved("Inner", "inner.example");
    let jump = nocterm_session::Target::new("ops", "typed.example", 2200);
    inner.jump_hosts = vec![JumpHostEntry::Address(jump.clone())];

    let spec = spec_for_profile(&inner, &Profiles::default()).unwrap();

    assert_eq!(spec.jumps[0].host.target, jump);
    assert_eq!(spec.jumps[0].host.auth, nocterm_session::Auth::Auto);
    assert_eq!(spec.jumps[0].credential, None);
}

fn workspace_parent(
    profile: &Profile,
    via: Vec<nocterm_session::Target>,
) -> nocterm_project_model::HostLocator {
    nocterm_project_model::HostLocator::Ssh {
        profile: profile.id.to_string(),
        target: profile.target.clone(),
        via,
    }
}

#[test]
fn workspace_reconnect_keeps_the_saved_authentication_and_exact_ordered_jump_route() {
    let first = saved("first-workspace-hop", "first.example");
    let mut second = saved("second-workspace-hop", "second.example");
    second.jump_hosts = vec![JumpHostEntry::Profile(first.id)];
    let mut destination = saved("workspace-destination", "destination.example");
    destination.jump_hosts = vec![JumpHostEntry::Profile(second.id)];
    let mut profiles = Profiles::default();
    for profile in [&first, &second, &destination] {
        profiles.upsert(profile.clone());
    }
    let expected = workspace_parent(
        &destination,
        vec![first.target.clone(), second.target.clone()],
    );
    let spec = resolve_parent_spec(&expected, &profiles).unwrap();
    assert_eq!(spec.address().via, vec![first.target, second.target]);
    assert_eq!(spec.auth, destination.auth);
    assert_eq!(spec.credential, destination.credential);
    assert_eq!(spec.jumps[0].credential, first.credential);
    assert_eq!(spec.jumps[1].credential, second.credential);
}

#[test]
fn workspace_reconnect_refuses_changed_destinations_and_routes_before_opening() {
    use nocterm_workspace_ports::ProjectConnectionError;
    let mut destination = saved("checked-workspace-destination", "original.example");
    let hop = nocterm_session::Target::new("ops", "hop.example", 22);
    destination.jump_hosts = vec![JumpHostEntry::Address(hop.clone())];
    let expected = workspace_parent(&destination, vec![hop.clone()]);
    let mut profiles = Profiles::default();
    let mut changed = destination.clone();
    changed.target.host = "different.example".into();
    profiles.upsert(changed);
    assert!(matches!(
        resolve_parent_spec(&expected, &profiles),
        Err(ProjectConnectionError::ChangedTargetOrRoute)
    ));
    destination.jump_hosts.clear();
    profiles.upsert(destination.clone());
    assert!(matches!(
        resolve_parent_spec(&expected, &profiles),
        Err(ProjectConnectionError::ChangedTargetOrRoute)
    ));
    destination.jump_hosts = vec![
        JumpHostEntry::Address(hop.clone()),
        JumpHostEntry::Address(nocterm_session::Target::new("ops", "second.example", 22)),
    ];
    profiles.upsert(destination.clone());
    let reversed = workspace_parent(
        &destination,
        vec![
            nocterm_session::Target::new("ops", "second.example", 22),
            hop,
        ],
    );
    assert!(matches!(
        resolve_parent_spec(&reversed, &profiles),
        Err(ProjectConnectionError::ChangedTargetOrRoute)
    ));
}

#[test]
fn workspace_reconnect_distinguishes_removed_profiles_from_invalid_routes() {
    use nocterm_workspace_ports::ProjectConnectionError;
    use std::error::Error as _;
    let hop = saved("deleted-workspace-hop", "hop.example");
    let mut destination = saved("deleted-workspace-destination", "destination.example");
    destination.jump_hosts = vec![JumpHostEntry::Profile(hop.id)];
    let expected = workspace_parent(&destination, vec![hop.target]);
    let mut profiles = Profiles::default();
    assert!(matches!(
        resolve_parent_spec(&expected, &profiles),
        Err(ProjectConnectionError::MissingProfile)
    ));
    profiles.upsert(destination);
    let error = resolve_parent_spec(&expected, &profiles).err().unwrap();
    assert!(matches!(error, ProjectConnectionError::InvalidRoute(_)));
    assert!(
        error
            .source()
            .unwrap()
            .source()
            .unwrap()
            .downcast_ref::<JumpRouteError>()
            .is_some()
    );
}

#[test]
fn saved_workspace_profile_resolution_requires_a_saved_profile() {
    use nocterm_project_model::HostLocator;
    use nocterm_workspace_ports::ProjectConnectionError;
    let direct = HostLocator::SshDirect {
        target: nocterm_session::Target::new("ops", "manual.example", 22),
        via: vec![],
    };
    assert!(matches!(
        resolve_parent_spec(&direct, &Profiles::default()),
        Err(ProjectConnectionError::UnsupportedHost)
    ));
}
