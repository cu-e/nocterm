//! Production authorizer fixture; records constrained requests without replacing crypto.
use super::*;
use futures::future::BoxFuture;
use nocterm_cloud_model::{
    authority::{AuthenticationBinding, RunBinding, TargetBinding},
    identity::{
        AccountIdentity, AccountScope, CloudChatId, Fingerprint, HubIdentity, NamespaceId,
        RunEpoch, RunId, RunnerId, RunnerKeyEpoch, Timestamp,
    },
    login::{LoginConsentBinding, LoginError, LoginProofRequest, SessionLoginAuthorizer},
    testing::{ManualToolTimer, ScriptedLoginAuthority},
};
use nocterm_session::{ConnectRequest, testing::FakeCredentialStore};
use nocterm_ssh::proof::{DeviceLoginAuthorizer, LoginConsentRequest, SessionLoginAttempt};
use nocterm_ssh_keys::{KeyMaterial, PublicKeyText, login::LoginSignature};
use std::sync::{Arc, Mutex};
pub(crate) struct RecordingAuthorizer {
    inner: Arc<DeviceLoginAuthorizer>,
    requests: Mutex<Vec<LoginProofRequest>>,
    pause: Mutex<Option<nocterm_session::testing::Gate>>,
}
impl RecordingAuthorizer {
    pub(crate) fn requests(&self) -> Vec<LoginProofRequest> {
        self.requests.lock().unwrap().clone()
    }
}
impl SessionLoginAuthorizer for RecordingAuthorizer {
    fn authorize_login(
        &self,
        request: LoginProofRequest,
    ) -> BoxFuture<'static, Result<LoginSignature, LoginError>> {
        self.requests.lock().unwrap().push(request.clone());
        let pause = self.pause.lock().unwrap().clone();
        let inner = self.inner.clone();
        Box::pin(async move {
            if let Some(pause) = pause {
                pause.wait().await;
            }
            inner.authorize_login(request).await
        })
    }
}
pub(crate) struct LoginSetup {
    request: ConnectRequest,
    binding: LoginConsentBinding,
    pinned: PublicKeyText,
    device: Arc<DeviceLoginAuthorizer>,
    recording: Arc<RecordingAuthorizer>,
    authority: Arc<ScriptedLoginAuthority>,
    timer: Arc<ManualToolTimer>,
    store: FakeCredentialStore,
}
impl LoginSetup {
    pub(crate) fn new(sshd: &Sshd) -> Self {
        let request = sshd.request();
        let host =
            PublicKeyText::parse(&fs::read_to_string(sshd.root().join("host_key.pub")).unwrap())
                .unwrap();
        let material = KeyMaterial::import(
            &Secret::new(fs::read_to_string(sshd.root().join("plain/id_ed25519")).unwrap()),
            None,
        )
        .unwrap();
        let key = material.public_key();
        let store = FakeCredentialStore::default();
        let credential = store
            .insert_private_key("device only".into(), key.as_str(), material.to_secret())
            .unwrap();
        let fp = |b| Fingerprint::from_bytes([b; 32]);
        let scope = AccountScope::new(
            HubIdentity::new(NamespaceId::new("hub").unwrap(), fp(1)),
            AccountIdentity::new(NamespaceId::new("account").unwrap(), fp(2)),
        );
        let run = RunBinding::new(
            scope,
            RunnerId::from_bytes([3; 16]),
            RunnerKeyEpoch::new(1).unwrap(),
            CloudChatId::from_bytes([4; 16]),
            RunId::from_bytes([5; 16]),
            RunEpoch::new(1).unwrap(),
        );
        let target = TargetBinding::new(
            Fingerprint::from_bytes(host.fingerprint_bytes().unwrap()),
            fp(6),
            NamespaceId::new(&request.target.user).unwrap(),
            fp(7),
        );
        let authentication = AuthenticationBinding::new(
            Fingerprint::from_bytes(key.fingerprint_bytes().unwrap()),
            fp(8),
        );
        let authority = Arc::new(ScriptedLoginAuthority::new(
            run.clone(),
            target.clone(),
            authentication.clone(),
        ));
        let timer = Arc::new(ManualToolTimer::default());
        let device = Arc::new(DeviceLoginAuthorizer::new(
            Arc::new(store.clone()),
            credential,
            authority.clone(),
            timer.clone(),
        ));
        let binding = device
            .issue(LoginConsentRequest {
                run,
                target,
                authentication,
                user_key: key,
                pinned_host: host.clone(),
                expires: Timestamp::from_millis(60000),
            })
            .unwrap();
        let recording = Arc::new(RecordingAuthorizer {
            inner: device.clone(),
            requests: Mutex::default(),
            pause: Mutex::default(),
        });
        Self {
            request,
            binding,
            pinned: host,
            device,
            recording,
            authority,
            timer,
            store,
        }
    }
    pub(crate) fn request(&self) -> ConnectRequest {
        self.request.clone()
    }
    pub(crate) fn attempt(&self) -> SessionLoginAttempt {
        SessionLoginAttempt::new(
            self.binding.clone(),
            self.request.target.clone(),
            self.pinned.clone(),
            self.recording.clone(),
        )
        .unwrap()
    }
    pub(crate) fn requests(&self) -> Vec<LoginProofRequest> {
        self.recording.requests()
    }
    pub(crate) fn revoke_device(&self) {
        self.authority.revoke();
        self.store.set_available(false);
    }
    pub(crate) fn expire(&self) {
        self.timer.set(60000);
    }
    pub(crate) fn reads(&self) -> usize {
        self.store.reads()
    }
    pub(crate) fn authorize(
        &self,
        request: LoginProofRequest,
    ) -> BoxFuture<'static, Result<LoginSignature, LoginError>> {
        self.device.authorize_login(request)
    }
    pub(crate) fn suspend(&self, gate: nocterm_session::testing::Gate) {
        self.store.suspend(gate);
    }
    pub(crate) fn pause_before_device(&self) {
        *self.recording.pause.lock().unwrap() = Some(nocterm_session::testing::Gate::pending());
    }
    pub(crate) fn observer(&self) -> std::sync::Weak<RecordingAuthorizer> {
        Arc::downgrade(&self.recording)
    }
}
