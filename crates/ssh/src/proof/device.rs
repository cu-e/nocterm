//! Device-side authorization: finite consent is consumed before original-store key load.
use futures::future::BoxFuture;
use nocterm_cloud_model::{
    authority::{AuthenticationBinding, RunBinding, TargetBinding},
    identity::{AttemptId, Timestamp},
    login::{
        LoginConsentAuthority, LoginConsentBinding, LoginConsentLedger, LoginConsentWindow,
        LoginError, LoginProofRequest, SessionLoginAuthorizer,
    },
    tool::ToolTimer,
};
use nocterm_session::{CredentialId, CredentialStore, SecretRequest};
use nocterm_ssh_keys::{
    PublicKeyText,
    login::{LoginSignature, LoginSigningKey},
};
use std::sync::{Arc, Mutex};

/// User-approved exact facts. Calling `issue` is a device composition action;
/// the remote `SessionLoginAuthorizer` exposes no consent-creation operation.
pub struct LoginConsentRequest {
    pub run: RunBinding,
    pub target: TargetBinding,
    pub authentication: AuthenticationBinding,
    pub user_key: PublicKeyText,
    pub pinned_host: PublicKeyText,
    pub expires: Timestamp,
}
pub struct DeviceLoginAuthorizer {
    store: Arc<dyn CredentialStore>,
    credential: CredentialId,
    authority: Arc<dyn LoginConsentAuthority>,
    timer: Arc<dyn ToolTimer>,
    ledger: Arc<Mutex<LoginConsentLedger>>,
}
impl DeviceLoginAuthorizer {
    pub fn new(
        store: Arc<dyn CredentialStore>,
        credential: CredentialId,
        authority: Arc<dyn LoginConsentAuthority>,
        timer: Arc<dyn ToolTimer>,
    ) -> Self {
        let ledger = LoginConsentLedger::new(timer.now());
        Self {
            store,
            credential,
            authority,
            timer,
            ledger: Arc::new(Mutex::new(ledger)),
        }
    }
    pub fn issue(&self, request: LoginConsentRequest) -> Result<LoginConsentBinding, LoginError> {
        let mut attempt = [0; 16];
        let mut nonce = [0; 32];
        getrandom::fill(&mut attempt).map_err(LoginError::backend)?;
        getrandom::fill(&mut nonce).map_err(LoginError::backend)?;
        let now = self.timer.now();
        let binding = LoginConsentBinding::new(
            request.run,
            request.target,
            request.authentication,
            AttemptId::from_bytes(attempt),
            nonce,
            LoginConsentWindow::new(now, request.expires)?,
            request.user_key,
        )?;
        self.authority.check(&binding)?;
        self.ledger
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .issue(binding.clone(), request.pinned_host, now)?;
        Ok(binding)
    }
}
impl SessionLoginAuthorizer for DeviceLoginAuthorizer {
    fn authorize_login(
        &self,
        request: LoginProofRequest,
    ) -> BoxFuture<'static, Result<LoginSignature, LoginError>> {
        let authority = self.authority.clone();
        let timer = self.timer.clone();
        let ledger = self.ledger.clone();
        let store = self.store.clone();
        let credential = self.credential;
        Box::pin(async move {
            authority.check(request.binding())?;
            ledger
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .consume(&request, timer.now())?;
            let query = SecretRequest::PrivateKey {
                credential,
                public_key: request.binding().user_key().as_str().to_owned(),
            };
            let secret = store
                .get(credential, &query)
                .await
                .map_err(|error| LoginError::Credential(nocterm_core::SharedCause::new(error)))?;
            authority.check(request.binding())?;
            ledger
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .check_consumed(&request, timer.now())?;
            let key = LoginSigningKey::import(&secret, request.binding().user_key())?;
            authority.check(request.binding())?;
            let signature = key.sign_login(request.login())?;
            authority.check(request.binding())?;
            ledger
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .check_consumed(&request, timer.now())?;
            Ok(signature)
        })
    }
}
