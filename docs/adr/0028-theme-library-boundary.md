# 0028. Theme formats are independent of native libraries and shared UI work

## Context

The domain performed directory discovery, HTTP and archive installation. Three
GPUI globals hid catalog and mutation dependencies. A page owned install work and
reloading could race committed changes across windows.

## Decision

`nocterm-themes` owns supplied-document parsing, palette projection rules,
identity validation, typed diagnostics and the `ThemeLibrary`/`ThemeRegistry`
ports. `CatalogBuilder` parses supplied documents in explicit precedence order;
paths identify origins and are never opened by the domain. Variant names are
unique across appearances and Nocterm Default is reserved.

`nocterm-themes-registry` implements every physical operation. Discovery scans
sorted user files before installed packs, rejects linked files/directories and
bounds each read. Ordinary discovery failures remain typed catalog diagnostics.
HTTP caps both announced and actual bytes and strips request URLs from transport
errors. Archive expansion bounds compressed/decompressed bytes, entries, accepted
files and individual files, validates originals and consumes trailing gzip data.
Only portable direct regular themes/*.json members are written. Private files
inside owned staging use the shared atomic primitive. Directory publication has
its own stage/backup/destination owner; failed rollback preserves the old pack at
an explicit recovery path. Successful final rename commits even if cleanup or
later discovery reports a problem. Trusted ancestors remain required; these
checks do not provide held-directory race resistance.

`nocterm-theme-service` depends on GPUI alone and injected ports. One handle is
retained by application composition and shared across windows. It admits at most
32 simultaneous mutation records, excludes concurrent changes of one id and uses
the existing WriteGate for physical ordering. A mutation holds that gate until
GUI publication acknowledges its resulting catalog; busy release and receipts
follow publication. Admitted work owns its lifetime independently of pages.
Reloads coalesce one active scan and one pending request. Epoch changes reject
pre-mutation results and reloads requested during mutations run after completion.
This service adds neither a global nor an application exit hook. Registry work
is not promised shutdown draining; abandoned dot-prefixed staging is ignored.

`nocterm-ui::ThemeUi` receives settings, the service handle and base tokens. It
retains projection observers and publishes through the existing Design mechanism.
Settings and command pages receive this explicit handle. Folder creation runs on
the background executor. Uninstall selection changes apply to current durable
settings and only removed variants with no surviving catalog counterpart; a
settings save failure does not undo the committed removal.

Appearance remains physically in settings-ui for this boundary. Moving all
settings pages to lazy SettingsPageSpec owners requires moving the generic page
contract to ui before ui can implement its page without a dependency cycle.
That remaining ownership work is separate from this complete I/O separation.

## Alternatives rejected

- Moving HTTP alone leaves domain filesystem and archive dependencies.
- Port forwarding to physical functions inside domain disguises rather than
  removes I/O ownership.
- A feature global or page-owned task hides dependencies and loses shared work.
- Releasing the physical gate before GUI acknowledgment permits reordered catalogs.
- Treating a committed rescan diagnostic as failed install encourages duplicate work.

## Consequences

Root composition alone imports native theme adapters. Tests supply documents and
shared dev-only in-memory ports; physical fixtures remain adapter tests. Existing
archive and discovery limitations remain explicit. Generated graph and notices
must follow the new manifests; architecture debt allowances only decrease after
measuring actual source identities.
