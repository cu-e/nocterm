# Performance measurements

Use an isolated checkout and an isolated `NOCTERM_HOME`. Never install hooks in
an ordinary development checkout. The installer supports both the baseline
`AgentPanel` and the `feat/agent-chat-flow` lifecycle module. The hooks do not
ship in production; they dispatch keys only inside the benchmark window.

```sh
python3 scripts/perf/install_hooks.py /path/to/benchmark-checkout
CARGO_PROFILE_RELEASE_STRIP=none CARGO_PROFILE_RELEASE_DEBUG=line-tables-only \
CARGO_PROFILE_RELEASE_LTO=off CARGO_PROFILE_RELEASE_CODEGEN_UNITS=16 \
cargo build --release --manifest-path /path/to/benchmark-checkout/Cargo.toml
```

Set `[local].program` in the isolated home's `config/settings.toml` to the
absolute path of `scripts/perf/shell.sh`. Compare alternating runs with identical
homes, profile overrides, window dimensions, focus, theme and compositor state.
The runner assumes a Hyprland monitor `PERF-1` actively displaying workspace 4;
it checks visibility and refuses hidden-window measurements. It does not switch
the user's active monitor or send global keystrokes. Requires Python, Hyprland
and `grim`. Each run creates a fresh directory with results, log and screenshot.
Logs and screenshots can contain private data: keep them outside the repository.

```sh
python3 scripts/perf/measure.py /path/to/nocterm --home /tmp/isolated-home \
  --mode shell --keys a,b,c,space,backspace --seconds 15 \
  --run-dir /tmp/perf-run-baseline-1
```

Add `--agent --chat` for the largest restored chat. `--mode type` measures
synthetic output, not input dispatch. CPU is a percentage of **one CPU core**.
RSS is the resident memory at the end of the measured interval. This runner
does not claim to measure frame latency or allocation counts.

Flood requires freshly instrumented binaries. It waits for an OSC title
sentinel to be consumed by the emulator (`flood-drained`), with a timeout. The
producer marker alone does not establish that queued output has been processed.
The acknowledgement measures emulator drain, not presentation of the last frame.
All waits check that the application is alive. Cleanup terminates only the
benchmark process group and preserves errors from the measurement.

By default the runner copies the home into `run-dir/home` and adjusts only that
copy's local shell command. Use `--reuse-home` only for disposable fixtures.
Fresh hooks acknowledge startup, shell readiness and internal key dispatch into
a connected, focused terminal; key counts are recorded once per second, with
identical instrumentation in each binary. `--legacy-hooks` permits older
binaries and labels their readiness as unverified.

For an explicitly authorized remote scenario, add `--ssh user@host --ssh-key
/path/to/key --ssh-tabs 1`. The hook uses Nocterm's SSH adapter, waits for a
connected session, and sends the same bounded key sequence. It creates shell
sessions and uses the existing host-key policy; it does not approve unknown
keys. Increasing `--ssh-tabs` creates additional remote terminal tabs.
