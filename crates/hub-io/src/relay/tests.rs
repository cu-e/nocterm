use futures::executor::block_on;
use nocterm_session::{RelayDialer as _, RelayError, Target};

use super::HubRelay;
use crate::{HubLinks, HubLinksConfig};

fn target() -> Target {
    Target {
        host: "db.internal".into(),
        port: 22,
        user: "deploy".into(),
    }
}

fn links(dir: &tempfile::TempDir) -> std::sync::Arc<HubLinks> {
    HubLinks::start(HubLinksConfig {
        credentials_dir: dir.path().join("hubs"),
        device_name: "relay test".into(),
    })
    .unwrap()
}

#[test]
fn a_hub_this_device_is_not_signed_in_to_is_not_dialled() {
    let dir = tempfile::tempdir().unwrap();
    let links = links(&dir);
    let relay = HubRelay::new(&links);
    for hub in [
        "hub.example",
        "https://Hub.Example:7443/",
        "not a hub address",
        "",
    ] {
        let dialled = block_on(relay.dial(hub, None, &target()));
        assert_eq!(
            dialled.unwrap_err(),
            RelayError::NotSignedIn { hub: hub.into() },
            "{hub:?}"
        );
    }
}

#[test]
fn dialling_after_the_links_stopped_finds_the_relay_closed() {
    let dir = tempfile::tempdir().unwrap();
    let links = links(&dir);
    let relay = HubRelay::new(&links);
    drop(links);
    let dialled = block_on(relay.dial("hub.example", None, &target()));
    assert_eq!(dialled.unwrap_err(), RelayError::Closed);
}
