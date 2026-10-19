# 0043. Connection views retain their explicit installation

## Context

Saved connections and facts used independent globals. Directory, editor, sidebar,
menu and credential callbacks looked up those globals, while country display and
probe admission read settings through a separate compatibility lookup. Installing
another connection owner could redirect an existing window or editor.

## Decision

The connections feature returns a cloneable `Installed` handle over private,
immutable collaborators: the exact connection entity, facts entity, editor inputs,
settings handle and installation subscription lifetimes. Application bootstrap
retains the handle and passes it to every workspace registration. Views and actions
capture the supplied installation; no connection or facts globals remain.

The existing `ConnectionDirectory` port stays narrow. Feature session intents
convert saved/recent values and record foreground use before the workspace session
factory receives an open request; background open does not record recent use.
The credential association callback captures the actual connection entity rather
than the UI-only installation, preserving an entity seam suitable for terminal
composition. Unavailable installation has an explicit typed association failure
and skips connection feature registration.

Country detection/display query the supplied `SettingsHandle`. Installation owns
its settings/profile/facts subscriptions and invalidates presentation when
published settings change. Manual country selection remains visible while detected
country display follows policy. Closing one window cannot redirect or dispose the
shared installation retained by the root and surviving windows. Distinct
installations remain independent.

## Alternatives rejected

- Renaming the globals into an Installed lookup facade retains hidden authority.
- Moving feature images and views into a lifecycle service couples service and UI.
- A broad workspace trait exposing arbitrary connection state expands consumer
  authority beyond directory/open needs.

## Consequences

Profile/facts formats, checked persistence and the shared shutdown coordinator
remain unchanged. Publication and observation scheduling still belong to their
existing feature owners. A separate service extraction and stamped observation
completion authority remain migration work; this decision does not resolve stale
observation publication after deletion, re-addition or policy changes.

This supersedes only the compatibility-global installation portion of earlier
connection boundary descriptions. Accepted ADR0013,0027,0036 and0039 retain their
historical context; their persistence/native capability contracts remain in use.
