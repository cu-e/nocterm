# 0085. System and user plugins share one contribution and settings model, with separate trust tiers

Status: accepted

## Context

[ADR 0082](0082-system-plugins-and-core-contributions.md) gave the built-in shell
contributions a plugin each, and the zoned status bar
([ADR 0083](0083-zoned-status-bar.md)) draws them from data. Two things are still
missing. A plugin has no way to say which settings it has, so each feature draws its own
page and nothing else can show or edit those settings (the layout editor, a Plugins page).
And plugins loaded at runtime are expected later: they must reuse what a plugin is, but
they must not inherit the authority of compiled-in code.

[ADR 0001](0001-settings-sections.md) and [ADR 0056](0056-owner-settings-pages.md)
keep each settings section, its validation and its page with its owner; the host only
renders.

## Decision

**One model, two trust tiers.** A plugin is an id, a name, a description, contributions,
and optionally a settings schema and the section that stores it. What is shared by every
tier is the identity types (`PluginId`, `ContributionId`), the contribution kinds,
placement hints and icons, the settings schema, the host-rendered form and the meaning of
enable state. What is not shared is the descriptor type, the privileges, the way code
runs, the storage of settings and of enable state, and what a validation failure does:

| | System | User |
| --- | --- | --- |
| Descriptor | `SystemContribution`, const data in the owner crate | a separate owned type, clamped when read |
| Privileges | pinned, shown by default, built-in groups, full icon set, required | none of these |
| Code | Rust in the application with the feature's own authority | a sandbox behind capabilities |
| Settings | the owner's typed section of `settings.toml` | a host-written document per plugin, never `settings.toml` |
| A bad declaration | fails startup with a typed `CatalogError` | rejects the package |

Separate descriptor types keep privileges out of data a manifest could set.

**A declarative settings schema.** A plugin declares groups of fields. A field has a key,
a label, optional help and one of five kinds: toggle, choice, multi-choice, bounded number
and single-line text. There is no colour, secret, password, masked or multi-line kind, and
no custom view: a need the five kinds cannot express is a new kind, added by amending this
decision. Limits are checked when the catalog is built (counts, lengths, option keys, a
default that satisfies its own field, a group attached to one of the plugin's own
contributions).

- An edit is data: set a value, or turn one option of a multi-choice field on or off. The
  schema applies it to the value current at write time, so two quick edits both land, and
  a refused edit leaves the stored value as it was. Refusals are typed and their text is
  meant to appear under the field.
- A text value is data, never interpreted. It is stripped of control characters, line
  breaks and every Unicode `Default_Ignorable_Code_Point` (zero-width characters, soft
  hyphen, bidirectional controls, variation selectors, tag characters, fillers) before it
  is bounded and stored, and at most two nonspacing or enclosing marks (categories Mn and
  Me) are kept after one base character. Stripping the joiners U+200C and U+200D splits
  emoji sequences and changes Persian and Indic spelling; that is accepted, because an
  invisible character cannot be told from a hidden payload. A stored value is sanitized and
  cut to its field's length again when it is read, so a hand-edited document shows no
  hidden or unbounded text. Text is displayed literally in an ordinary input. Text is not
  for credentials.
- A number field is edited by a relative step as well as by a value, so quick steps
  compose on the stored value. A field's current value is its stored value, else its
  default, for every reader and every edit.

**Storage per tier.** A system plugin's values stay in its owner's typed section; the
field key is the serde key, so no key moves and no migration is needed. The owner supplies
a binding with `read` and `write` functions over its section, and writes run inside the
settings store's checked update so that they are ordered with the owner's own writes.

**The form is the host's.** `nocterm-ui` renders the schema (`PluginSettingsForm`) and
reads and writes through a port named for its need (`PluginSettingsSource`). The system
implementation is `SectionBinding`; a directory maps a plugin to its schema and source.
Features declare the schema and the binding; the host draws it. A feature does not gain a
view inside the form.

## Consequences

- Monitor's status bar metrics are the first schema; `[monitor].status_bar` keeps its key
  and values, and the Monitor page is unchanged.
- The generated plugins reference lists every settings field with its default, options or
  range, and where it is stored.
- A second tier adds a descriptor type, a storage and a sandbox; none of them changes the
  schema, the form or the source port.
- Accepted ADRs 0001, 0056 and 0082 are unchanged.
