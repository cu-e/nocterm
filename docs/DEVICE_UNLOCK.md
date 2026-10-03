# Device unlock

Settings → Vault detects the operating system's authentication capability. First
unlock with the master password, then choose Enable. Later, lock the vault and use
Unlock with Fingerprint, Touch ID or Windows Hello. Disable removes the local
registration. The master password remains available on every machine.

The portable `vault.bin` envelope is unchanged. A bounded private
`vault.device-unlock` companion stores a versioned vault ID, current KDF salt and
public native metadata/ciphertext. It never stores a master password, a plaintext
wrapping key, or a reusable Windows Hello signature. Password rotation changes the
salt and cryptographically invalidates old registrations before native cleanup.
Moving only `vault.bin` preserves password access; device unlock must be enabled
separately on the destination. There is no cloud synchronization of device keys.

## Platform behavior

| Platform | Authenticated key release | Lifetime and requirements |
| --- | --- | --- |
| macOS | Security.framework Data Protection Keychain, `biometryCurrentSet` | Touch ID enrollment and a signed application able to access the protected keychain. The item is available only while unlocked, is device-local and does not synchronize. Changing enrolled fingers invalidates access. |
| Windows | Windows Hello RSA credential signs a registration-specific challenge; HKDF-SHA256 derives the key for authenticated XChaCha20-Poly1305 wrapping | Windows Hello enrollment. Windows chooses fingerprint, face or PIN; this API cannot require fingerprint exclusively. Removing the Hello credential invalidates access. |
| Linux | Root-owned Nocterm broker verifies the user's enrolled finger with root-owned fprintd before releasing a session key | Optional system service below. After each Nocterm start or broker restart, enable again after password unlock. Keys are memory-only, connection-bound and expire after eight hours. |

macOS capability detection uses the small safe `tid-rs` LocalAuthentication bridge;
its preflight result only controls availability. A narrow local bridge also owns
the query's retained `LAContext` and a single-use invalidation handle. The actual
secret is released by the Keychain access-control policy through the maintained
`security-framework` crate. The bridge never receives vault keys or evaluates a
separate authentication-success gate. Windows uses Microsoft's `windows`
bindings and Linux uses `zbus` and `nix`. Workspace crates forbid unsafe Rust;
the small vendored bridge isolates reviewed native reference/lifecycle operations.

Windows packaged applications have stronger credential namespace isolation than
unpackaged executables. Namespace validation prevents accidental cross-vault
deletion; it does not establish isolation from arbitrary software already running
as the same Windows user. Native Windows GUI prompt placement and macOS signing
must be checked on those platforms before distribution. Capability detection
never proves that an authentication scan succeeded.

## Optional Linux installation

Linux Secret Service does not promise biometric access control on each read. A
successful fprintd UI check followed by an ordinary same-user keyring read would
leave a bypass. Nocterm therefore uses a separately privileged, optional broker.
The desktop application never installs it or changes PAM/polkit automatically.

Build a reviewed release, then explicitly install as administrator:

```sh
cargo build --release -p nocterm-vault-broker
sudo scripts/install-vault-broker.sh
```

The installer installs a root-owned binary, system-D-Bus policy and hardened
systemd unit, then starts that unit. Install fprintd using your distribution and
enroll a finger in the operating system settings. The application uses the actual
system-bus socket and ignores user-controlled system-bus address overrides.
Passive detection lists supported devices and enrolled fingers without initiating
a scan. If hardware, enrollment or broker is missing, password unlock still works.

To uninstall, stop and disable `nocterm-vault-broker.service`, remove its unit,
`/etc/dbus-1/system.d/dev.nocterm.VaultBroker1.conf` and
`/usr/local/libexec/nocterm-vault-broker`, then reload systemd. Stopping the broker
immediately erases its in-memory registrations. No persistent broker secret file
or setuid helper is installed.

## Security boundaries

Within the desktop process, password-derived wrapping keys stay on the vault
worker. Native providers store and release them under their platform protection.
Native release
must match the current vault ID and salt, and the vault file is compared again
after any authentication prompt. Lock, cancellation and leaving the Vault page
invalidate an epoch; a late successful native response cannot reopen the vault.
macOS passes its fresh context to `kSecUseAuthenticationContext`. A scoped, joined
watchdog invalidates that context on cancellation or after 30 seconds, terminating
the outstanding policy evaluation and freeing the vault worker for password
fallback. There is no detached native query; the context and its invalidation
handle hold independent native retains until query and watchdog both finish.
The native bridge uses autorelease pools on background threads. Real Touch ID
prompt cancellation still requires native macOS hardware verification.
The portable authenticated envelope also rejects a wrong native key. Native
registration is cleaned up if companion-file persistence fails.

Linux registrations bind the bus-authenticated UID, initiating unique connection,
vault binding and random token. Another connection under the same UID cannot read,
cancel or delete them. fprintd's unique owner must belong to root; only its own
`verify-match` completed signal releases a key. Failed matches, reader contention,
disconnect, timeout and cancellation refuse release. Keys are zeroized on drop,
and the broker limits total and per-user registrations and its systemd resources.
Expiry uses a fixed interval independent of bus traffic, so frequent connection
events cannot postpone erasure of expired registrations.
The broker claims the reader before subscribing to verification events, then
starts the new scan. A completion from the previous claim cannot authorize the
new request; a synchronous completion from the new scan is still observed.

This protects against reading device metadata and bypassing a process-local
authentication flag. It does not protect an unlocked process from a debugger,
root/admin compromise, kernel compromise, swap or physical memory acquisition.
OS IPC serialization may temporarily contain plaintext keys; zeroizing owned
application buffers cannot erase every OS/library copy. Mac/Windows OS credentials
and Linux session-key lifetimes differ deliberately and are shown in the UI.

Automated tests exercise envelope integrity, binding, rotation, failed writes,
epoch cancellation and an isolated fake-fprintd D-Bus protocol. They do not claim
real fingerprint recognition. Native CI compiles/links macOS and Windows adapters
and runs tests without requesting biometric input. Hardware tests require a person
and are recorded separately; the system broker is not installed by tests.

## Verification recorded for this implementation

On Linux, the required workspace build, formatting, Clippy, documentation and
architecture checks passed. The final workspace suite passed 375 tests, including 24
real local SSH/SFTP tests; the vendored terminal suite passed 135 tests and native
menu tests passed two. The regression for a previous claim's successful signal
first reproduced an unauthorized key release, then passed after the sequencing
fix. After limiting the broker runtime to two worker threads, its build, Clippy,
formatting and five tests passed again.

Follow-up regressions also reproduced both review findings against the old code:
continuous private-bus events prevented expiry cleanup, and cancellation without
native invalidation blocked password fallback on the single vault worker. Both
passed with the periodic expiry timer and joined cancellation runner. The final
broker suite passed six tests. These cancellation tests exercise a blocking test
query; they do not establish real Touch ID prompt behavior.

Windows MSVC cross-target check and Clippy passed; Windows runtime and native
macOS build/runtime were not exercised on this Linux host. A native Wayland
startup smoke test mapped a separate Nocterm window using temporary application
directories. Passive detection found an enrolled fingerprint reader and reported
the missing optional broker correctly. No actual authentication scan was requested,
and no system service or authentication policy was installed or changed.

The dependency audit still reports the existing unpatched transitive RSA advisory
[RUSTSEC-2023-0071](https://rustsec.org/advisories/RUSTSEC-2023-0071.html) and five
informational maintenance notices. No advisory exclusions were added; this
dependency audit is not a green security certification.

Relevant platform contracts: [Apple biometric keychain policy](https://developer.apple.com/documentation/security/secaccesscontrolcreateflags/biometrycurrentset),
[Windows Hello signing](https://learn.microsoft.com/en-us/windows/apps/develop/security/windows-hello),
[fprintd verification](https://fprint.freedesktop.org/fprintd-dev/Device.html),
[Secret Service locking](https://specifications.freedesktop.org/secret-service/latest/ch10.html).
