# Local tid-rs 0.1.1 build patch

Upstream: <https://github.com/lightsing/tid-rs>, MIT.

Nocterm only uses `LAContext::can_evaluate_policy` for capability detection.
Actual authenticated key release is enforced by Security.framework's keychain
ACL, not by this library's process-local authentication result.

The upstream build script passes its Objective-C static archive through
`rustc-link-arg`, which does not propagate native-library metadata from an rlib
to dependent executables, and does not check compiler exit status. The local
patch uses the standard `cc` build helper for target-aware compilation, checked
failure and propagated static linking, and explicitly enables Objective-C blocks.
It also fixes an Objective-C `NSString` pointer declaration that prevented native
compilation. The Rust API and authentication behavior are unchanged.

Native macOS CI builds/links an adapter test executable, then exercises metadata
validation without requesting authentication. Linux cannot verify Apple linking.
