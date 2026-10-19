//! Explicit collaborators for observations; concrete I/O is composed by the app.
use nocterm_connections_model::observation::{
    CountryCode, CountryLocator, FlagRepository, ObservationError, OfflineObservations,
    PublicAddressResolver,
};
use nocterm_connections_model::os_observation::{OfflineOsDetector, RemoteOsDetector};
use std::{collections::BTreeMap, sync::Arc};

#[derive(Clone)]
pub struct ObservationDeps {
    resolver: Arc<dyn PublicAddressResolver>,
    locator: Arc<dyn CountryLocator>,
    flags: Arc<dyn FlagRepository>,
    network_enabled: bool,
    os: Arc<dyn RemoteOsDetector>,
}
impl ObservationDeps {
    pub fn new(
        resolver: Arc<dyn PublicAddressResolver>,
        locator: Arc<dyn CountryLocator>,
        flags: Arc<dyn FlagRepository>,
        network_enabled: bool,
        os: Arc<dyn RemoteOsDetector>,
    ) -> Self {
        Self {
            resolver,
            locator,
            flags,
            network_enabled,
            os,
        }
    }
    pub fn offline() -> Self {
        Self::new(
            Arc::new(OfflineObservations),
            Arc::new(OfflineObservations),
            Arc::new(OfflineObservations),
            false,
            Arc::new(OfflineOsDetector),
        )
    }
    pub(crate) fn os(&self) -> &dyn RemoteOsDetector {
        self.os.as_ref()
    }
    pub(crate) fn resolver(&self) -> &dyn PublicAddressResolver {
        self.resolver.as_ref()
    }
    pub(crate) fn locator(&self) -> &dyn CountryLocator {
        self.locator.as_ref()
    }
    pub(crate) fn flags(&self) -> &dyn FlagRepository {
        self.flags.as_ref()
    }
    pub(crate) fn network_enabled(&self) -> bool {
        self.network_enabled
    }
}

/// Immutable observation request; its background work captures no view or App.
pub(crate) struct LocationRequest {
    host: String,
    cached: Arc<BTreeMap<String, String>>,
    observations: ObservationDeps,
}
impl LocationRequest {
    pub(crate) fn new(
        host: &str,
        cached: Arc<BTreeMap<String, String>>,
        observations: ObservationDeps,
    ) -> Self {
        Self {
            host: host.to_owned(),
            cached,
            observations,
        }
    }
    pub(crate) fn locate(
        self,
    ) -> Result<(String, Option<(String, CountryCode)>), ObservationError> {
        let found = self.country()?;
        Ok((self.host, found))
    }
    fn country(&self) -> Result<Option<(String, CountryCode)>, ObservationError> {
        if !self.observations.network_enabled() {
            return Ok(None);
        }
        let Some(ip) = self.observations.resolver().resolve(&self.host)? else {
            return Ok(None);
        };
        let key = ip.as_ip().to_string();
        if let Some(code) = self
            .cached
            .get(&key)
            .and_then(|code| CountryCode::new(code).ok())
        {
            return Ok(Some((key, code)));
        }
        let code = self.observations.locator().country(ip)?;
        Ok(code.map(|code| (key, code)))
    }
}
