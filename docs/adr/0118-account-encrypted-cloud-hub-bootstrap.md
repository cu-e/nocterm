# 0118. Cloud hub trust bootstraps from the normal encrypted account

Status: accepted

## Context

A second device must discover Cloud history after ordinary account sign-in,
without a separate Cloud unlock or trusting directory-supplied runner keys.
The runner registry is scoped by a pinned Cloud hub authority, so it cannot
itself provide the first authority pin needed to open that registry.

## Decision

The native account adapter stores one immutable hub-authority public key in an
exact personal encrypted collection. Its deterministic selector and authenticated
private context use the normal configured canonical hub address, raw native hub
instance ID, account ID and account public-key digest. They contain no Cloud
authority, runner ID, device ID or login generation. Ordinary account keys open
this record on another device configured for the same origin and account.

An explicit initial setup compares an allowlist of public runner CLI metadata
through an independently trusted channel. Native setup validates its complete
origin/account/scope and verifies the candidate hub's actual signed capability
before compare-creating any authority record or collection. A wrong initial pin
therefore leaves no permanent trust that could block corrected setup. The existing
encrypted runner registry remains the sole runner pin roster. Partial completion
between authority registration and runner registration is explicit and resumable.

Reads never create or select another collection. A missing record requires setup;
deleted, moved, malformed or conflicting records are refused. Same-value retries
read/write the same selector. A different authority requires an explicit supported
rotation and is never silently adopted from a hub response or ordinary settings.
Account collection/object encryption and guarded Store operations are reused;
there is no additional crypto primitive or Cloud wire format.

Native metadata derives the Cloud namespace using the sole public canonical
namespace function after authenticating the authority. Returned scope values are
descriptive metadata, not fresh authority. Cloud capability validation and fresh
nonce-bound runner catalog/head witnesses remain mandatory before history replay.

Captures retain their original normal account membership and weak key owner.
Finite work leases precede SDK/decode/crypto allocations and follow actual request
and response bodies, native workers and results. Guarded resident metadata may
outlive work, but private account keys do not remain on idle factory handles.
Lock, logout and a replacement login fence callbacks and publication; an account
switch never retargets an existing source.

## Consequences

The application can compose existing runner registry/catalog/material/source
adapters after ordinary sign-in. A separate URL origin is not implicitly linked,
even if it reports the same alias or raw instance ID. Store availability is not
a new freshness oracle: withholding bootstrap state refuses access, and history
freshness still requires the pinned runner's challenge response. Authority
rotation and first-chat creation remain distinct explicit transitions.
