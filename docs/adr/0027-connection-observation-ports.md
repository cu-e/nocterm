# 0027. Country and flag observations use injected native capabilities

## Context

The connections feature resolved host names, constructed HTTP clients and read
and wrote SVG flag files. Its image cache and built-in fallback were coupled to
native acquisition, preventing memory-only substitutions and leaving disk cache
reads unbounded. A deterministic temporary cache filename could collide between
independent publishers.

## Decision

`nocterm-connections-model` owns checked country keys, public-address policy and
synchronous resolver, locator and flag ports. It performs no I/O and names no HTTP
protocol/client types. Error variants preserve native causes through opaque Error
sources. `nocterm-connections-io` implements those ports with local DNS, bounded
HTTP and flag-file operations. Application bootstrap injects native capabilities
when persistence is configured and inert capabilities otherwise.

Only public IPs reach the country service. The existing address policy, blocked
local names, eight-second HTTP timeout, endpoint/redirect behavior and SVG sniff
remain unchanged. Both HTTP and disk flag reads are capped at 256 KiB; country
responses are capped at 4 KiB. Validated codes constrain URL/file keys. Network
errors remove request URLs before retaining their sources.

Flag caching uses the shared private atomic publication primitive: exclusive
staging, mode 0600, exact final-path replacement and trusted ancestors. Failure to
stage or publish is logged with a typed source and still returns downloaded bytes.
The feature owns GPUI conversion, its memory cache and embedded fallback SVGs.
Acquisition stays memory, then disk, then HTTP, then embedded fallback.

## Alternatives rejected

- Passing a client or flag path into feature lookup helpers retains I/O ownership
  in presentation and cannot substitute complete capabilities.
- Moving the whole existing feature to a domain crate moves its UI and native
  debt rather than separating boundaries.
- Writing another temporary-file helper duplicates the atomic publication owner.

## Consequences

Blocking calls run only on the feature's existing background executor. DNS can
outlast the HTTP timeout and cannot promise immediate cancellation. The SVG sniff
is not full SVG validation. Country keys validate shape rather than current ISO
catalogue membership. A malformed cached country is rechecked through the locator.

This boundary does not supersede the remaining connection profile storage,
remote OS probing, global compatibility handles or publication orchestration;
they retain their current owners. It creates no new writer, global or quit hook.
