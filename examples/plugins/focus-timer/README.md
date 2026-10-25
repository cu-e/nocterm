# Focus timer

An example code plugin written in Rust with `nocterm-plugin-sdk`. The status bar item shows the
minutes since the round started; activating it opens the panel, which draws a meter against the
goal (a setting) and has Reset and Close buttons. A round that reached the goal counts toward the
badge on the bar item; the badge is set when a round is counted and again each time the item is
pressed, because only a reply to an event can change it. The plugin keeps no state of its own:
the start and the count live in its stored data.

This directory is a workspace of its own, so the application's workspace and lockfile do not carry
the wasm32 target or the `cdylib`.

```sh
rustup target add wasm32-unknown-unknown

# Test natively with the SDK's test host.
cargo test --manifest-path examples/plugins/focus-timer/Cargo.toml

# Build the package: one .wasm file that carries plugin.toml as its manifest.
cargo build --release --target wasm32-unknown-unknown \
  --manifest-path examples/plugins/focus-timer/Cargo.toml

# Check it as an installation would, then try it for one session (a build with user plugins).
FILE=examples/plugins/focus-timer/target/wasm32-unknown-unknown/release/focus_timer.wasm
nocterm plugin check "$(dirname "$FILE")"
nocterm plugin dev "$FILE"
```

See [Code plugins](../../../docs/plugins/guide.md#code-plugins) in the guide.
