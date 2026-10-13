//! Explicitly scripted, no-I/O observation and document ports for dev-dependency consumers.
use crate::observation::{
    CountryCode, CountryLocator, FlagRepository, ObservationError, PublicAddress,
    PublicAddressResolver,
};
use std::{collections::VecDeque, sync::Mutex};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ObservationCall {
    Resolve(String),
    Country(PublicAddress),
    Flag(CountryCode),
}

#[derive(Default)]
pub struct ScriptedObservations {
    resolutions: Mutex<VecDeque<Result<Option<PublicAddress>, ObservationError>>>,
    countries: Mutex<VecDeque<Result<Option<CountryCode>, ObservationError>>>,
    flags: Mutex<VecDeque<Result<Option<Vec<u8>>, ObservationError>>>,
    calls: Mutex<Vec<ObservationCall>>,
}
impl ScriptedObservations {
    pub fn resolve_next(&self, result: Result<Option<PublicAddress>, ObservationError>) {
        self.resolutions.lock().unwrap().push_back(result);
    }
    pub fn country_next(&self, result: Result<Option<CountryCode>, ObservationError>) {
        self.countries.lock().unwrap().push_back(result);
    }
    pub fn flag_next(&self, result: Result<Option<Vec<u8>>, ObservationError>) {
        self.flags.lock().unwrap().push_back(result);
    }
    pub fn calls(&self) -> Vec<ObservationCall> {
        self.calls.lock().unwrap().clone()
    }
}
impl PublicAddressResolver for ScriptedObservations {
    fn resolve(&self, host: &str) -> Result<Option<PublicAddress>, ObservationError> {
        self.calls
            .lock()
            .unwrap()
            .push(ObservationCall::Resolve(host.into()));
        self.resolutions
            .lock()
            .unwrap()
            .pop_front()
            .unwrap_or(Ok(None))
    }
}
impl CountryLocator for ScriptedObservations {
    fn country(&self, ip: PublicAddress) -> Result<Option<CountryCode>, ObservationError> {
        self.calls
            .lock()
            .unwrap()
            .push(ObservationCall::Country(ip));
        self.countries
            .lock()
            .unwrap()
            .pop_front()
            .unwrap_or(Ok(None))
    }
}
impl FlagRepository for ScriptedObservations {
    fn load(&self, code: CountryCode) -> Result<Option<Vec<u8>>, ObservationError> {
        self.calls.lock().unwrap().push(ObservationCall::Flag(code));
        self.flags.lock().unwrap().pop_front().unwrap_or(Ok(None))
    }
}

/// An identifier computed only from supplied test data, with no entropy or global sequence.
pub fn profile_id_for(value: &str) -> crate::profiles::ProfileId {
    let mut hash: u64 = 14695981039346656037;
    for byte in value.bytes() {
        hash = (hash ^ u64::from(byte)).wrapping_mul(1099511628211);
    }
    let mut bytes = [0; 16];
    bytes[8..].copy_from_slice(&hash.to_be_bytes());
    crate::profiles::ProfileId::from_bytes(bytes)
}

mod repository;
pub use repository::{InMemoryConnectionRepository, RepositoryCall, RepositoryGate};

/// Scripted OS capability; calls do not touch the supplied filesystem.
#[derive(Default)]
pub struct ScriptedOsDetector {
    replies:
        Mutex<VecDeque<Result<Option<crate::os::OsId>, crate::os_observation::OsObservationError>>>,
    calls: std::sync::atomic::AtomicUsize,
}
impl ScriptedOsDetector {
    pub fn answer_next(
        &self,
        result: Result<Option<crate::os::OsId>, crate::os_observation::OsObservationError>,
    ) {
        self.replies
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .push_back(result);
    }
    pub fn calls(&self) -> usize {
        self.calls.load(std::sync::atomic::Ordering::SeqCst)
    }
}
impl crate::os_observation::RemoteOsDetector for ScriptedOsDetector {
    fn detect(
        &self,
        _: std::sync::Arc<dyn nocterm_session::RemoteFs>,
    ) -> crate::os_observation::OsFuture {
        self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let result = self
            .replies
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .pop_front()
            .unwrap_or(Ok(None));
        Box::pin(async move { result })
    }
}
