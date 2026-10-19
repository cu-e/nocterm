use super::*;
use crate::profiles::Profile;
fn profile() -> Profile {
    Profile {
        id: ProfileId::from_bytes([1; 16]),
        name: "host".into(),
        description: String::new(),
        target: Target::new("root", "host", 22),
        auth: Default::default(),
        options: Default::default(),
        group: None,
        credential: None,
        launch: None,
        icon: None,
        icon_color: None,
        country: None,
    }
}
fn documents(profile: &Profile) -> Profiles {
    let mut documents = Profiles::default();
    documents.upsert(profile.clone());
    documents
}
#[test]
fn imported_identity_and_presentation_edits_keep_authority() {
    let mut p = profile();
    let before = documents(&p);
    let mut epochs = ObservationEpochs::imported(&before);
    let initial = epochs.publication(0, &before);
    assert_eq!(initial.get(p.id).unwrap().stamp, ProfileStamp::at(0));
    p.name = "renamed".into();
    p.group = Some("Work".into());
    p.icon_color = Some("#ffffff".into());
    p.credential = Some(nocterm_session::CredentialId::from_bytes([2; 16]));
    let after = documents(&p);
    epochs.transition(&before, &after, 1);
    assert_eq!(
        epochs.publication(1, &after).get(p.id).unwrap().stamp,
        initial.get(p.id).unwrap().stamp
    );
}
#[test]
fn endpoint_and_override_cycles_survive_equal_final_profiles() {
    let original = profile();
    let mut before = documents(&original);
    let mut epochs = ObservationEpochs::imported(&before);
    let mut p = original.clone();
    for (generation, host, icon, country) in [
        (1, "other", Some("debian"), Some("de")),
        (2, "host", None, None),
    ] {
        p.target.host = host.into();
        p.icon = icon.map(str::to_owned);
        p.country = country.map(str::to_owned);
        let after = documents(&p);
        epochs.transition(&before, &after, generation);
        before = after;
    }
    assert_eq!(before, documents(&original));
    let projection = epochs.publication(2, &before);
    let stamp = projection.get(original.id).unwrap().stamp;
    assert_eq!(stamp.incarnation, 0);
    assert_eq!(stamp.endpoint, 2);
    assert_eq!(stamp.icon_override, 2);
    assert_eq!(stamp.country_override, 2);
}
#[test]
fn deletion_prunes_and_equal_readdition_gets_fresh_incarnation() {
    let p = profile();
    let before = documents(&p);
    let mut epochs = ObservationEpochs::imported(&before);
    let empty = Profiles::default();
    epochs.transition(&before, &empty, 1);
    assert_eq!(epochs.publication(1, &empty).iter().count(), 0);
    epochs.transition(&empty, &before, 2);
    assert_eq!(
        epochs.publication(2, &before).get(p.id).unwrap().stamp,
        ProfileStamp::at(2)
    );
}
#[test]
fn endpoint_includes_user_and_port_and_each_override_is_independent() {
    let mut p = profile();
    let before = documents(&p);
    let mut epochs = ObservationEpochs::imported(&before);
    p.target.user = "other-user".into();
    p.target.port = 2222;
    let after = documents(&p);
    epochs.transition(&before, &after, 1);
    let stamp = epochs.publication(1, &after).get(p.id).unwrap().stamp;
    assert_eq!(stamp.endpoint, 1);
    assert_eq!(stamp.icon_override, 0);
    assert_eq!(stamp.country_override, 0);
}
