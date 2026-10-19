# 0045. Private panel view owners retain controls and completion authority

## Context

AgentPanel retained controls, disposable presentation jobs and chat coordination
as flat state. Shared native layout and settings acquisition depended on ambient
installation. Search and media callbacks could observe a replacement view with
an identical-looking query or row key.

## Decision

Five private feature owners retain the actual GPUI controls and presentation
lifetimes. MessageList owns scroller, reveal backlog, navigation, expanded rows
and the existing ImagePipeline. ComposerView owns textarea, queue presentation,
command selection and disposable edit/loading/image-preparation state.
HistoryColumn owns query/catalog incarnation, bounded search, original rename
input and display clock. PanelLayout owns both native split entities and exact
reversible widening; MenuView owns menu/search/usage focus and their bindings.
Owners expose named transitions and immutable render projections, never mutable
state bags, parent callbacks or a second authoritative transcript/composer.

Immutable PanelDeps supplies image ports, shared LayoutHandle and SettingsHandle.
Bootstrap retains the same dependencies across windows. Layout observation copies
one normalized visible scalar before its callback, excluding unrelated keys and
persistence-only notifications. External split refresh is marked by observation
and consumed during native group rebuild. Reconciliation follows changing native
container bounds through the settling draw, then stops once those bounds are
stable even when the requested width is clamped. A final confirmation paint
separates the previous measured bounds from changes arriving in native prepaint.
A reset or the single confirmation schedules a weak next-frame notification bound
to the original window, selected split and refresh incarnation. Hidden or replaced callbacks cannot advance another refresh. Input
handlers never see a reset split between frames, and reconciliation emits no
persistence event.

Finite callbacks carry original chat/input/session/composer or list identities.
Search admission validates both query and catalog incarnation; rename Enter/blur
can commit only the original input once. Borrowed transcript rows produce owned
presentation effects before GPUI updates; no full transcript or media baseline is
cloned. Accepted submission alone clears input; refusal retains text and queued
media. Existing document admission, HistoryReader permits and shared persistence
remain authoritative.

## Consequences

AgentPanel retains fifteen private fields, including supplied settings. It still
coordinates chat membership, document admission, notices, sign-in cards and legacy
runtime/authentication acquisition. Further coordinator and installation extraction
are separate decisions; this record does not claim four implementation files or
protocol-neutral history. UI appearance, data formats, native image decoding,
shutdown deadlines and accepted Thread boundaries remain unchanged.
