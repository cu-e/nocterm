//! Explicit original-binding guard for native login tests, never authentication.
use crate::{
    authority::{AuthenticationBinding, RunBinding, TargetBinding},
    login::{LoginConsentAuthority, LoginConsentBinding, LoginError},
};
use std::sync::atomic::{AtomicBool, Ordering};
pub struct ScriptedLoginAuthority {
    run: RunBinding,
    target: TargetBinding,
    authentication: AuthenticationBinding,
    revoked: AtomicBool,
}
impl ScriptedLoginAuthority {
    pub fn new(
        run: RunBinding,
        target: TargetBinding,
        authentication: AuthenticationBinding,
    ) -> Self {
        Self {
            run,
            target,
            authentication,
            revoked: AtomicBool::new(false),
        }
    }
    pub fn revoke(&self) {
        self.revoked.store(true, Ordering::Release);
    }
}
impl LoginConsentAuthority for ScriptedLoginAuthority {
    fn check(&self, binding: &LoginConsentBinding) -> Result<(), LoginError> {
        if self.revoked.load(Ordering::Acquire) {
            return Err(LoginError::Revoked);
        }
        if binding.run() != &self.run
            || binding.target() != &self.target
            || binding.authentication() != &self.authentication
        {
            return Err(LoginError::Mismatch);
        }
        Ok(())
    }
}
