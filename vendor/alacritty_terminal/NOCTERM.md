# Nocterm's Alacritty terminal patch

Origin: the crates.io `alacritty_terminal` 0.26.0 source distribution,
[upstream repository](https://github.com/alacritty/alacritty). Original copyright
headers and [Apache-2.0 license](LICENSE-APACHE) are retained. The internal unit
tests remain unchanged; Nocterm's additional coverage lives in
`crates/vt/src/emulator.rs`. This is an internal Cargo patch, not a new published
terminal library.

Only `src/term/cell.rs` and `src/term/mod.rs` differ from the distribution's
library sources. Cargo uses the original normalized package dependencies.
The upstream 46 MiB reference-recording fixtures and their integration target
are omitted from this minimal source copy. Upstream's full reference suite can
be run from its complete source distribution; it is not claimed as part of
Nocterm's verification.

## Behavior and memory

Metadata is disabled by default. `Term::set_output_timestamp_ms` opts in and
supplies the host's observation time. Nocterm calls it when feeding the parser.
The resulting `LogicalLine` records a monotonic number and the first output time
in Unix milliseconds. Soft wraps share the number and time; normal line feeds
allocate a new number on output, including explicitly empty lines. Alternate
screen output does not allocate normal-screen numbers. CR and row-local
EL/ECH/ICH/DCH editing retain the line's identity. Screen erasure/reset drops
cleared metadata; remaining history is never renumbered.

Metadata travels with cells during scrolling, insertion/deletion and resize
reflow. It lives inside the existing `CellExtra`, preserving the upstream
24-byte Cell size bound. Attribute reset operations preserve lineage while
removing hyperlink/underline data. A bounded, one-entry cache shares the same
`Arc<CellExtra>` between ordinary characters of a logical line and attribute
template; it does not retain an unbounded map of old lines. The scrollback limit
still bounds retained cells and metadata. Marked output blanks count as occupied
for resize, so explicit blank lines are preserved. Without opt-in, upstream
cell emptiness and terminal behavior are unchanged.

The VT adapter exports metadata separately from the terminal text. Rendering
uses UTC `HH:MM:SS`, with this machine's clock; it does not interpret network
chunks as lines or use the remote host's clock. The gutter stays outside the
PTY grid, clipboard text and selection, and is hidden in alternate screen.

## Verification and updates

```sh
cargo test --manifest-path vendor/alacritty_terminal/Cargo.toml --lib
cargo test -p nocterm-vt -p nocterm-terminal
cargo clippy -p nocterm-vt -p nocterm-terminal --all-targets -- -D warnings
```

When updating Alacritty, compare this directory with the new published library,
review every metadata hook against its character-writing, row-editing, clearing
and resize paths, then rerun both suites. Keep the upstream cell-size cap and
other unit assertions intact. Do not replace the cell lineage with a parallel
snapshot line counter: snapshots cannot distinguish wraps, redraws or reflow.
