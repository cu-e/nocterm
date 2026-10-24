# 0088. The wall clock is a foundation contract read by a local adapter, and the Clock is a plugin

Status: accepted

## Context

The status bar needs the local time. Rust's standard library gives the UTC instant but not
the offset of the local time zone, and reading that offset needs a time-zone library.
[ADR 0040](0040-native-chat-metadata.md) already moves a native time reading behind a
consumer-named port, and `ConnectionClock` does the same for connections; both give scalar
times and no zone. A widget that must show hours and minutes needs the zone and must be
tested without the real time.

## Decision

**The contract.** `nocterm_core::clock::WallClock` has one infallible method, `now`, that
returns `WallTime { unix_seconds, utc_offset_seconds }`. `WallTime` only does integer
arithmetic on the two numbers (the local hour, minute and second, the local calendar date,
`seconds_to_next_minute`), so the foundation layer takes on no I/O and no time-zone or
calendar library. `FixedClock`, a clock a test sets
and counts the reads of, is behind core's `test-support` feature.

**The adapter.** `nocterm_local::SystemClock` reads the system time and the zone in effect
at that instant with chrono (`default-features = false`, feature `clock`, the same
specification as the agent crate). Reading the local zone is a native local fact, so it sits
beside the local terminal and command adapters; core would otherwise depend on a time-zone
library. A zone the system cannot report is UTC.

**Composition.** The clock travels in `nocterm_workspace::Deps` (`with_clock`) and is
supplied where the application builds those dependencies. `HostServices` is native
connection capabilities and gets no field. A window composed without a clock has no clock
widget, so test windows need none. Nothing is a global.

**The Clock plugin.** The `clock` system plugin is declared by `nocterm-workspace`, which
creates its view, and is listed in the application catalog like any other plugin. It has one
status bar widget, `clock.time`, with the right zone and order 25 and `auto: Tray`. The
Default arrangement therefore does not change; a layout that places the widget shows the
time as `HH:MM`. The view is created by the widget factory, so it and its timer exist only
while a layout places the widget and the plugin is enabled; dropping the view drops the
timer.

**The Clock settings.** The plugin owns the `[clock]` section, `ClockSettings`, in
`nocterm-workspace` beside the view, and one schema group on `clock.time` over the plugin
settings API ([ADR 0085](0085-one-plugin-model-with-separate-trust-tiers.md)): `format`
(choice `hour24` or `hour12`, default `hour24`), `seconds` (toggle, default off) and `date`
(toggle, default off). With the defaults the text is still `HH:MM`, so the Default bar is
unchanged. `hour12` writes `1:05 PM`, `seconds` adds `:SS` before the meridiem, and `date`
puts a short local date such as `Oct 24` before the time. The form is the one the Plugins
page and the layout editor already draw, bound by a `SectionBinding` over the section. The
view reads the section through the settings handle that `Deps::with_settings` carries,
follows its revisions, and re-arms its timer for the new style: once per second with seconds
on, else at the minute boundary but never later than five seconds, because the timer runs on
the monotonic clock and does not see a suspend, a time-zone change or a clock step; reading
again is cheap and the view redraws only when the text changes. Every window shares the handle, so a change shows in all of
them at once. The section sits in the workspace because the view is its only reader; a
domain crate would hold a type nothing else uses.

## Consequences

- Any consumer that needs the local time takes a `WallClock` and is tested with
  `FixedClock`; the existing consumer-named ports stay as they are.
- The Plugins page lists Clock with the other plugins, and switching it off removes the
  widget and stops its timer in every window.
- The Clock is the first plugin whose settings live in a section that the window shell
  owns; the settings reference lists `[clock]` and `xtask` depends on the workspace crate to
  render it.
- A text that depends on a setting is formatted by `ClockSettings`, which takes a `WallTime`,
  so every combination is tested without a window or a real clock.
