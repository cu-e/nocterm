//! Explicit scripted ports, gates and accounting for shared dev-dependency consumers.
use crate::observation::{
    CountryCode, CountryLocator, FlagRepository, ObservationError, PublicAddress,
    PublicAddressResolver,
};
use std::{
    collections::VecDeque,
    sync::{Arc, Mutex, Weak},
};
mod gates;
mod repository;
pub use gates::{BlockingGate, BlockingRelease};
pub use repository::{InMemoryConnectionRepository, RepositoryCall, RepositoryGate};
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ObservationCall {
    Resolve(String),
    Country(PublicAddress),
    Flag(CountryCode),
}
#[derive(Clone, Copy, PartialEq, Eq)]
enum Stage {
    Resolve,
    Country,
    Flag,
}
#[derive(Default)]
pub struct ScriptedObservations {
    resolutions: Mutex<VecDeque<Result<Option<PublicAddress>, ObservationError>>>,
    countries: Mutex<VecDeque<Result<Option<CountryCode>, ObservationError>>>,
    flags: Mutex<VecDeque<Result<Option<Vec<u8>>, ObservationError>>>,
    calls: Mutex<Vec<ObservationCall>>,
    gates: Mutex<VecDeque<(Stage, Arc<BlockingGate>)>>,
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
    pub fn suspend_next_resolution(&self, gate: Arc<BlockingGate>) {
        self.gates.lock().unwrap().push_back((Stage::Resolve, gate));
    }
    pub fn suspend_next_country(&self, gate: Arc<BlockingGate>) {
        self.gates.lock().unwrap().push_back((Stage::Country, gate));
    }
    pub fn suspend_next_flag(&self, gate: Arc<BlockingGate>) {
        self.gates.lock().unwrap().push_back((Stage::Flag, gate));
    }
    fn begin(&self, stage: Stage, call: ObservationCall) -> Option<Arc<BlockingGate>> {
        self.calls.lock().unwrap().push(call);
        let gate = {
            let mut gates = self.gates.lock().unwrap();
            if gates.front().is_some_and(|(kind, _)| *kind == stage) {
                gates.pop_front().map(|(_, gate)| gate)
            } else {
                None
            }
        };
        if let Some(gate) = &gate {
            gate.wait();
        }
        gate
    }
}
impl PublicAddressResolver for ScriptedObservations {
    fn resolve(&self, host: &str) -> Result<Option<PublicAddress>, ObservationError> {
        let gate = self.begin(Stage::Resolve, ObservationCall::Resolve(host.into()));
        let result = self
            .resolutions
            .lock()
            .unwrap()
            .pop_front()
            .unwrap_or(Ok(None));
        if let Some(gate) = gate {
            gate.complete();
        }
        result
    }
}
impl CountryLocator for ScriptedObservations {
    fn country(&self, ip: PublicAddress) -> Result<Option<CountryCode>, ObservationError> {
        let gate = self.begin(Stage::Country, ObservationCall::Country(ip));
        let result = self
            .countries
            .lock()
            .unwrap()
            .pop_front()
            .unwrap_or(Ok(None));
        if let Some(gate) = gate {
            gate.complete();
        }
        result
    }
}
impl FlagRepository for ScriptedObservations {
    fn load(&self, code: CountryCode) -> Result<Option<Vec<u8>>, ObservationError> {
        let gate = self.begin(Stage::Flag, ObservationCall::Flag(code));
        let result = self.flags.lock().unwrap().pop_front().unwrap_or(Ok(None));
        if let Some(gate) = gate {
            gate.complete();
        }
        result
    }
}
type OsReply = (
    Result<Option<crate::os::OsId>, crate::os_observation::OsObservationError>,
    Option<nocterm_session::testing::Gate>,
);
#[derive(Default)]
pub struct ScriptedOsDetector {
    replies: Mutex<VecDeque<OsReply>>,
    calls: std::sync::atomic::AtomicUsize,
    sessions: Mutex<Vec<Weak<dyn nocterm_session::RemoteFs>>>,
}
impl ScriptedOsDetector {
    pub fn answer_next(
        &self,
        result: Result<Option<crate::os::OsId>, crate::os_observation::OsObservationError>,
    ) {
        self.replies.lock().unwrap().push_back((result, None));
    }
    pub fn held_next(
        &self,
        result: Result<Option<crate::os::OsId>, crate::os_observation::OsObservationError>,
        gate: nocterm_session::testing::Gate,
    ) {
        self.replies.lock().unwrap().push_back((result, Some(gate)));
    }
    pub fn calls(&self) -> usize {
        self.calls.load(std::sync::atomic::Ordering::SeqCst)
    }
    pub fn sessions(&self) -> Vec<Weak<dyn nocterm_session::RemoteFs>> {
        self.sessions.lock().unwrap().clone()
    }
}
impl crate::os_observation::RemoteOsDetector for ScriptedOsDetector {
    fn detect(&self, fs: Arc<dyn nocterm_session::RemoteFs>) -> crate::os_observation::OsFuture {
        self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        self.sessions.lock().unwrap().push(Arc::downgrade(&fs));
        let (result, gate) = self
            .replies
            .lock()
            .unwrap()
            .pop_front()
            .unwrap_or((Ok(None), None));
        Box::pin(async move {
            if let Some(gate) = gate {
                gate.wait().await;
            }
            drop(fs);
            result
        })
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

/// Supplied clock with public call accounting, reusable across service/feature tests.
pub struct CountingClock {
    seconds: u64,
    calls: std::sync::atomic::AtomicUsize,
}
impl CountingClock {
    pub fn new(seconds: u64) -> Self {
        Self {
            seconds,
            calls: std::sync::atomic::AtomicUsize::new(0),
        }
    }
    pub fn calls(&self) -> usize {
        self.calls.load(std::sync::atomic::Ordering::SeqCst)
    }
}
impl crate::value_ports::ConnectionClock for CountingClock {
    fn unix_seconds(&self) -> u64 {
        self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        self.seconds
    }
}
