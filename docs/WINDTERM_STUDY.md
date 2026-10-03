# WindTerm study and focused integration

Scope authorized by the user's instruction to study WindTerm and integrate the
best session/shell/transfer capabilities. No screenshots or image inspection are
used for this stage. Existing workspace, vault and Explorer work remains in place.

## Evidence

Studied repository revision `a8336c1d1e3dd1a981500dae8ef9eb42066de95f`.
WindTerm explicitly publishes only part of its source. Its complete terminal/text
engine, high-level transfer scheduler and present application vault format cannot
be reconstructed from the available code; benchmark numbers do not establish an
algorithm or comparable Nocterm throughput.

- [Published components](https://github.com/kingToolbox/WindTerm/blob/a8336c1d1e3dd1a981500dae8ef9eb42066de95f/src/README.md): PTY, libssh modifications, utility classes, widgets.
- [Feature list](https://github.com/kingToolbox/WindTerm): session proxies, terminal families, logging, shell and file management.
- [SFTP upload](https://kingtoolbox.github.io/2020/06/21/transfer-upload/) and [download](https://kingtoolbox.github.io/2020/06/21/transfer-download/): multiple files and folders in each direction.
- [SFTP performance](https://kingtoolbox.github.io/2023/11/15/benchmark-sftp-transfer/): large files and many small files are explicit workloads; its implementation is not published.
- [Manual logging](https://kingtoolbox.github.io/2020/10/16/manual_logging/) and [automatic logging](https://kingtoolbox.github.io/2020/10/16/automated_logging/): per-session lifecycle.
- [Published cryptographic utility](https://github.com/kingToolbox/WindTerm/blob/a8336c1d1e3dd1a981500dae8ef9eb42066de95f/src/Utility/Cryptographic.cpp): PBKDF2/SHA3-512 and AES-CBC utility. This does not establish the complete current application's protections. Nocterm keeps its own versioned Argon2id/XChaCha20-Poly1305 authenticated vault.

No WindTerm code is copied into the application. We adopt useful behavior while
retaining Rust adapters, GPUI Kit controls and the current dependency boundaries.

## Selected behavior and implementation plan

1. Keep full regression gates for the existing dock/PTY/vault/uploads. Fix queue
   rendering reentrancy and use cached matching open-session context for explicit
   Retry, independently of which utility tab is focused.
2. Add typed inheritable session options: TERM, charset, proxy and logging.
   Saved profiles also gain a description. Native selects offer familiar TERM
   values plus a custom value; each profile may inherit or override the default.
   TERM announces capabilities to the server; selecting a legacy name does not
   claim a separate complete VT520 emulator.
3. Keep charset conversion at the terminal boundary, with persistent incremental
   decoding across network chunks. UTF-8 is default; Windows-1251, KOI8-R,
   Windows-1252 and GBK are the focused legacy choices. Encode text input/paste
   separately; key/mouse/control responses and SFTP binary data remain protocol
   bytes. Unrepresentable input must produce a visible error, not silent loss.
4. Add explicit direct/HTTP CONNECT/SOCKS5 routes in the SSH adapter. SOCKS5 can
   resolve DNS at the proxy. Bound handshakes and headers; preserve target host-key
   verification and cancellation. This first subset uses proxies without proxy
   authentication; do not persist plaintext proxy passwords or run arbitrary
   external ProxyCommand strings. Jump servers are outside this stage.
5. Add output-only session logs: opt-in automatic start and manual lifecycle,
   private unique files, bounded writer queues and file limits, visible errors.
   Never log authentication prompt answers or the terminal input stream. Remote
   output may itself include confidential text, so enabled logging must be visible.
6. Complete bidirectional SFTP streaming: bounded download readers, same retained
   transfer queue, progress/cancel/retry/collision rules, and remote files/folders
   dragged to Local or downloaded with an explicit action. Local temporary files
   publish atomically. Reject unsafe remote names and traversal; skip links and
   special files; retain owned handle/temp cleanup. Evaluate bounded pipelining
   only where offset, acknowledgement and cancellation semantics stay correct.
7. Regenerate schemas/action/dependency references and update architecture/use docs.
   Verify code and behaviors with unit/GPUI tests and real private OpenSSH servers;
   finish with an independent read-only review. No screenshots, commits or pushes.

Files: settings schema/forms, connection models/editor/storage, session contract,
Workspace SessionSpec; terminal codec/logging and VT input boundaries; SSH proxy
and SFTP adapters; transfers service and Explorer UI; generated docs and CI.

Highest review risks: codec fragmentation versus escape/control bytes, shell
startup/quoting and process cleanup, logging back-pressure and secret boundaries,
proxy DNS/host-key/timeout behavior, download path safety, atomic collision handling
and disconnect/cancel cleanup. Existing `read_dir` returns a directory Vec, so
listing a single very large directory remains an explicit memory limitation even
when transfer queues and file contents are bounded.

Tests: option inheritance and old-profile migration, selects/save/reconnect,
fragmented Cyrillic/GBK and text encoding failures, unchanged control sequences,
private capped output logs, fake HTTP/SOCKS protocol edge cases plus tunneled SSH,
recursive downloads/empty folders/unsafe names/links/partial EOF/collisions,
many small files/cancellation and dropped handles against real SFTP. Run full
fmt/build/clippy/tests, vendor emulator tests, generated docs, architecture and
rustdoc checks. Linux verification does not establish Windows/macOS execution.
