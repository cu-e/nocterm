# 0149. Cloud composers own captured image and model drafts

Status: accepted. Supersedes the Cloud text-only composer and refused-text-only
rules in [0134](0134-one-composer-for-every-conversation-and-pending-prompts.md),
decisions 1, 2 and 5. Extends the server binding controls of
[0147](0147-desktop-answers-bound-chats-and-runs-prompts-on-bound-servers.md).

## Context

The shared composer hid both attachment controls and model selection for Cloud chats.
The device protocol already carries inline image prompt parts and explicit models,
but its original high-level chat action admitted only text. Binding a server required
first leaving the chat to install the account's CA, and an empty binding form offered
no way to do that. Model changes in another control must not retarget a submitted prompt,
and an acknowledgement must not discard images added while delivery was pending.

## Decision

1. `nocterm-agent-chat` provides the optional, captured `ConversationComposer` port
   beside `ConversationServers`. The port exposes immutable image snapshots, atomic
   append/remove, model choices and selection, and a draft revision. `nocterm-cloud-chat`
   owns that draft in `PromptDraft`; views own input and presentation, never provider
   credentials or remote session rules. The neutral `ConversationView` capabilities
   describe supported controls; a remote composer does not require a Local thread.
2. Prepared images reuse the existing `PromptImage`, image collection admission,
   shared physical work permits and preview cache. A picker retains its original source
   and composer binding; the source rechecks account lifetime on acceptance. A batch
   either appends completely inside the existing eight-image/twenty-MiB limit or changes
   nothing. The draft is accounted for within the chat's shared history admission.
3. Image, model and server target mutations advance one draft revision. A send captures that revision,
   its images, selected model and server. A successful completion clears only that
   original revision; changing the draft while delivery waits creates a different input,
   and never silently retries an earlier body. Model listings are descriptive; the runner
   still checks the sealed agent definition and its configured provider limits.
4. `nocterm-cloud-conversation-model::ConversationAction::Prompt` carries neutral inline
   image bytes and an optional explicit `ModelName` beside text and server. hub-io maps
   these to the contract device core's image prompt action and its existing signed prompt
   layout. An explicit model applies to the next prompt even after earlier runs used
   another model. Available models come from the agent catalog when creating a chat and
   from its seat's catalog entry when reopening one. An own agent instead gets every model
   from its canonical encrypted provider record and actual delivered signed lease. Single-model
   records keep their original format; multi-model records use the contract's format 2.
   Native writes delegate to that record owner. No image, model or server selection
   is a new source of authorization.
   Logical image commands keep the existing 32 MiB encrypted-publication envelope
   and 16 KiB relay frames. Native transport streams those frames; browser transport
   uses capability-gated unary fragments under the same 32 KiB service message limit.
   Old hubs refuse large prompts with update guidance before pending admission.
   The encrypted outbox keeps the exact original command body across frame boundaries.
   A failed submission without an admitted command cannot report a successful replay;
   sending the unchanged draft again makes a new admission attempt and preserves its
   text and images if that attempt is refused again.
5. The Cloud pending-prompt owner retains original submitted bodies for retry, including
   images, model and server. Retention is bounded to 32 commands and 32 MiB, charged to
   history admission before insertion, and released for accepted or dismissed prompts.
   It follows the existing four-refusal display bound and protects retained retry bodies
   from idle eviction. Replacing a refused prompt is transactional: a failure before the
   replacement is tracked restores the original body and row. Encrypted command delivery
   and restart recovery remain the device core's responsibility; an ordinary delivery retry
   continues to publish exactly its original signed bytes. Its restored pending transcript
   contains text, not a complete attachment/model/server body. If that original body is no
   longer held after restart, an explicit new retry fails with a typed actionable error;
   it never reconstructs and silently sends a text-only replacement.
   The native encrypted outbox admits at most 32 commands and 32 MiB before private
   publication. A stable per-chat private OS lock coordinates bounded metadata
   admission, same-command replacement and deletion across installations. Resume
   reads use the remaining byte budget. A current hub capability downgrade retains
   a valid original stored command rather than deleting it as malformed.
6. The Cloud attachment **+** opens the server binding controls. Their onboarding view
   lists saved connections, opens them through the original `ConnectionDirectory`,
   and explicitly allows agent access through the existing `Hubs` service. The service
   retains CA editing and typed errors; there is no dependency on the hubs-ui feature.
   Successful CA installation reloads binding choices. Binding still requires explicit
   server selection and duration and never silently grants trust.
7. Reopening an own-runner chat does not guess a credential from a model name or a
   runner summary. The person explicitly chooses one of their own agents through the
   neutral provider options of the captured source. Adapter `PromptModels` owns
   `Unchosen`/`Choosing`/`Ready` and becomes ready only with the actual delivered lease's
   complete model list. An active run refuses this initial choice. A background task
   retains selection through physical completion even if its UI waiter is dropped.
   Renewed leases refresh those choices; a monotonic revision and the existing task
   notification coalesce `ComposerChanged` updates under subscriber backpressure.
   The service advances its draft revision and drops an invalid selection, so other
   windows and cancelled waiters do not leave stale or unleased models in the composer.
   Vault model edits become available on lease renewal or in a new chat; no extra
   record watcher is installed.
8. The personal-agent editor loads `PersonalAgents::read_models` before constructing
   an own-agent form. Its immutable base is the authenticated private record, not the
   public listing. `OwnModelBase` distinguishes that model snapshot from an explicitly
   missing key; repairing absence requires a newly entered credential. Saving checks
   the agent's original listing version and owner before private publication. An opaque
   `OwnKeyVersion` binds the exact original account, collection, object and read version;
   compare-and-swap never re-reads a later writer's version while publishing an older
   secret. Incompatible private model changes are refused. A retry whose desired models
   are already stored leaves the private record alone and repairs listing metadata.

## Alternatives

- A separate Cloud-only composer would duplicate input, paste, image preparation and
  submission fencing. The shared composer uses the optional captured port instead.
- Showing attachment buttons without an image command path would accept a user's image
  and omit it from the actual prompt. The device action carries inline parts end to end.
- Using the latest run's model unconditionally would make a model picker ineffective
  after the first prompt. Each submitted explicit choice reaches the command.
- Reconstructing a failed prompt from its displayed text and the current draft would
  lose images or run against another model/server. Retained bodies preserve the original.
- Installing the CA automatically when listing servers would turn discovery into a
  trust mutation. Connection, CA installation and chat binding remain explicit actions.

## Consequences

Cloud chats provide the familiar attachment controls and a picker for multiple models.
Image preparation remains bounded and off the render/input path. A failed prompt retains
its submitted attachment body while its pending owner holds it; no new provider authority
or unencrypted persistence is introduced. Provider/operator request limits can still reject
an image request and are not raised by adding an attachment.
