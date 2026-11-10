use super::support::host;
use super::support::{dto, manifest};
use crate::{Additions, ContentHash, Grant, GrantError, HostError, Manifest, Package};

fn hash(byte: u8) -> ContentHash {
    ContentHash::new([byte; 32])
}

fn package(manifest: Manifest, byte: u8) -> Package {
    Package::new(hash(byte), manifest, alloc::vec![byte])
}

#[test]
fn consent_grants_required_capabilities_and_only_accepted_optional_ones() {
    let package = package(manifest(), 1);
    let id = package.manifest().plugin().id();
    let declined = Grant::consent(&package, &[]).unwrap();
    assert_eq!(declined.check(id, &hash(1), "clock.now"), Ok(()));
    assert_eq!(
        declined.check(id, &hash(1), "monitor.usage"),
        Err(HostError::Unavailable)
    );
    assert_eq!(
        declined.check(id, &hash(1), "storage.set"),
        Err(HostError::NotGranted)
    );
    let accepted = Grant::consent(&package, &["monitor.usage"]).unwrap();
    assert_eq!(accepted.check(id, &hash(1), "monitor.usage"), Ok(()));
    assert_eq!(accepted.granted().count(), 2);
}

#[test]
fn a_grant_is_bound_to_the_plugin_and_the_exact_hash() {
    let package = package(manifest(), 1);
    let id = package.manifest().plugin().id();
    let grant = Grant::consent(&package, &[]).unwrap();
    assert!(grant.covers(id, &hash(1)));
    assert_eq!(
        grant.check(id, &hash(2), "clock.now"),
        Err(HostError::NotGranted)
    );
    let other = nocterm_plugin_model::PluginId::user("acme", "other").unwrap();
    assert_eq!(
        grant.check(&other, &hash(1), "clock.now"),
        Err(HostError::NotGranted)
    );
}

#[test]
fn only_declared_known_optional_capabilities_can_be_accepted() {
    let known = package(manifest(), 1);
    for id in ["clock.now", "storage.set", "monitor.nothing"] {
        assert_eq!(
            Grant::consent(&known, &[id]),
            Err(GrantError::NotOptional(id.into()))
        );
    }
    let mut unknown = super::support::example();
    unknown.optional_capabilities.push("hologram.show".into());
    let package = package(Manifest::validate(&unknown, &host()).unwrap(), 1);
    assert!(Grant::consent(&package, &["hologram.show"]).is_err());
    let grant = Grant::consent(&package, &[]).unwrap();
    assert_eq!(
        grant.check(package.manifest().plugin().id(), &hash(1), "hologram.show"),
        Err(HostError::Unavailable)
    );
}

#[test]
fn an_update_lists_what_it_adds() {
    let before = manifest();
    let mut next = super::support::example();
    next.capabilities.push("workspace.kind".into());
    next.settings.as_mut().unwrap().group[0]
        .field
        .push(crate::FieldDto {
            key: "zone".into(),
            label: "Zone".into(),
            kind: Some(crate::FieldKindName::Toggle),
            ..crate::FieldDto::default()
        });
    let after = Manifest::validate(&next, &host()).unwrap();
    let added = Additions::between(&before, &after);
    assert_eq!(added.capabilities, ["workspace.kind"]);
    assert_eq!(added.settings, ["zone"]);
    assert!(Additions::between(&before, &before).is_empty());
    let _ = dto;
}

#[test]
fn an_optional_capability_that_becomes_required_is_an_addition() {
    let before = manifest();
    let mut next = super::support::example();
    next.optional_capabilities.clear();
    next.capabilities.push("monitor.usage".into());
    let after = Manifest::validate(&next, &host()).unwrap();
    assert_eq!(
        Additions::between(&before, &after).capabilities,
        ["monitor.usage"]
    );
    assert!(Additions::between(&after, &before).is_empty());
}

#[test]
fn a_grant_takes_its_hash_from_the_package() {
    let package = package(manifest(), 7);
    let grant = Grant::consent(&package, &[]).unwrap();
    assert_eq!(grant.hash(), package.hash());
}
