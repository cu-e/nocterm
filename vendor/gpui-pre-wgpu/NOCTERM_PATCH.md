# Nocterm Linux renderer patch

Upstream: gpui-pre-wgpu 0.3.7, Zed revision `1a28cff4b409169bac058bca40dfbfeb7621d19b`.
Crate archive SHA-256: `f0b02657b56b09140ce542f5e4434fb96d9fdaba0fd0035ba7dba13c171ddb9b`.
The archive's license and original shaders are preserved. Changes are Apache-2.0.

Renderer methods are mechanically split into child modules for resources,
pipelines, instances, paths, lifecycle, frame recording and headless rendering.
Cosmic text implementation is mechanically split into helpers, platform methods,
constructors and tests; production function bodies are unchanged. Test fixtures
are exact pinned upstream font bytes, with licenses and provenance in
`test-fixtures/`; they are not application assets. Tests use `wgpu::naga`.

Linux uses the original full renderer by default. Setting
`NOCTERM_EXPERIMENTAL_LINUX_RETAINED_RENDERER=1` explicitly enables the experimental
bounded persistent same-format image at surface creation/replacement. Other
values leave full rendering enabled; there are no per-frame environment reads.
Supported surfaces
must advertise COPY_DST and matching RGBA/BGRA render/copy capabilities.
A partial frame loads the retained target, overwrites its damaged rectangle
without blending, and replays intersecting batches in original order. Reopened
path passes restore the damage scissor. Unchanged frames still acquire, copy,
submit and present the complete image for compositor callbacks.

Atlas uploads are flushed before comparing their content revision; successful
uploads, removals, clearing and recovery invalidate retention. Resize, alpha,
subpixel layout and GPU/surface errors invalidate it too. Scoped validation,
internal and out-of-memory checks validate retained allocation before use;
unsupported capabilities, memory limits or allocation errors use original full
rendering. Unsupported scene coverage or exhausted snapshot budgets also
use original direct full rendering before allocating a retained image. Supported
whole-viewport damage uses the original attachment clear on the retained image.
Each owned retained texture is bounded at 64 MiB in addition to adapter limits;
in-flight GPU commands can retain previously submitted resources until completion.

The original headless full renderer remains the independent pixel oracle.
Linux scroll-copy is not enabled. Other WGPU platforms and Windows retain their
original rendering paths. Actual Linux CPU/UI measurements are required before
shipping; no speedup is claimed by this patch description. The measured V2
same-binary CPU comparison passed the 5% regression budget, but moving scroll
still cost about 3.3–3.8% more CPU per event on the tested Radeon/Vulkan system.
Retention therefore remains opt-in. The common Div scroll-boundary fix remains
active with the default full renderer.

Verification commands for the fork:

```sh
cargo test --manifest-path vendor/gpui-pre-wgpu/Cargo.toml --lib --features test-support --locked \
  --config "patch.crates-io.gpui-pre.path=\"$PWD/vendor/gpui-pre\""
cargo clippy --manifest-path vendor/gpui-pre-wgpu/Cargo.toml --all-targets --features test-support --locked \
  --config "patch.crates-io.gpui-pre.path=\"$PWD/vendor/gpui-pre\"" -- -D warnings
```

The standalone fork lock records its test dependency set separately from the
application lock. The CLI patch selects Nocterm's shared scene damage API without
changing the upstream fork dependency declaration. Shipping builds use the
application lock.

The normalized manifest restores Criterion 0.8.2 with `html_reports`, matching
the [pinned upstream workspace](https://github.com/zed-industries/zed/blob/1a28cff4b409169bac058bca40dfbfeb7621d19b/Cargo.toml).
The packaged `layout_line` benchmark uses the same licensed local font fixtures
as the text tests, so standalone all-target checks do not need an upstream Zed
checkout. These dependencies and fixtures are for tests and benchmarks; the
original `Cargo.toml.orig` remains intact.

Native tests assert whole-image equality against original full rendering,
including alpha, clipping, transformed fallback, path pass scissor restoration,
atlas mutations, target formats, allocation scopes and poisoned presentation
destinations. They complement actual compositor UI/performance runs.
GPU test fixtures share a guard from before native instance creation until after
renderer destruction. Independent concurrent fixture lifetimes reproduced a
Vulkan loader crash in debug-object naming; sharing an instance instead exposed
concurrent GL adapter enumeration errors. The guard keeps these driver lifetimes
exclusive while the default test harness still runs all tests, including shader
and policy checks, in parallel. Each GPU test retains its own instance, device,
queue and error state, and all pixel assertions are unchanged. Shipping renderer
construction and driver settings are unchanged.

Linux captures into a bounded spare snapshot and compares with a per-call exact
record-pair memo. The committed snapshot and spare rotate only after successful
submission with the same atlas revision. Renderer failure, atlas races, lifecycle
invalidation and viewport shape changes release both snapshots and the memo.
Capacity budgets and derived reserved-byte limits live in the shared GPUI damage
module; the original capture/comparison APIs and Windows callers are unchanged.
