# Nocterm Windows renderer patch

Upstream: `gpui-pre-windows` 0.3.7, crates.io archive SHA-256
`f05592a6f9e3e6bb7a9f2a0d4779271cf747a948db1c07b02448de777ffff8f3`.
The upstream Apache-2.0 license, manifests, shaders and build script are preserved.
Modified source files carry an Apache-2.0 modification notice.

The renderer owns a persistent BGRA render target. Scene snapshots from GPUI
identify changed device-pixel regions. Rectangular ClearView and a scissored
replay of the original batch order update those pixels; batches whose shader
coverage is outside the damage are skipped. Path batches preserve their original
MSAA and compositing semantics. Presentation still copies the complete retained
image into the existing swap chain and performs the required Present call.

First frames, background-appearance changes, resize, device recovery, unsupported
scene coverage, atlas texture changes and failed draws/presentation invalidate
retention. Devices without rectangular ClearView use full rendering. Failure to
allocate the extra retained target falls back to the original swap-chain target
and full rendering. Every successful atlas upload invalidates retention because
linear filtering can sample neighboring tile texels, including newly inserted
tiles. Cached glyph lookup performs no upload and does not invalidate retention.
Every rendering pass explicitly sets its rasterizer state and scissor on the
shared device context.

Renderer responsibilities were extracted into child modules to keep each file
below 800 lines. Existing shader compilation and driver probing are unchanged.
`render_to_image` retains the original full rendering path as a pixel oracle;
native renderer tests compare incremental pixels with that oracle on the same
atlas across cursor, clipping, alpha, paths, sprites and resource transitions.

Partial rendering is enabled on capable hardware and software adapters. Prior
native profiling established the WARP rendering bottleneck. The performance of
this partial-rendering change still requires a native benchmark; hardware-GPU
performance remains unmeasured.

## Verified scroll copies

When GPUI proves a `SceneScrollPlan`, the renderer preserves the source region
in a separate scratch texture before copying any destination row runs. This
prevents overlapping up/down copies from destroying source pixels. Unproven
rows and ordinary damage outside the candidate region are cleared and replayed
in disjoint strips; scene instance buffers are uploaded once for the entire
plan. Each strip restores the rasterizer, scissor and targets normally.
No original shader, alpha-composition or atlas-sampling semantics are replaced.

Scratch allocation is lazy. Allocation failure falls back to ordinary damage
without altering target pixels and disables repeated allocation attempts until
resources are recreated. Resize and device recovery discard scratch resources.
The same atlas-content revision, background appearance and rectangular ClearView
requirements guard both retained damage and scroll copies.

Native scroll tests compare real scratch copies against full rasterization for
mono/subpixel glyph patterns, dense three/six-line shifts, integer up/down overlap,
1080×680 and odd 1079×679 viewports, guarded fractional opaque backdrops, stationary alpha selection
and cursors, clipped glyph rows, resize, atlas mutations and fallback paths.
A poisoned swap-chain destination verifies the actual copied output before
Present can rotate or discard its contents. Native validation and performance
results belong to the build artifacts, not to this source provenance document.

Partial damage and scroll repairs pack mono/subpixel instances separately for
each actual redraw rectangle, preserving original texture and blend order in
each strip. Overlapping repairs duplicate their selected instances in the packed
stream. Each glyph-kind stream uploads once with WRITE_DISCARD before the first
strip; per-strip monotonic prefix counts include the strip's absolute base and
map original batch endpoints onto compact buffer offsets. No instance buffer
maps occur inside strip draws. Preparation bounds original instance count and
combined packed instance count to 65,536, rectangles to 64, and rectangle
comparisons/prefix entries to 1,000,000 plus one endpoint per strip/kind. If
limits are exceeded, preparation restores original whole-scene buffers and batch
offsets before any target write. Unknown geometry/transforms remain conservatively
selected. Full rendering also restores original buffers and offsets. Native pixel
tests cover disjoint repairs, empty preceding/intermediate strips, nonzero base
offsets, overlapping repairs with distinct alpha colors, multiple atlas textures,
fragmented selection, one upload per glyph kind across all strips, comparison
and packed-length fallback, and full-frame restoration. Dense 1080x680 partial
tests verify fewer than 10% of instances enter the actual prepared GPU stream.
