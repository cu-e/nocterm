# Architecture and security audit

Scope: the Rust workspace and composition root at `6b51496`, followed by the
changes on `refactor/architecture-security-audit`. This is a source audit with
regression tests, not a claim that every possible defect has been excluded.
The device-unlock feature is developed and reviewed separately.

## Plan and boundaries

The architect stage examined trust boundaries, storage, asynchronous work,
resource budgets, feature dependencies and reusable UI contracts. The user
explicitly authorized implementation, verification, review, local commits and
merges into `master` without an additional confirmation. Release-owned files
and unrelated local data are outside this work. No screenshots are used.

Implement confirmed findings in Core/SSH, Connections/Explorer/Transfers,
Settings/UI, Terminal and xtask. Keep domain crates free of GUI dependencies;
compose adapters in the application. Verify failure paths and concurrency,
then require a read-only reviewer with no blocking findings before merging.

## Findings and corrections

| Finding | Correction | Evidence to exercise |
| --- | --- | --- |
| Predictable TOML temporary files could follow a pre-existing symlink or collide with another writer. The invalid-file backup had the same symlink problem. | Private, uniquely created files and atomic publication; backup publication replaces a link instead of following it. Intentional configuration symlinks still resolve normally. | Victim-file preservation, parallel writers, backup-link regression, round-trip comments. |
| SSH trust lookup ignored revoked records and host patterns; file-open failures could look like an unknown host. | A bounded trust-store snapshot loads before the handshake; malformed records and unreadable existing files fail closed. Revocation precedes ordinary trust. Hashed, wildcard, negated, alias and non-default-port names are matched. | Unit fixtures and real OpenSSH connections reject revoked/malformed trust before asking for credentials. |
| Private-key input was unbounded and copied through ordinary strings; Unix special files could block opening. | Bounded regular-file readers, regular-file inode checks, shared zeroizing key buffers. | Oversized and special-file rejection; normal keys and configuration symlinks remain supported. |
| SFTP listing and upload-handle registration had no aggregate budget; link metadata requests ran serially. | Entry/name/page limits, invalid-name rejection before joining paths, 128 upload handles, an ordered window of eight link STAT requests. | Protocol fixtures check pipelining, ordering and CLOSE on rejected listings. |
| Profile/recent/settings fsync ran on the UI thread. Concurrent forms could silently replace newer state. | Bounded ordered background persistence. Profiles validate their original snapshot; settings validate a revision again when dequeued. Publish durable settings/profiles only after success. Recent history coalesces independently of SSH opening. | Concurrent drafts, failed writes, editor preservation and serialized persistence tests. |
| Explorer sorting repeatedly folded strings inside comparisons on the UI thread. Recursive scans and upload discovery lacked uniform resource bounds. | Sort/filter off the UI thread, precomputed keys, cancellable bounded listing/stat traversal, shared pinned-directory helpers on Unix, source/depth/visit/history limits for transfers. | Admission boundaries, deeply nested trees, cancellation and real filesystem traversal fixtures. |
| A remote process could grow one cell's combining sequence without limit; title-stack pushes also amplified a large title. | Cap combining sequences at 64 marks before allocation/copy, titles at 4096 UTF-8 bytes, hyperlink URI/ID at 4096/1024 bytes. Reject oversized metadata without partial application. | Independent reproducer, 100,000-mark recovery, ordinary/wide Unicode, title-stack and hyperlink regressions; preserve a 5000-mark search workload across bounded clusters. |
| Combining characters were present in emulator frames but absent from the renderer's text. | Include the marks with the base cell's style and UTF-8 run lengths; preserve hidden-cell behavior and grid positioning. | Unicode combining and hidden-text regressions. |
| Unterminated OSC sequences could grow VTE's std-mode buffer without limit. | Bound OSC ingress to 1 MiB before forwarding it to VTE. Discard oversized commands completely; cancellation must not dispatch a truncated prefix. Ordinary output uses borrowed slices. | Fragmented UTF-8/control sequences, exact boundaries, oversized title/clipboard recovery and differential native-parser parity tests. |
| OSC 52 from a background or untrusted terminal could replace the desktop clipboard. | Denied by default; optional writes require focus on the actual terminal screen in the active application window. Clipboard reads remain denied. | Native GPUI tests exercise default denial, opt-in, search-input focus and blurred terminals. |
| Architecture checks only inspected declared internal edges. | Also reject runtime/build GUI dependencies in foundation/domain/adapters and internal dependencies without a declared layer. Dev-only GUI test support is allowed. | Negative dependency fixtures and generated dependency checks. |

## Resource budgets

| Area | Bound |
| --- | --- |
| Settings / profile writers | 16 / 32 pending jobs plus one running job each |
| Profile mutations | 64 KiB candidate metadata |
| Explorer listing | 100,000 entries, 8 MiB aggregate names |
| Local statistics / transfer discovery | 256 directory levels, 1,000,000 visited nodes |
| Transfer admission | 4096 sources, 1 MiB total paths, 16 KiB per path/target |
| Pending download listings across the DFS stack | 100,000 entries, 8 MiB names |
| Pending remote deletion paths | 16 MiB, in addition to depth/entry limits |
| Retained transfer history | 64 batches; active jobs are retained |
| SSH trust input | 4 MiB per file, 64 KiB per line, 16,384 records total |
| SFTP wire packet / listing | 256 KiB upstream packet cap; 100,000 entries and 16 MiB aggregate names |
| SFTP handles / link metadata | 128 upload handles; eight concurrent link STAT requests |
| OSC ingress | 1 MiB per command, before VTE payload allocation |
| Terminal cell/title metadata | 64 combining marks per cell; 4096-byte titles, at most 16 MiB title-stack text; hyperlink URI/ID 4096/1024 bytes |

## Retained design constraints

The existing bounded session channels, transport-independent filesystem API,
native dock pane tree, settings schema and separate style tokens remain the
extension seams. Features communicate through Workspace contracts and the
composition root, not by importing sibling features. No speculative plugin,
account, cloud or IDE framework is introduced.

Persistence queues keep the interface responsive, but filesystem sync can
still block an operating-system worker. The GPUI dependency allows native quit
handlers 200 ms; drains are bounded best effort and log unfinished saves.
An explicit successful save is the durability signal. Abrupt process termination
cannot guarantee queued persistence.

Transfer batches currently run sequentially with four file workers per batch.
A stalled batch may delay a later batch on another host. A fair scheduler needs
measured workloads and a separate policy decision; this audit makes no untested
throughput claim. Retained job history is bounded without evicting active work. The Connections
sidebar still constructs all visible rows; no FPS benchmark was performed.

Unix local scans/operations pin directories and reject symlink traversal where
specified. Windows path-based operations retain the documented ancestor-race
limitation. SFTP v3 operations cannot pin remote filesystem identity across
separate path requests. Upstream SFTP rejects wire packets over 256 KiB before allocation. Parsing
inside that packet cap precedes our aggregate listing budgets. Host certificate authority records are not treated as
ordinary trusted host keys; SSH host-certificate validation is not implemented.

## Dependency evidence and remaining advisory

`cargo audit --json` with cargo-audit 0.22.2 and the RustSec database from
2026-10-02 reports `RUSTSEC-2023-0071` for transitive `rsa 0.10.0-rc.18`, plus
informational unmaintained-package notices. The audit command is **not green**.
No ignore entry, crypto downgrade or weakened check was added.

RustSec lists no patched version; its tracking issue concerns timing behavior
in RSA padding. See the [advisory](https://rustsec.org/advisories/RUSTSEC-2023-0071.html)
and [upstream tracking](https://github.com/RustCrypto/RSA/issues/626).
The examined Nocterm SSH paths use RSA signing and public-key verification;
an attacker-controlled PKCS#1 v1.5 decryption oracle was not identified.
That reachability observation does not prove all RSA operations safe. Follow
upstream fixes and re-run dependency auditing when upgrading russh/ssh-key.

## Verification

The implementation includes targeted storage, trust-store, SFTP protocol,
concurrency, resource-budget and terminal regressions. Required final gates are
`cargo fmt --all --check`, locked workspace build and tests, all-target clippy
with warnings denied, mandatory local OpenSSH/SFTP tests, generated-doc freshness,
architecture boundaries and rustdoc with warnings denied. Exact final results
are recorded in the completion report. Native macOS/Windows execution and
fingerprint interaction cannot be established by Linux tests.

Initial Linux audit gates: **348 workspace tests, 0 failed, 0 ignored**, and **24
mandatory real OpenSSH/SFTP tests**. Formatting, locked all-target workspace
build/clippy, generated-document freshness, architecture checks and rustdoc with
warnings denied passed. The independent tester added inactive-window OSC 52 and
pre-I/O download admission regressions. A temporary-config native Wayland smoke
mapped a 948 × 562 window and selected the AMD hardware Vulkan adapter; no image
was captured and existing application sessions were left running. This smoke
establishes window creation/render startup, not interactive behavior of every UI
control. Independent source review is the final merge gate.

First independent review requested the cell-metadata cap and separate ownership
of profile/history persistence errors. Both findings were reproduced and fixed;
successful profile writes now leave an outstanding history error visible. The
patched engine retains its original 132 tests and adds three regressions.
Standalone engine clippy also exposed three original Unix error-propagation
style warnings; equivalent `?` propagation fixed them without disabling a lint.

After review corrections, the same required gates passed again with **353
workspace tests, 0 failed, 0 ignored**, including the 24 OpenSSH/SFTP tests.
The standalone patched engine passed **135 tests** and all-target clippy with
warnings denied. A separate optional standalone vendor format check differs
from default stable rustfmt because the copied upstream sources retain their
original style; the required workspace format check passes. No formatting
check was weakened and no mass vendor reformat was introduced.
