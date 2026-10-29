//! One borrowed worker retains strong transient keys only during actual work.
use super::*;
use crate::account::operations::OpenedWork;
pub(super) struct RegistryOperation {
    work: OpenedWork,
    bound: BoundOperations,
    cloud: Arc<CloudClient>,
    scope: AccountScope,
    context: Vec<u8>,
    registration: Option<RunnerRegistration>,
    resident: AllocationLease,
}
impl RegistryOperation {
    pub(super) fn new(
        work: OpenedWork,
        bound: BoundOperations,
        cloud: Arc<CloudClient>,
        scope: AccountScope,
        context: Vec<u8>,
        registration: Option<RunnerRegistration>,
        resident: AllocationLease,
    ) -> Self {
        Self {
            work,
            bound,
            cloud,
            scope,
            context,
            registration,
            resident,
        }
    }
    pub(super) async fn execute(&self) -> Result<TrustedRunnerRoster, RunnerRegistryError> {
        let (version, runners) = storage::load_or_register(
            &self.work,
            &self.scope,
            &self.context,
            self.registration.as_ref(),
        )
        .await?;
        self.work.source().check().map_err(account)?;
        self.cloud.check_source().map_err(backend)?;
        let result = TrustedRunnerRoster {
            resident: Allocated::new(
                RosterData {
                    scope: self.scope.clone(),
                    version,
                    runners,
                    bound: self.bound.clone(),
                    cloud: self.cloud.clone(),
                },
                self.resident.clone(),
            ),
        };
        self.work.source().publish(|| result).map_err(account)
    }
}
