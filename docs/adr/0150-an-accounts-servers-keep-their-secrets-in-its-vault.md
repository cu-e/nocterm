# 0150. An account's servers keep their secrets in its vault

Status: accepted. Supersedes in part
[0148](0148-hub-chats-run-on-the-hub-and-use-the-agent-chat-ui.md) (the web panel's
credential "used once and never stored") and the list of agent servers not saved here that
followed [0149](0149-agent-access-connects-with-saved-credentials-and-profiles-have-one-space.md).

## Context

A hub window shows the connections of the account, kept in the account's vault (profiles
sealed end to end, ADR 0101/0149), and its passwords and keys in the vault of the hub. Yet a
server rarely opened there without asking:

- The web panel connected a server for agents with a password or key it used once and never
  stored. The server existed only as the hub's agent server, so a hub window listed it apart,
  under *Agent servers not saved here*, and choosing it opened a new connection that asked for
  the password again.
- The connection editor had no password field. *Password* meant "always ask", and the record a
  prompt had saved was shown as a raw *Credential ID* to copy from the vault. A password reached
  the vault only from a prompt whose *Remember* box, off by default, the user had to tick.
- A *Key file* is a path on one computer; in a hub window it made a server that no other
  computer of the account could open.

## Decision

1. **The account's vault is the home of its servers.** A server of the account is a profile in
   the vault with its secret as a vault record: a password bound to its address, or a private
   key bound to its public half (the formats of `nocterm-hub-io`). Every client that opens the
   account's keys signs in from them; the hub stores only ciphertext. The hub's agent server
   (`ServerService`) is the separate grant that lets agents in with the agent key, matched to the
   profile by address.
2. **The web panel saves what it connects.** Connecting a server in the web panel needs the vault
   open (it unlocks in place) and saves the profile and the password or key the person typed,
   built and sealed inside the contract's WebAssembly (`sealPassword`, `sealSshKey`), so a secret
   never returns to JavaScript. A server already saved at the address keeps its settings and
   takes the new secret. A server added with the agent key's line by hand is saved without one.
   The panel marks an agent server the vault does not keep and offers to save it.
3. **The editor keeps a typed password.** The connection editor has a password field for
   *Automatic* and *Password*. Saving keeps the password in the window's vault through the
   `SavedPasswords` port (the account's vault in a hub window, this computer's otherwise) before
   the connection is saved, and the connection names the record. A record keeps its id when its
   password changes while the address stays the same; *Forget* unlinks it. The raw *Credential ID*
   field is removed.
4. **An account window keeps a prompt's answer.** In a window of an account the password prompt's
   *Keep in the account's vault* box is on by default; this computer's prompt still asks.
5. **No key files for an account.** A hub window offers *Vault key* instead of *Key file*; a
   connection that already names a key file keeps the choice, with *Move to vault*.
6. **No separate list.** The sidebar no longer lists agent servers the window has not saved:
   `ServerAgentAccess::unsaved`, `UnsavedAgentServer` and the seeded editor are removed.

## Alternatives rejected

- **An account SSH key installed on every server.** Devices would sign in without any password,
  and the hub could even install the line with the agent key. It changes what is written to
  servers behind the person's back, and still leaves servers that refuse keys; the person's own
  password or key, kept end to end, is what they expect.
- **Give devices the agent key.** The hub's key for agents would leave the hub and every device
  would become a way to sign in as the agents.
- **Keep the list of unsaved agent servers.** It is the second list that made a saved server
  look unsaved; the web panel, where such servers come from, now repairs them.
- **Open credentials in the browser.** The panel is served by the hub, so a secret it opens is
  one a compromised hub could read; it only seals.

## Consequences

- A server connected in the web panel, or created in a hub window with its password or key,
  opens without asking on every computer of the account once its keys are open.
- A server connected in the web panel before this change has no secret: the panel marks it and
  *Save to vault* asks for it once; or the first prompt in a hub window keeps it.
- The web panel's keys module grows by the OpenSSH key parser (`ssh-key`, the version native
  clients use).
