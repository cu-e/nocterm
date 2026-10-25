# 0091. User plugin code runs in-process on wasmi behind the runtime port

Status: accepted

## Context

[ADR 0089](0089-user-plugin-contract.md) made a user plugin a declarative package and said that a
code runtime needs its own decision and would implement the same `PluginRuntime` port. A package
that can only fill templates cannot count down, keep a list or decide what to show. Code from an
unverified publisher must still reach nothing the user did not consent to: it runs in the
application's process, so the sandbox, the resource bounds and the failure behavior are part of the
contract, not of an implementation detail.

## Decision

**Engine.** A code plugin is a WebAssembly module run by wasmi 2.x in the application's process,
with `default-features = false` and the features `std`, `validate` and `portable-dispatch`: an
interpreter with no JIT and no native code generation, no text-format parser, SIMD or 64-bit
memories. The module is the whole sandbox: it gets no WASI and no host import at all. A
module that imports anything, lacks an export, or exports one with another type is refused.
Out-of-process engines, a JIT engine and a process sandbox were rejected for the first release:
a JIT engine adds a large dependency closure and a licence review; a process sandbox is
platform-specific and can be added behind the same port later.

**Limits.** Every call burns at most 4 000 000 units of fuel (instantiation and the start function
20 000 000), an instance holds at most 16 MiB of memory, at most 16 instances are live (at most
256 MiB), the input of a call is at most 256 KiB, the reply at most 256 KiB (the largest valid
view is about 200 KiB), and a call may return at most eight log lines; a reply with more is
malformed. The constants live in `nocterm-plugin-abi`, are part of the ABI contract and are listed
in the generated plugin reference.

**ABI v1.** A module exports `memory`, `nocterm_abi_version() -> i32` (1),
`nocterm_alloc(len) -> ptr` and `nocterm_call(ptr, len) -> i64`; the result packs the reply's
pointer in the high 32 bits and its length in the low 32 bits. Calls and replies are postcard
encodings of types owned by `nocterm-plugin-abi`. Those wire types embed the public types of
`nocterm-plugin-api` that a plugin sees (the view and its nodes, tones and icons, the effects and
events, the host errors and the capability values); only the observations and the setting values
are mirrored by types of the ABI crate. A reorder of those enums, or a variant added anywhere but
last, therefore changes the bytes a guest sees, and the golden samples below are the only guard
against it. A call is a render of one contribution or an event on it, with a snapshot of what the plugin was granted; the reply is a view
or effects, and log lines. The host reads guest memory only through bounds-checked slices, never
trusts a pointer or a length, and rejects trailing bytes and oversize replies. The ABI is
versioned by `ABI_VERSION`. `crates/plugin-abi/abi-golden.json` records the export names and the
hex bytes of canonical samples; `cargo xtask docs --check` fails when it is stale, and `cargo xtask
docs` refuses to change or remove a sample of the current version: that needs a new `ABI_VERSION`,
and appending a sample does not. A missing or unparsable golden file fails the same commands
instead of being recreated. The samples cover every variant of every enum that crosses the
boundary.

**Package.** A code package is one file: the module, with the manifest TOML in its custom section
`nocterm.manifest`. The first four bytes (`\0asm`) tell it from a declarative package, never the
file name. The manifest says `runtime = "wasm"`, a module whose manifest says anything else, and a TOML file
that says `wasm`, are refused. The fingerprint and the grant bind to the hash of the whole file, so
consent, update and the re-check at load work as for a declarative package. Before consent the host
only scans section headers (a bounded walk that never reads past the buffer) and parses the TOML;
the module is validated and compiled only when it is instantiated, after consent. A module is at
most 2 MiB and its manifest at most 256 KiB. The store keeps one file per plugin, `<id>.wasm` or
`<id>.toml`, and publishing one kind removes the other.

**Faults.** A trap, an exhausted fuel budget, a memory limit, a reply the host cannot decode
(including more log lines than allowed), a view or effects of a code plugin that the host refuses
to draw or apply (too many nodes, a key used twice, too many effects), and a module that fails to
compile or start count toward the three faults that disable a plugin in a session. Having no code
runtime composed and having no room for another live instance are not the plugin's fault and do
not count. A guest that traps, runs out of fuel or memory, or breaks the protocol is dropped and the next
call restarts it from the cached module, so a plugin loses its in-memory state with such a fault
and keeps only what it stored through `storage.*`. A view or effects set the host refuses counts
toward the three faults but keeps the guest and its state. A call the host cannot encode (a snapshot above the input limit) is
refused before the guest is touched, is not a fault, and keeps the guest and its state; the
largest snapshot the host can build is far below the limit.

**Effects.** Guest effects go through the same path as declarative ones: validated, brokered by the
host with its grants, gestures and rate limits, and applied after the current update ends. The
code runtime adds no way to act.

**Composition.** `nocterm-plugin-wasm` (adapter) implements `PluginRuntime`; the application
composes it with the declarative interpreter in `Runtimes`, which picks one by the manifest. Guest log lines go to an injected
`GuestLog`: the application logs them at `debug` for installed plugins. `plugin check` instantiates
and renders a code package like an installed one, validates each view as the host does, and drops
its log. The crate sits in the isolation policy with `wasmi` as its only direct external dependency and is compiled out with the rest of the
user-plugin crates.

## Consequences

- A call runs on the thread that asks, normally the UI thread. At 4 000 000 fuel a call costs on the
  order of a millisecond in a release build, and the host caps renders at 32 a second and disables a
  plugin after three faults. That bound is accepted for the preview; before user plugins are enabled
  in release builds the cost is measured on the reference machine and calls either move off the UI
  thread or the call fuel drops.
- wasmi is an interpreter. A plugin is slower than native code, which is the price of containing it
  without a platform sandbox. Dependencies of the development profile are built with optimizations,
  otherwise the tests of the runtime are too slow to use.
- A change of the wire types, the exports or the limits is a change of the ABI and shows up as a
  diff of `abi-golden.json` and the generated reference.
- A plugin author needs the module and the manifest section in one file; a helper that appends
  the section to a built module is possible without a change of the format.
