//! Retained LAContext for a Security.framework query and its cancellation.
use core_foundation::base::{CFType, TCFType};
use std::{ffi::c_void, ptr::NonNull};

extern "C" {
    fn create_keychain_la_context() -> *mut c_void;
    fn retain_la_context(context: *mut c_void) -> *mut c_void;
    fn drop_la_context(context: *mut c_void);
    fn invalidate_la_context(context: *mut c_void);
}

/// A fresh context with biometric reuse disabled, owned by the querying thread.
/// It never evaluates a separate policy or releases a secret by itself.
pub struct KeychainContext {
    context: CFType,
}
impl KeychainContext {
    /// Creates a retained context, or returns None if native allocation fails.
    pub fn new() -> Option<Self> {
        // SAFETY: Objective-C alloc/init returns +1 or null. The bridge exposes
        // an NSObject-compatible reference, which CFType balances with CFRelease.
        let pointer = NonNull::new(unsafe { create_keychain_la_context() })?;
        Some(Self {
            context: unsafe { CFType::wrap_under_create_rule(pointer.as_ptr()) },
        })
    }

    /// Supplies the same retained context to kSecUseAuthenticationContext.
    pub fn authentication_context(&self) -> CFType {
        self.context.clone()
    }

    /// Creates an independently retained handle with only invalidate access.
    pub fn invalidation_handle(&self) -> KeychainInvalidation {
        // SAFETY: self owns a valid context. Retain cannot return null for a
        // valid object and keeps it alive until the handle's Drop runs.
        let pointer = unsafe { retain_la_context(self.context.as_CFTypeRef() as *mut c_void) };
        KeychainInvalidation {
            context: NonNull::new(pointer).expect("retaining a valid LAContext"),
        }
    }
}

/// An owning, single-use cross-thread cancellation handle.
/// No setters, policy evaluation, pointer access or context sharing are exposed.
pub struct KeychainInvalidation {
    context: NonNull<c_void>,
}
// SAFETY: NSObject retain/release supports transfer between threads. The only
// operation on this independently retained LAContext is Apple's invalidate,
// which terminates outstanding policy evaluation and is idempotent. We expose
// no general LAContext access and deliberately do not implement Sync.
unsafe impl Send for KeychainInvalidation {}
impl KeychainInvalidation {
    /// Terminates outstanding authentication on the querying context.
    pub fn invalidate(self) {
        // SAFETY: the handle owns its retain throughout the native call.
        unsafe { invalidate_la_context(self.context.as_ptr()) };
    }
}
impl Drop for KeychainInvalidation {
    fn drop(&mut self) {
        // SAFETY: balances precisely the retain used to create this handle.
        unsafe { drop_la_context(self.context.as_ptr()) };
    }
}
