# Nocterm GPUI patch

This directory vendors the released crates.io `gpui-pre` 0.3.7 archive, whose
SHA-256 checksum is
`0e87a42bb37c7cb4e76dd1ac0ce88851e46e976e0373a47ab3e0757abffee54d`.
Its manifest identifies the upstream Zed revision as
`1a28cff4b409169bac058bca40dfbfeb7621d19b` (`gpui` 0.2.2).
The upstream normalized and original manifests, build script, resources, examples,
tests, documentation, and `LICENSE-APACHE` are retained from that archive.
The normalized manifest additionally registers the Nocterm scroll integration test.
The root Cargo manifest patches `gpui-pre` to this directory and excludes it
from the Nocterm workspace. Cargo regenerated the lock graph for the local
patches, unifying several compatible dependency edges onto versions already
present in the lockfile; no library package versions were upgraded. The
Cargo-generated graph is retained and verified with `--locked` builds.

The input-presentation policy changes `src/window.rs` and adds
`src/window/input_presentation.rs`. Windows windows cache the
adapter's `is_software_emulated` classification when created and refresh it on
forced frames, which include device recovery. Driver queries never run on every
normal frame. A known software adapter skips the optional high-rate-input
presentation tail for unchanged scenes: WARP redraws those scenes on the CPU,
and the display-underclocking workaround offers no benefit there. The same
policy governs whether recent input bypasses the inactive-window frame limit.
Hardware and unknown adapters, and other platforms, retain upstream behavior.

Required and explicit presentations remain honored. Dirty and forced frames
still draw and present; next-frame callbacks, animation invalidation, platform
wakeups, and frame scheduling are unchanged. This patch does not rate-limit
terminal updates or disable actual animation frames.

Pure regression tests cover adapter policies, explicit presentation requests,
and changes of renderer classification after recovery. They can run without a
graphics session using:

```sh
rustc --edition=2024 --test vendor/gpui-pre/src/window/input_presentation.rs \
  -o /tmp/nocterm-gpui-input-presentation-tests
/tmp/nocterm-gpui-input-presentation-tests
```

## Retained scene damage

`src/scene/damage.rs` exposes an owned `SceneSnapshot` and conservative
`SceneDamage` results for platform renderers. Snapshots compare exact primitive
fields in actual renderer order within 64-pixel tiles. Records are stored once;
tiles contain indices. Order numbers and path IDs are normalized, while path
batch boundaries and individual-versus-spanning intermediate copies remain
part of the signature. No raw byte comparisons or probabilistic hashes are
used. The scene primitive types gain additive `PartialEq` implementations.

Coverage includes content masks, fractional-pixel antialiasing guards, the
Windows shader's three-blur-radius drop shadow extent, and inset shadow element
bounds. Nonidentity glyph transforms, external surfaces, nonfinite values,
viewport changes, or bounded storage limits request full rendering. Batch
intersection uses individual path coverage for same-order paths and spanning
coverage for mixed-order intermediate copies. Platform renderers remain
responsible for invalidating retained pixels when atlas contents, device state,
or other renderer state changes.

Regression tests cover equality, local insertions and deletions, ordering,
clips, transparency, shadows, fractional edges, path vertices/copy semantics,
and conservative fallbacks. The packaged upstream GPUI library unit tests refer
to fonts absent from the crates.io archive. A workspace integration adapter
therefore runs these same cases against the public production API:

```sh
cargo test -p nocterm-terminal --test scene_damage
```

## Verified vertical scroll reuse

`src/scene/damage/scroll.rs` adds the optional `SceneScrollPlan` API separately
from ordinary tile damage. A bounded search matches up to four large, unchanged,
opaque rectangular backdrops and up to eight integer vertical shifts inferred
from matching glyph atlas tiles, horizontal geometry and colors. Up to 128 current
anchors query an exact identity index over up to 16,384 previous visible glyphs;
source search covers rows beyond the first dense text row, and total matches are
capped at 262,144. Proof compares
ordered primitive contributions on each destination scanline with the previous
source scanline. Equivalent backdrop candidates are deduplicated by region and color and pair
the deepest matching opaque layer in each scene, so hidden earlier unsupported
layers cannot block reuse or consume all candidate slots. The backdrop hides
every earlier layer. Flat solid quads have
uniform pixel color over their exact integer clipped horizontal spans; glyphs
must match type, atlas tile, color, identity transform, size and horizontal
position, with precisely translated integer Y positions. Supported glyph bounds
must be fully contained by their masks and by an at-most-8192-pixel viewport so
clipping cannot change interpolated atlas coordinates or rasterizer phase.

Fractional opaque flat backdrops use only a one-pixel-guarded integer interior;
antialiasing fringes and clip boundaries remain ordinary damage. Every fractional
quad after the backdrop rejects its full conservative coverage.

Fractional, clipped or transformed glyphs, rounded/bordered/non-solid quads,
shadows, paths, underlines and color sprites reject affected scanlines. Unknown
coverage rejects the candidate. Up to512 conservative coverage intervals from
tall unsupported edge decorations in both scenes may crop the horizontal copy
interior, retaining at least half its original width; cropped columns remain
ordinary damage. This avoids a one-pixel parent-border AA guard rejecting every
pane scanline without changing supported shader rules. Supported-row counts and
maximum translated overlap prune candidates that cannot meet the reuse threshold
or improve an existing plan. Unsupported rows never allocate further glyph
reference lists; original reference-budget accounting remains unchanged. Only
equal rows may be copied; all remaining
rows and ordinary damage outside the region must be replayed. Comparisons use
exact fields rather than hash equality or a terminal-specific heuristic.
Backdrop search, candidate shifts, scanline references and copy/replay strips
are capped; insufficient reuse or exhausted limits retain ordinary damage.
The renderer must additionally enforce unchanged atlas contents, appearance,
viewport and device resources.

The separate integration suite avoids upstream test-only font assets:

```sh
cargo test -p gpui-pre --test nocterm_scroll
```

Generic Div wheel input clamps the proposed offset before comparing it with the
current drawable offset and notifying. Prepaint and input share the same full
element bounds, device-snapped padding and two-decimal scroll limit. Fitting
content and scroll boundaries avoid transient invalidation; event propagation,
axis routing and precise-gesture filtering remain unchanged. The scrolling
methods are extracted into `elements/div/scrolling.rs` and tests assert raw
offsets and window invalidation immediately after dispatch, before prepaint.
Upstream SVG font test fixtures are exact pinned bytes under `test-fixtures/`,
with original licenses and provenance; production font loading is unchanged.
Spring elements read the executor clock, matching scheduled animation frames and
deterministic test clock advancement; production executors retain their normal
monotonic clock.

Linux opts into `ReusableSceneSnapshot` and `SceneComparisonMemo`; classic
`SceneSnapshot::capture` and `damage_since` remain unchanged. Two snapshots reuse
outer record/tile storage and tile-index vectors. Actual capacities are bounded
per snapshot at 50,000 records, 65,536 tiles and 500,000 tile references; nested
path/vertex capacities use the existing 50,000/500,000 limits. The memo is bounded
at 50,000 entries and resets before each comparison. Each entry remembers only
the last old record index and its exact equality result, including false results.
Viewport shape changes or capacity failure release storage. Reserved-byte limits
derive from these capacities and Rust type sizes, including both snapshots and
the memo; storage cannot grow indefinitely as content moves between tiles.

Nested view caching changes `src/view.rs`, `src/window.rs` and
`src/text_system/line_layout.rs`. Upstream draws every cached view nested in a cached
view that draws again, so one notified view redraws all cached views under each of its
cached ancestors. Here a cached view drawn again keeps the window's refresh state, and
the cached views nested in it are reused unless they, a view inside them or the
window's refresh asks for a draw. Each cached view records the cached views prepainted
inside it. When it is reused, its output is copied whole without visiting them, and
their recorded prepaint and paint ranges move by the distance the copy moved. A cached
view is reused only if the draw before painted it, so a range from an older frame is
never replayed. A view still draws again when it is notified, when a view inside it is
notified, when its bounds, content mask or text style change, or on a window refresh,
which focus, activation and theme changes request. Reading another entity while
rendering does not redraw a cached view, as upstream. The regression test
`nested_cached_views_survive_their_ancestors_reuse` covers reuse after a copied frame
and input on the moved output.

Content-sized caching adds `cached_to_content` to `Entity` and `AnyView` in
`src/view.rs`. Upstream lays a cached view out from its style alone, so a view whose
size follows its content (a composer that grows with its text, a status-bar widget)
cannot be cached. A view cached to its content lays its content out, as an uncached
view does, whenever it draws again; a frame that reuses it lays it out at its style
with the size its content last took along the axes the style leaves auto. A reused
size that the content no longer fits, because the bounds changed, lays the content
out again the next frame. `AnyView::observe` calls back on the notifications of a view
held without its type, for an owner that draws part of it from a cached view of its
own. The regression test `views_cached_to_content_keep_their_contents_size` covers
the measured size, its reuse and a notified size change.
