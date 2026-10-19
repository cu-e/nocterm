# Theme fixtures

`dracula.json` is the original theme from https://github.com/dracula/zed,
retrieved from Zed's registry, extension dracula 1.1.2, on 2026-10-04.
It is MIT licensed; its copyright and permission notice are in `dracula-LICENSE`.

`search_response.json` records the public Zed registry search for dracula on
2026-10-04, https://api.zed.dev/extensions?max_schema_version=1&provides=themes&filter=dracula.
The minimal versioned fixtures are handwritten, under this repository's Apache-2.0 license.

The original Dracula 1.1.2 variant has `terminal.ansi.bright_white = "ffffffff"`
without a leading `#`; this invalid color is intentionally retained to test
refinement from nocterm's built-in ANSI palette.
