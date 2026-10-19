use super::tickets::JobKey;
use nocterm_connections_model::observation::{CountryCode, FlagBytes, ObservationError};
use std::{collections::HashMap, sync::Arc};
#[derive(Clone, Debug)]
pub enum FlagState {
    Requested,
    Loading,
    Ready(FlagBytes),
    Unavailable,
}
struct FlagEntry {
    state: FlagState,
    job: Option<JobKey>,
}
pub(super) struct RawFlags {
    entries: HashMap<CountryCode, FlagEntry>,
}
impl RawFlags {
    pub(super) fn new() -> Self {
        Self {
            entries: HashMap::new(),
        }
    }
    pub(super) fn request(&mut self, code: CountryCode, network: bool) {
        self.entries.entry(code).or_insert(FlagEntry {
            state: if network {
                FlagState::Requested
            } else {
                FlagState::Unavailable
            },
            job: None,
        });
    }
    pub(super) fn pending(&self) -> Option<CountryCode> {
        self.entries
            .iter()
            .find_map(|(code, entry)| matches!(entry.state, FlagState::Requested).then_some(*code))
    }
    pub(super) fn loading(&mut self, code: CountryCode, job: JobKey) {
        if let Some(entry) = self.entries.get_mut(&code) {
            entry.state = FlagState::Loading;
            entry.job = Some(job);
        }
    }
    pub(super) fn finish(
        &mut self,
        code: CountryCode,
        job: &JobKey,
        result: Result<Option<Vec<u8>>, ObservationError>,
    ) -> bool {
        let Some(entry) = self.entries.get_mut(&code) else {
            return false;
        };
        if !entry
            .job
            .as_ref()
            .is_some_and(|original| Arc::ptr_eq(original, job))
        {
            return false;
        }
        entry.job = None;
        entry.state = match result {
            Ok(Some(bytes)) => match FlagBytes::new(bytes) {
                Ok(bytes) => FlagState::Ready(bytes),
                Err(error) => {
                    tracing::debug!(%error,"flag body refused");
                    FlagState::Unavailable
                }
            },
            Ok(None) => FlagState::Unavailable,
            Err(error) => {
                tracing::debug!(%error,"flag acquisition failed");
                FlagState::Unavailable
            }
        };
        true
    }
    pub(super) fn state(&self, code: CountryCode) -> Option<&FlagState> {
        self.entries.get(&code).map(|entry| &entry.state)
    }
}
