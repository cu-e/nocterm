# 0143. The runner publishes chat summaries; journal v7

Status: accepted. Supersedes the journal version rule of
[0125](0125-cloud-agents-tail-account-keys-journal-horizon-own-runner.md) ("any other
version is still refused"). Implements the runner side of contract ADR 0008 (runner chat
directory).

## Context

A device could list a runner's Cloud chats only by identity: the catalog head witness names
each chat, its head and control revision, but no title, time, model or state. Getting those
meant replaying every chat. The runner is the only party that sees a chat's plaintext and
orders its events, so the contract added a per-runner directory stream: one signed summary
per chat, sealed from the runner to the account (`nocterm_hub_model::cloud::summary`,
`nocterm_keys::cloud::seal_summary`, `CloudRunner::publish_summary`, capability bit 256). The
hub keeps the newest revision per chat. Existing chats must appear too, without a person
doing anything.

## Decision

1. **Facts come from the runner's own journal.** `nocterm-cloud-model::journal` owns
   `ChatSummaryFacts`, a reduction of journaled observations: the first non-empty line of the
   first accepted prompt (at most 256 bytes; the contract cuts the 120-byte title from it), the
   model of the latest prompt, the statuses of unfinished runs, open approvals and deletion.
   Its `ChatActivity` is the most demanding state of the unfinished runs (approval, grant,
   provider, SSH, running), otherwise whether the latest finished run failed. The coordinator
   maps native records to observations; there is no second event model.
2. **One tracker per chat.** `ChatSummaryTracker` keeps the facts, the newest staged revision,
   the newest revision the hub confirmed and the staged bytes. A change drafts revision
   `max(staged, published) + 1`; at most one attempt per chat every 5 s, growing to 5 min after
   failures; a deletion goes out at once and is the chat's last summary. A staged revision
   that the facts overtook is replaced by a higher revision, never republished with other
   bytes.
3. **Ports.** `ChatSummaryDirectory` (available, seal, publish) is implemented by
   `nocterm-hub-io::cloud::RunnerChatDirectory`: it reads the chat's current head (the facts a
   head witness reports), maps the draft to the contract summary with `title_from_prompt`,
   signs it with the runner signing key, seals it to the pinned account key and publishes it
   under the summary's deterministic publication id. It is available only while the hub
   negotiated bit 256; otherwise nothing is sealed. `ChatSummaryJournal` is implemented by the
   runner journal. Hosts are `Own` (feature bit provider lease) or `Managed { agent }` from
   the boot document; bound servers stay empty until per-chat SSH binding.
4. **Journal v7.** The journal adds `chat_summaries`: per chat the newest staged revision, the
   published revision, creation and last-activity times, the deletion flag and, while
   unconfirmed, the sealed bytes and their chat key epoch. The sealed bytes are already sealed
   to the account and the times are scheduling data, so the row needs no chat key; a deletion
   can be staged after key retirement. `SCHEMA_VERSION` is 7. Versions 5 and 6 are upgraded in
   order on open, each step in one transaction; every other version is refused.
5. **Delivery and backfill.** The runner's existing delivery sweep calls
   `RunCoordinator::publish_summary` for each catalog chat after flushing its outbox. The
   first call for a chat derives its facts from the whole journal (snapshot archive plus
   records): this backfills chats from before v7, with commands at their issue times. Later
   calls read only the records after the last one seen, at the current time; a compaction gap
   re-derives the facts and keeps the title, model and times already known. Journal work goes
   through the coordinator's single journal gate; network waits never hold it. A restarted
   runner restores the row, continues its revisions and publishes once.

## Alternatives rejected

- **Deriving summaries from `CoordinatorEvent`s at commit time.** Background and cleanup
  commits bypass the coordinator log, and a crash between commit and derivation would lose a
  change. Reading committed records covers every writer and is the same path as the backfill.
- **Backfilling inside the SQL migration.** It needs the chat keys and the coordinator's record
  codec; the migration only adds the table and the first sweep backfills.
- **Encrypting the row with the chat key.** It would make a deletion summary impossible after
  key retirement and protects nothing the account box does not already protect.

## Consequences

- Devices can list a runner's chats with title, last activity, host, model, state and pending
  approvals without replaying them; a hub without bit 256 receives nothing new.
- A summary is "current" when its head and control revision match the catalog witness; during
  a run it lags by up to 5 s.
- The first start after the upgrade replays every chat once more to backfill it.
- A runner stores one bounded row per chat outside the chat's logical storage charge.
