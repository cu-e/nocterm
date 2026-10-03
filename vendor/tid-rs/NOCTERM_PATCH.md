# Local tid-rs 0.1.1 build patch

Upstream: <https://github.com/lightsing/tid-rs>, MIT.

Nocterm uses `LAContext::can_evaluate_policy` only for capability detection.
Actual authenticated key release is enforced by Security.framework's keychain
ACL, not by this library's process-local authentication result.

The upstream build script passes its Objective-C static archive through
`rustc-link-arg`, which does not propagate native-library metadata from an rlib
to dependent executables, and does not check compiler exit status. The local
patch uses the standard `cc` build helper for target-aware compilation, checked
failure and propagated static linking, and explicitly enables Objective-C blocks.
It also fixes an Objective-C `NSString` pointer declaration that prevented native
compilation. The original policy-evaluation API is unused and unmodified.

`KeychainContext` is a narrow additional safe API for cancellable Keychain queries.
It owns a +1 LAContext reference as CFType on the query's originating thread, and
provides a separately retained single-use `KeychainInvalidation` handle. Only that
handle implements Send; it deliberately does not implement Sync or expose raw
pointers, setters or authentication methods. NSObject retain/release ownership
is balanced independently for the query and watchdog. The handle only calls
Apple's idempotent `invalidate`, which terminates outstanding policy evaluation.
Objective-C lifecycle functions establish autorelease pools on Rust worker threads.
No native callback, detached query, or process-local success flag releases keys.

The application passes the retained context through the safe
`ItemSearchOptions::local_authentication_context` API in `security-framework`.
The existing `biometryCurrentSet` Keychain ACL remains the authentication boundary.
See [Apple's invalidate contract](https://developer.apple.com/documentation/localauthentication/lacontext/invalidate())
and [authentication context key](https://developer.apple.com/documentation/security/ksecuseauthenticationcontext).

Native macOS CI builds/links an adapter test executable, then exercises metadata
validation without requesting authentication. Linux cannot verify Apple linking.
