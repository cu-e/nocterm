# 0035: Image codecs are injected and previews retain request generations

Status: accepted. Complements the private composer admission decision in
[0034](0034-chat-config-composer-owners.md).

## Context

Prompt validation decoded images inside AI domain code. Transcript previews then
validated and decoded the same image again without applying decoder allocation
limits to that second decode. The feature also encoded PNG itself. Cache keys could
be reused after a clear or chat change, so occupied-key checks alone did not identify
the request allowed to publish a result.

## Decision

AI owns pure source/header preflight, immutable checked image facts, prompt encoding
and collection budgets. ImageInspector supplies physically validated original bytes;
prompt admission checks facts and Arc identity against its own preflight.
ImageThumbnailer supplies bounded PNG previews with checked dimensions and output
size. Errors retain typed native causes without exposing the image library or adding
protocol-specific types to the new codec contracts.

The separate nocterm-image-codec adapter owns native PNG/JPEG/GIF/WebP decode and
PNG encoding. Shared preflight precedes allocation. Decoding caps dimensions at
4096 per axis and allocation at 64 MiB. Previews preserve aspect and alpha, fit
requested bounds up to 640 pixels and encode through a sink capped at 4 MiB. The
adapter owns no worker, filesystem access or GPUI lifecycle. This avoids conflating
image codecs with agent storage or ACP transport.

Application composition constructs one codec and retains explicit ImageDeps in
Booted for every panel. Runtime needs no codec and no native fallback or new global
is introduced. Feature background work schedules the injected port and constructs
GPUI Image only from checked preview results. Prompt/history retain original bytes,
MIME, base64 encoding and exact encoded-size admission.

ImagePipeline replaces the panel's existing cache field. It owns codec handles and
PreviewCache; each request binds cache generation, request identity and key. Clear
invalidates generation before dropping tasks. Only a current one-shot delivery can
publish success or failure and trigger row remeasurement. Composer admission uses
the existing AgentThread subject command, validating a complete candidate before
replacing images and retaining revision, dirty and queue-edit ownership.

## Alternatives

Keeping a domain decoder or a feature fallback would preserve hidden native work.
Passing codecs through Runtime would add an unrelated service responsibility.
Checking only a cache key would let an old result overwrite a newer request with
that same key. Direct composer mutation would bypass subject lifecycle effects.

## Consequences

Physical format, corruption, allocation and alpha tests belong to the adapter.
Pure admission and feature behavior use shared scripted evidence without a decoder
fallback. Checked source identity proves port consistency, not trust in a malicious
injected implementation; host composition selects the native adapter. Existing ACP
prompt payloads and history wire coupling remain separate migration responsibilities.
