//! An in-memory [`CredentialStore`] for terminal tests.
use futures::{FutureExt as _, channel::oneshot, future::BoxFuture};
use nocterm_session::{
    CredentialFuture, CredentialId, CredentialStore, CredentialStoreError, Secret, SecretRequest,
};
use std::sync::Mutex;

#[derive(Default)]
struct Inner {
    available: bool,
    records: Vec<(CredentialId, SecretRequest, Secret)>,
    waiters: Vec<oneshot::Sender<()>>,
}

#[derive(Default)]
pub(crate) struct FakeStore(Mutex<Inner>);

/// Whether a saved answer to `saved` answers `asked`.
fn same_question(saved: &SecretRequest, asked: &SecretRequest) -> bool {
    match (saved, asked) {
        (SecretRequest::Password { target: a, .. }, SecretRequest::Password { target: b, .. }) => {
            a == b
        }
        (
            SecretRequest::KeyPassphrase { path: a, .. },
            SecretRequest::KeyPassphrase { path: b, .. },
        ) => a == b,
        _ => false,
    }
}

impl FakeStore {
    pub(crate) fn unlocked() -> Self {
        let store = Self::default();
        store.0.lock().unwrap().available = true;
        store
    }

    pub(crate) fn unlock(&self) {
        let mut inner = self.0.lock().unwrap();
        inner.available = true;
        for waiter in inner.waiters.drain(..) {
            let _ = waiter.send(());
        }
    }

    /// Saves directly, as another device or an earlier session would have.
    pub(crate) fn insert(&self, request: SecretRequest, secret: &str) -> CredentialId {
        let mut inner = self.0.lock().unwrap();
        let id = CredentialId::from_bytes((inner.records.len() as u128 + 1).to_be_bytes());
        inner.records.push((id, request, Secret::new(secret)));
        id
    }

    /// Every saved credential as its question and answer.
    pub(crate) fn records(&self) -> Vec<(SecretRequest, String)> {
        self.0
            .lock()
            .unwrap()
            .records
            .iter()
            .map(|(_, request, secret)| (request.clone(), secret.expose().to_owned()))
            .collect()
    }
}

impl CredentialStore for FakeStore {
    fn is_available(&self) -> bool {
        self.0.lock().unwrap().available
    }

    fn wait_available(&self) -> BoxFuture<'static, ()> {
        let mut inner = self.0.lock().unwrap();
        if inner.available {
            return futures::future::ready(()).boxed();
        }
        let (sender, receiver) = oneshot::channel();
        inner.waiters.push(sender);
        receiver.map(|_| ()).boxed()
    }

    fn get(&self, id: CredentialId, request: &SecretRequest) -> CredentialFuture<Secret> {
        let inner = self.0.lock().unwrap();
        let result = if inner.available {
            inner
                .records
                .iter()
                .find(|(saved, _, _)| *saved == id)
                .filter(|(_, saved, _)| same_question(saved, request))
                .map(|(_, _, secret)| secret.clone())
                .ok_or_else(|| {
                    CredentialStoreError::Failed(
                        "The credential does not match this authentication request.".into(),
                    )
                })
        } else {
            Err(CredentialStoreError::Locked)
        };
        futures::future::ready(result).boxed()
    }

    fn save(
        &self,
        _label: String,
        request: &SecretRequest,
        secret: Secret,
    ) -> CredentialFuture<CredentialId> {
        let mut inner = self.0.lock().unwrap();
        let result = if inner.available && request.is_storable() {
            let id = CredentialId::from_bytes((inner.records.len() as u128 + 1).to_be_bytes());
            inner.records.push((id, request.clone(), secret));
            Ok(id)
        } else {
            Err(CredentialStoreError::Failed(
                "The vault cannot save this answer.".into(),
            ))
        };
        futures::future::ready(result).boxed()
    }
}
