use super::{Gate, lock};
use crate::{CredentialFuture, CredentialId, CredentialStore, Secret, SecretRequest};
use futures::future::BoxFuture;
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

/// Classified fake-store failures. The legacy CredentialStore port still converts
/// them to its existing string error; direct test setup and queries keep this type.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum FakeCredentialError {
    #[error("the credential store is locked")]
    Locked,
    #[error("the credential does not exist")]
    NotFound,
    #[error("the credential does not answer this request")]
    BindingMismatch,
    #[error("interactive answers cannot be stored")]
    NotStorable,
    #[error("the credential identifier space is exhausted")]
    Exhausted,
}

struct Record {
    request: SecretRequest,
    secret: Secret,
    label: String,
}

struct CredentialState {
    available: bool,
    availability: Gate,
    operation: Option<Gate>,
    records: HashMap<CredentialId, Record>,
    next: u128,
    failure: Option<FakeCredentialError>,
}

/// Saved secrets with binding validation and event-driven availability. Retry
/// flags do not change a binding; interactive/MFA requests are never saved.
#[derive(Clone)]
pub struct FakeCredentialStore {
    state: Arc<Mutex<CredentialState>>,
}

impl Default for FakeCredentialStore {
    fn default() -> Self {
        Self {
            state: Arc::new(Mutex::new(CredentialState {
                available: true,
                availability: Gate::open(),
                operation: None,
                records: HashMap::new(),
                next: 1,
                failure: None,
            })),
        }
    }
}

impl FakeCredentialStore {
    pub fn set_available(&self, available: bool) {
        let mut state = lock(&self.state);
        if available == state.available {
            return;
        }
        state.available = available;
        if available {
            state.availability.release();
        } else {
            state.availability = Gate::pending();
        }
    }

    pub fn suspend(&self, gate: Gate) {
        lock(&self.state).operation = Some(gate);
    }

    pub fn fail_next(&self, error: FakeCredentialError) {
        lock(&self.state).failure = Some(error);
    }

    pub fn insert(
        &self,
        label: String,
        request: &SecretRequest,
        secret: Secret,
    ) -> Result<CredentialId, FakeCredentialError> {
        let mut state = lock(&self.state);
        check(&mut state)?;
        if !request.is_storable() {
            return Err(FakeCredentialError::NotStorable);
        }
        let id = format!("{:032x}", state.next)
            .parse()
            .map_err(|_| FakeCredentialError::Exhausted)?;
        state.next = state
            .next
            .checked_add(1)
            .ok_or(FakeCredentialError::Exhausted)?;
        state.records.insert(
            id,
            Record {
                request: request.clone(),
                secret,
                label,
            },
        );
        Ok(id)
    }

    pub fn lookup(
        &self,
        id: CredentialId,
        request: &SecretRequest,
    ) -> Result<Secret, FakeCredentialError> {
        let mut state = lock(&self.state);
        check(&mut state)?;
        let record = state
            .records
            .get(&id)
            .ok_or(FakeCredentialError::NotFound)?;
        if !same_binding(&record.request, request) {
            return Err(FakeCredentialError::BindingMismatch);
        }
        Ok(record.secret.clone())
    }

    pub fn labels(&self) -> Vec<String> {
        let mut labels: Vec<_> = lock(&self.state)
            .records
            .values()
            .map(|record| record.label.clone())
            .collect();
        labels.sort();
        labels
    }
}

fn check(state: &mut CredentialState) -> Result<(), FakeCredentialError> {
    if !state.available {
        return Err(FakeCredentialError::Locked);
    }
    if let Some(error) = state.failure.take() {
        return Err(error);
    }
    Ok(())
}

fn same_binding(saved: &SecretRequest, asked: &SecretRequest) -> bool {
    match (saved, asked) {
        (
            SecretRequest::Password { target: saved, .. },
            SecretRequest::Password { target: asked, .. },
        ) => saved == asked,
        (
            SecretRequest::KeyPassphrase { path: saved, .. },
            SecretRequest::KeyPassphrase { path: asked, .. },
        ) => saved == asked,
        _ => false,
    }
}

impl CredentialStore for FakeCredentialStore {
    fn is_available(&self) -> bool {
        lock(&self.state).available
    }

    fn wait_available(&self) -> BoxFuture<'static, ()> {
        let store = self.clone();
        Box::pin(async move {
            loop {
                let gate = {
                    let state = lock(&store.state);
                    if state.available {
                        return;
                    }
                    state.availability.clone()
                };
                gate.wait().await;
            }
        })
    }

    fn get(&self, id: CredentialId, request: &SecretRequest) -> CredentialFuture<Secret> {
        let store = self.clone();
        let request = request.clone();
        let gate = lock(&self.state).operation.clone();
        Box::pin(async move {
            if let Some(gate) = gate {
                gate.wait().await;
            }
            store
                .lookup(id, &request)
                .map_err(|error| error.to_string())
        })
    }

    fn save(
        &self,
        label: String,
        request: &SecretRequest,
        secret: Secret,
    ) -> CredentialFuture<CredentialId> {
        let store = self.clone();
        let request = request.clone();
        let gate = lock(&self.state).operation.clone();
        Box::pin(async move {
            if let Some(gate) = gate {
                gate.wait().await;
            }
            store
                .insert(label, &request, secret)
                .map_err(|error| error.to_string())
        })
    }
}
