use super::*;
use crate::test_support::profile_id_for;
use nocterm_session::CredentialId;

fn profile(name: &str, host: &str, jumps: Vec<JumpHostEntry>) -> Profile {
    Profile {
        description: String::new(),
        options: Default::default(),
        id: profile_id_for(name),
        name: name.to_owned(),
        target: Target::new("root", host, 22),
        auth: Auth::Auto,
        group: None,
        credential: None,
        launch: None,
        icon: None,
        icon_color: None,
        country: None,
        jump_hosts: jumps,
    }
}

fn via(name: &str) -> JumpHostEntry {
    JumpHostEntry::Profile(profile_id_for(name))
}

fn saved(profiles: Vec<Profile>) -> Profiles {
    let mut saved = Profiles::default();
    for profile in profiles {
        saved.upsert(profile);
    }
    saved
}

fn hosts(route: &[ResolvedJump]) -> Vec<&str> {
    route
        .iter()
        .map(|hop| hop.host.target.host.as_str())
        .collect()
}

fn document(entries: &str) -> String {
    format!(
        "[[connection]]\nid = \"6f1c1f3e-8f6e-4b8a-9d5e-0a1b2c3d4e5f\"\nname = \"db\"\nhost = \"10.0.0.5\"\nuser = \"admin\"\n{entries}"
    )
}

const EXPECTED_WITHOUT_JUMPS: &str = "[[connection]]\nid = \"00000000-0000-0000-0891-4e07b53ba1e3\"\nname = \"db\"\nhost = \"10.0.0.5\"\nuser = \"root\"\n";

#[test]
fn a_profile_without_jumps_serializes_as_before() {
    let mut saved = Profiles::default();
    saved.upsert(profile("db", "10.0.0.5", Vec::new()));
    let text = toml::to_string(&saved).unwrap();
    assert_eq!(text, EXPECTED_WITHOUT_JUMPS);
    let reloaded: Profiles = toml::from_str(&text).unwrap();
    assert_eq!(reloaded, saved);
    let plain: Profiles = toml::from_str(&document("")).unwrap();
    assert!(plain.iter().all(|profile| profile.jump_hosts.is_empty()));
}

#[test]
fn both_entry_forms_round_trip_and_omit_the_default_port() {
    let mut saved = Profiles::default();
    saved.upsert(profile(
        "db",
        "10.0.0.5",
        vec![
            via("bastion"),
            JumpHostEntry::Address(Target::new("ops", "gw.example", 22)),
            JumpHostEntry::Address(Target::new("ops", "gw2.example", 2222)),
        ],
    ));
    let text = toml::to_string(&saved).unwrap();
    assert_eq!(
        text.matches("[[connection.jump_hosts]]").count(),
        3,
        "{text}"
    );
    assert_eq!(text.matches("port = ").count(), 1, "{text}");
    assert_eq!(toml::from_str::<Profiles>(&text).unwrap(), saved);
}

#[test]
fn hand_written_entries_load_and_unknown_keys_are_ignored() {
    let text = document(
        "\n[[connection.jump_hosts]]\nprofile = \"0b9f3a51-2d64-4f1e-8c7a-5e6d7c8b9a0f\"\nnote = \"future\"\n\n[[connection.jump_hosts]]\nhost = \"bastion.example\"\nuser = \"ops\"\nport = 2222\n",
    );
    let loaded: Profiles = toml::from_str(&text).unwrap();
    let jumps = &loaded.iter().next().unwrap().jump_hosts;
    assert_eq!(jumps.len(), 2);
    assert_eq!(
        jumps[1],
        JumpHostEntry::Address(Target::new("ops", "bastion.example", 2222))
    );
}

#[test]
fn malformed_entries_are_rejected_with_a_message_naming_the_problem() {
    let id = "profile = \"0b9f3a51-2d64-4f1e-8c7a-5e6d7c8b9a0f\"";
    for (entry, expected) in [
        (format!("{id}\nhost = \"x\"\nuser = \"u\""), "not both"),
        (format!("{id}\nport = 2222"), "not both"),
        (String::new(), "needs a saved connection"),
        ("host = \"bastion\"".to_owned(), "needs a `user`"),
    ] {
        let text = document(&format!("\n[[connection.jump_hosts]]\n{entry}\n"));
        let error = toml::from_str::<Profiles>(&text).unwrap_err().to_string();
        assert!(error.contains(expected), "{entry:?} gave {error}");
    }
}

#[test]
fn an_address_hop_uses_automatic_authentication_and_no_credential() {
    let destination = profile(
        "db",
        "10.0.0.5",
        vec![JumpHostEntry::Address(Target::new("ops", "gw", 2222))],
    );
    let route = resolve_jump_route(&saved(vec![destination.clone()]), &destination).unwrap();
    assert_eq!(
        route,
        [ResolvedJump {
            host: JumpHost {
                target: Target::new("ops", "gw", 2222),
                auth: Auth::Auto
            },
            credential: None
        }]
    );
}

#[test]
fn a_profile_hop_takes_the_target_auth_and_credential_of_its_profile() {
    let mut bastion = profile("bastion", "bastion.example", Vec::new());
    bastion.auth = Auth::Password;
    bastion.credential = Some(CredentialId::from_bytes([3; 16]));
    let destination = profile("db", "10.0.0.5", vec![via("bastion")]);
    let route = resolve_jump_route(&saved(vec![bastion.clone()]), &destination).unwrap();
    assert_eq!(
        route,
        [ResolvedJump {
            host: JumpHost {
                target: bastion.target,
                auth: Auth::Password
            },
            credential: bastion.credential
        }]
    );
}

#[test]
fn nested_jump_hosts_come_before_the_profile_that_uses_them() {
    let a = profile("a", "a.example", Vec::new());
    let b = profile("b", "b.example", vec![via("a")]);
    let d = profile("d", "d.example", vec![via("b")]);
    let route = resolve_jump_route(&saved(vec![a, b, d.clone()]), &d).unwrap();
    assert_eq!(hosts(&route), ["a.example", "b.example"]);
}

#[test]
fn a_route_that_passes_through_its_own_profile_is_a_cycle() {
    let own = profile("a", "a.example", vec![via("a")]);
    assert!(matches!(
        resolve_jump_route(&saved(vec![own.clone()]), &own),
        Err(JumpRouteError::Cycle { profile }) if profile == "a"
    ));
    let a = profile("a", "a.example", vec![via("b")]);
    let b = profile("b", "b.example", vec![via("a")]);
    assert!(matches!(
        resolve_jump_route(&saved(vec![a.clone(), b]), &a),
        Err(JumpRouteError::Cycle { .. })
    ));
}

#[test]
fn an_unsaved_edit_that_closes_a_cycle_is_detected() {
    let a = profile("a", "a.example", Vec::new());
    let b = profile("b", "b.example", vec![via("a")]);
    let stored = saved(vec![a.clone(), b]);
    assert!(resolve_jump_route(&stored, &a).unwrap().is_empty());
    let edited = profile("a", "a.example", vec![via("b")]);
    assert!(matches!(
        resolve_jump_route(&stored, &edited),
        Err(JumpRouteError::Cycle { .. })
    ));
}

#[test]
fn a_reference_to_a_removed_profile_is_reported_with_the_referrer() {
    let destination = profile("db", "10.0.0.5", vec![via("gone")]);
    assert_eq!(
        resolve_jump_route(&saved(vec![destination.clone()]), &destination),
        Err(JumpRouteError::MissingProfile {
            referrer: "db".into(),
            missing: profile_id_for("gone")
        })
    );
}

#[test]
fn eight_hops_are_accepted_and_nine_are_too_long() {
    let hops = |count: usize| {
        profile(
            "db",
            "10.0.0.5",
            (0..count)
                .map(|n| JumpHostEntry::Address(Target::new("ops", format!("h{n}"), 22)))
                .collect(),
        )
    };
    let ok = hops(8);
    assert_eq!(
        resolve_jump_route(&saved(vec![ok.clone()]), &ok)
            .unwrap()
            .len(),
        8
    );
    let long = hops(9);
    assert_eq!(
        resolve_jump_route(&saved(vec![long.clone()]), &long),
        Err(JumpRouteError::TooLong { limit: 8 })
    );
}
