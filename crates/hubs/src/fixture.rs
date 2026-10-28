//! A registry over a [`FakeHub`] and in-memory settings, shared by the tests of this crate.

use std::sync::Arc;

use gpui::{Entity, TestAppContext};
use nocterm_hubs_model::testing::{FakeAdminKeys, FakeHub, FakeServer};
use nocterm_hubs_model::{HubAddress, HubId, HubRecord, HubsSettings};
use nocterm_settings::SettingsDocument;
use nocterm_store::{SettingsConfig, SettingsHandle, StoreError};

use crate::{Deps, Hubs, install};

pub(crate) const ADDRESS: &str = "hub.home:7443";

pub(crate) struct Fixture {
    pub hub: FakeHub,
    pub admin_keys: FakeAdminKeys,
    pub hubs: Entity<Hubs>,
    pub settings: SettingsHandle,
}

pub(crate) fn address() -> HubAddress {
    HubAddress::parse(ADDRESS).unwrap()
}

pub(crate) fn id() -> HubId {
    HubId::from(address())
}

/// The server most tests talk to: certificate `[7; 32]`, user `ada` in organization `lab`.
pub(crate) fn server() -> FakeServer {
    FakeServer::new(ADDRESS, 7)
        .named("Home hub")
        .with_user("ada", "secret", "lab")
        .with_provider("corp", "Corporate SSO")
}

pub(crate) fn pinned_record() -> HubRecord {
    server().pinned_record("Home")
}

/// Installs the registry over `hub` with `known` servers in the settings.
pub(crate) fn fixture(cx: &mut TestAppContext, hub: FakeHub, known: Vec<HubRecord>) -> Fixture {
    build(cx, hub, known, None)
}

/// As [`fixture`], but every write of the settings fails.
pub(crate) fn fixture_with_failing_settings(cx: &mut TestAppContext, hub: FakeHub) -> Fixture {
    fixture_with_known_failing_settings(cx, hub, Vec::new())
}

/// As [`fixture_with_failing_settings`], with `known` servers already listed.
pub(crate) fn fixture_with_known_failing_settings(
    cx: &mut TestAppContext,
    hub: FakeHub,
    known: Vec<HubRecord>,
) -> Fixture {
    let writer: nocterm_store::SnapshotWriter<SettingsDocument> = Arc::new(|_| {
        Box::pin(async {
            Err(StoreError::backend(std::io::Error::from(
                std::io::ErrorKind::PermissionDenied,
            )))
        })
    });
    build(cx, hub, known, Some(writer))
}

fn build(
    cx: &mut TestAppContext,
    hub: FakeHub,
    known: Vec<HubRecord>,
    writer: Option<nocterm_store::SnapshotWriter<SettingsDocument>>,
) -> Fixture {
    cx.update(|cx| {
        let document = SettingsDocument::default();
        let config = match writer {
            Some(writer) => SettingsConfig::with_writer(document, writer),
            None => SettingsConfig::in_memory(document),
        };
        let settings = config
            .with(HubsSettings {
                servers: known,
                ..Default::default()
            })
            .install(cx);
        let admin_keys = FakeAdminKeys::alongside(&hub);
        let deps = Deps {
            trust: Arc::new(hub.clone()),
            sign_in: Arc::new(hub.clone()),
            admin_keys: Arc::new(admin_keys.clone()),
        };
        let installed = install(deps, settings.clone(), cx);
        Fixture {
            hub,
            admin_keys,
            hubs: installed.hubs().clone(),
            settings,
        }
    })
}

impl Fixture {
    pub(crate) fn status(&self, cx: &mut TestAppContext) -> Option<crate::HubStatus> {
        self.hubs.read_with(cx, |hubs, _| hubs.status(&id()))
    }

    pub(crate) fn servers(&self, cx: &mut TestAppContext) -> Vec<HubRecord> {
        cx.update(|cx| self.settings.setting::<HubsSettings>(cx).servers.clone())
    }
}
