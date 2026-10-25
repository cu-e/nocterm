# User plugins

Preview. A user plugin is a single file the user installs: a TOML manifest, or a WebAssembly
module that carries the manifest. Both are drawn by the host inside the status bar and the
panels, and neither can reach more than the capabilities the user agreed to.

User plugins are compiled in with the cargo feature `nocterm/user-plugins`; release builds are
made without it. A build with the feature loads no plugin until the user allows it, and
`nocterm --safe-mode` loads none whatever the setting says.

| Document | What it is |
| --- | --- |
| [Guide](guide.md) | How to write, check and try a plugin, and what the user sees. |
| [Reference](reference.md) | Every capability, manifest key, vocabulary and limit. Generated. |
| [manifest.schema.json](manifest.schema.json) | JSON Schema 2020-12 of the manifest for editors. Generated. |
| [`examples/plugins`](../../examples/plugins) | A declarative clock, a notes plugin and a complete code plugin. |

## Commands

```sh
nocterm plugin check path/to/directory   # read every package as the application would
nocterm plugin dev path/to/plugin.wasm   # try one file in a session that saves nothing of it
nocterm --safe-mode                      # start without any user plugin
```

## Compatibility

`crates/plugin-api/api-surface.json` is the contract. A CI job compares it with the file at the
latest release tag and fails on a change that could break a published plugin: a removed or
retyped capability, field, manifest key, vocabulary value or effect field; a manifest key whose
vocabulary changed or whose integer range narrowed; a stricter rate, cap or limit, and any new
limit; a key that became required. Any other change except wording needs a higher `api_version`;
an entry marked deprecated may be removed on a higher major version, and a new limit needs one
too. A new field of an effect request must be optional for the guest, and the surface records each
field with its type, so the job fails one that is required or retyped. The same job holds the ABI golden file of code plugins (`crates/plugin-abi/abi-golden.json`) to
the release: a changed or removed export or sample needs a higher `abi_version`. The check cannot run without the tag list: a base it cannot read fails the job. It is
`python3 scripts/check-plugin-api-compat.py`.

`cargo xtask docs` regenerates the reference, the schema and the surface file, and
`cargo xtask docs --check` also fails when this guide names an unknown capability.
