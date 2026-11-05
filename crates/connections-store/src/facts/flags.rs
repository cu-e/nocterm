use super::tickets::JobKey;
use nocterm_connections_model::observation::{CountryCode, FlagBytes, ObservationError};
use std::{
    collections::HashMap,
    sync::Arc,
    time::{Duration, Instant},
};
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
    attempts: u8,
    retry_at: Option<Instant>,
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
            attempts: 0,
            retry_at: None,
        });
    }
    pub(super) fn pending(&self) -> Option<CountryCode> {
        self.entries
            .iter()
            .find_map(|(code, entry)| matches!(entry.state, FlagState::Requested).then_some(*code))
    }
    pub(super) fn loading(&mut self, code: CountryCode, job: JobKey) {
        if let Some(entry) = self.entries.get_mut(&code) {
            entry.attempts += 1;
            entry.retry_at = None;
            entry.state = FlagState::Loading;
            entry.job = Some(job);
        }
    }
    pub(super) fn finish(
        &mut self,
        code: CountryCode,
        job: &JobKey,
        result: Result<Option<Vec<u8>>, ObservationError>,
        now: Instant,
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
                if entry.attempts < 3 && retryable(&error) {
                    entry.retry_at =
                        Some(now + Duration::from_secs(if entry.attempts == 1 { 5 } else { 30 }));
                }
                FlagState::Unavailable
            }
        };
        true
    }
    pub(super) fn retry_due(&mut self, now: Instant) {
        for entry in self.entries.values_mut() {
            if entry.retry_at.is_some_and(|deadline| deadline <= now) {
                entry.retry_at = None;
                entry.state = FlagState::Requested;
            }
        }
    }
    pub(super) fn retry_deadline(&self) -> Option<Instant> {
        self.entries
            .values()
            .filter_map(|entry| entry.retry_at)
            .min()
    }
    pub(super) fn state(&self, code: CountryCode) -> Option<&FlagState> {
        self.entries.get(&code).map(|entry| &entry.state)
    }
}

fn retryable(error: &ObservationError) -> bool {
    match error {
        ObservationError::Network(_)
        | ObservationError::Resolution(_)
        | ObservationError::Body(_)
        | ObservationError::Cache(_) => true,
        ObservationError::Status { status, .. } => *status >= 500 || matches!(*status, 408 | 429),
        ObservationError::BodyLimit { .. } | ObservationError::Decode(_) => false,
    }
}
