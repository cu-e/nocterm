//! Actual accepted session deadlines and sticky native clock, independent of RPC readiness.
use super::{RunnerCredentialDeadlines, RunnerHubError};
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Clone, Copy)]
pub(super) enum ClockSample {
    Millis(i64),
    Unavailable,
}
pub(super) fn now() -> ClockSample {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .and_then(|duration| i64::try_from(duration.as_millis()).ok())
        .map_or(ClockSample::Unavailable, ClockSample::Millis)
}
#[derive(Default)]
pub(super) struct RuntimeAuthority {
    deadlines: Option<RunnerCredentialDeadlines>,
    highwater: Option<i64>,
    fault: Option<RuntimeFault>,
}
#[derive(Clone, Copy)]
enum RuntimeFault {
    Clock,
    Expired,
    Source,
}
impl RuntimeAuthority {
    fn observe(&mut self, sample: ClockSample) -> Result<i64, RunnerHubError> {
        if let Some(fault) = self.fault {
            return Err(fault.error());
        }
        let valid = match sample {
            ClockSample::Millis(value)
                if value > 0 && self.highwater.is_none_or(|old| value >= old) =>
            {
                Some(value)
            }
            _ => None,
        };
        let Some(now) = valid else {
            self.fault = Some(RuntimeFault::Clock);
            return Err(RunnerHubError::Clock);
        };
        self.highwater = Some(now);
        Ok(now)
    }
    pub(super) fn accept(
        &mut self,
        deadlines: RunnerCredentialDeadlines,
        sample: ClockSample,
    ) -> Result<(), RunnerHubError> {
        let now = if self.deadlines.is_some() {
            self.check(sample)?
        } else {
            self.observe(sample)?
        };
        if deadlines.access <= 0 || deadlines.refresh <= now || deadlines.refresh < deadlines.access
        {
            self.fault = Some(RuntimeFault::Expired);
            return Err(RunnerHubError::HardExpired);
        }
        self.deadlines = Some(deadlines);
        Ok(())
    }
    pub(super) fn check(&mut self, sample: ClockSample) -> Result<i64, RunnerHubError> {
        let now = self.observe(sample)?;
        let deadlines = self.deadlines.ok_or(RunnerHubError::Unavailable)?;
        if now >= deadlines.refresh {
            self.fault = Some(RuntimeFault::Expired);
            return Err(RunnerHubError::HardExpired);
        }
        Ok(now)
    }
    pub(super) fn deadlines(&self) -> Result<RunnerCredentialDeadlines, RunnerHubError> {
        self.deadlines.ok_or(RunnerHubError::Unavailable)
    }
    pub(super) fn access_live(&self, now: i64) -> bool {
        self.deadlines.is_some_and(|value| now < value.access)
    }
    pub(super) fn fence_source(&mut self) {
        self.fault = Some(RuntimeFault::Source);
    }
}
impl RuntimeFault {
    fn error(self) -> RunnerHubError {
        match self {
            Self::Clock => RunnerHubError::Clock,
            Self::Expired => RunnerHubError::HardExpired,
            Self::Source => RunnerHubError::Unavailable,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn runner_runtime_accepted_sdk_deadlines_and_clock_cannot_revive_after_expiry() {
        let session = crate::cloud::runner_test_support::session(
            &crate::cloud::runner_test_support::subject(),
            "runtime",
        );
        let deadlines = RunnerCredentialDeadlines {
            access: session.access_expires_at_ms(),
            refresh: session.refresh_expires_at_ms(),
        };
        let ClockSample::Millis(now) = super::now() else {
            panic!("native clock");
        };
        let mut authority = RuntimeAuthority::default();
        authority
            .accept(deadlines, ClockSample::Millis(now))
            .unwrap();
        assert!(
            authority
                .check(ClockSample::Millis(deadlines.access + 1))
                .is_ok()
        );
        assert!(!authority.access_live(deadlines.access + 1));
        assert!(matches!(
            authority.check(ClockSample::Millis(deadlines.refresh)),
            Err(RunnerHubError::HardExpired)
        ));
        assert!(matches!(
            authority.check(ClockSample::Millis(now)),
            Err(RunnerHubError::HardExpired)
        ));
        let mut authority = RuntimeAuthority::default();
        authority
            .accept(deadlines, ClockSample::Millis(now))
            .unwrap();
        assert!(matches!(
            authority.check(ClockSample::Millis(now - 1)),
            Err(RunnerHubError::Clock)
        ));
        assert!(matches!(
            authority.check(ClockSample::Millis(now + 1)),
            Err(RunnerHubError::Clock)
        ));
    }
}
