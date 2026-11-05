# Hubs

A hub is a self-hosted Nocterm server. It keeps your saved servers and vault passwords so they
follow you to your other computers, and it can carry the SSH connection to a host your computer
cannot reach.

A window shows one environment: this computer, or the account of one hub. You sign in to a
server and manage it on the **Servers** page of the settings.

## Switch between this computer and a server

The title bar has an environment menu after the Help menu. It lists **Local**, every server you
have signed in to with its status, and **Sign in to server…**. Choosing a server shows it in
this window; the arrow beside it opens the server in a new window. If the window has open tabs,
Nocterm asks before replacing them. A server window shows only the connections of its account
(your own and your organizations') and keeps their passwords in the account's vault; a local
window shows only the connections of this computer. Workspaces, files, containers, snippets and
the other features work over the connections of the window you are in. Finishing a sign-in opens
the server in the window that started it, or in a new window when that window has tabs.

## Sign in to a server

1. Run **Hubs: Sign in to server** from the command palette, or press **Sign in to a server…** on
   the Servers page of the settings.
2. Enter the address, `hub.example.com` or `hub.example.com:7443` (the default port is 7443).
   `http://` addresses are refused; a leading `https://` is accepted and dropped.
3. Optionally enter the certificate's SHA-256 fingerprint. The server's setup prints it: run
   `docker compose logs setup` in the server's directory and copy the value of the certificate
   line. A fingerprint you enter is checked against the certificate the server presents, and
   Nocterm continues only if they match.
4. Choose how to trust the server. Without a fingerprint:
   - a certificate issued by a public authority is trusted as such; nothing is pinned, so
     routine renewals do not matter;
   - a self-signed certificate is shown with its fingerprint. Compare it with the one the
     server printed, and choose **Trust and pin this certificate** only if they are the same. This is trust on
     first use, like an SSH host key.
5. Sign in with the account's name and password, or with **Continue with** and the provider's
   name for single sign-on. Single sign-on opens your browser; finish the login there. The
   address is also shown in the dialog with a copy button, in case the browser did not open.
   Cancelling the dialog ends the wait.
6. Your account keys open as part of signing in:
   - The first time an account signs in, Nocterm creates its keys and shows a **recovery key**
     once. Write it down: it is the only way to open the keys if you lose your password, and it
     is neither kept on this device nor shown again. The dialog cannot be closed on this step
     until you choose **I have saved it**. If the window is lost anyway (a crash, the computer
     turned off), the next time the keys open Nocterm offers to **issue a new recovery key**,
     here or on the Servers page; the old one stops working and the new one is shown once the
     same way.
   - After single sign-on, the password of the provider does not open the keys, so Nocterm asks
     for a **vault passphrase** of at least eight characters. The first time, this sets it.
   - If the account's password was reset on the server, Nocterm asks for the **recovery key**; the
     new password opens the keys from then on.
7. Choose **Done**.

The server is now on the Servers page. Nocterm keeps its address, how it is trusted and its name in
`settings.toml` under `[hubs]`, which contains no passwords or tokens:

```toml
[[hubs.servers]]
address = "hub.example.com:7443"
fingerprint = "ab:cd:..."   # or: trust = "public-ca"
label = "Home"
color = "blue"              # optional: red, yellow, green, cyan, blue or magenta
```

The device key and the tokens of each server are in a private file (readable by you only) in
a `hubs` folder of Nocterm's state directory, apart from the settings. They stay when you
close Nocterm, so you sign in once per server and device. They are deleted when you sign out,
when the server revokes the device, and when you remove the server.

## The window of a hub

A window of a hub shows the connections and the vault of your account on that hub, and only
those. The window of this computer shows only the connections and the vault of this computer.
Workspaces, containers, files, the monitor, snippets, the agent and plugins are features of this
computer in every window; they work over the connections the window shows.

- The connections of a hub window are your own (**Personal**) and those of your organizations.
  A connection you create there is saved on the hub, encrypted with your account's keys, and the
  password or key you enter goes into the vault of the hub. They never appear in the window of
  this computer.
- The server keeps only encrypted data. The keys come from your account password (or the vault
  passphrase) on your computers; the server cannot read your passwords, keys or connection
  details. If you lose both the password and the recovery key, that data cannot be recovered.
- When the account's keys are closed (after a restart or an automatic lock) the list shows
  **Account locked - Unlock**; **Unlock** asks for the password of that hub's vault. If the
  server cannot be reached the list shows the last one received and cannot be changed; after
  you sign out it is empty and offers **Sign in**. After a restart the list stays empty until
  the server answers.
- Connections are copied both ways while the window is in step with the hub. A change on the
  server that does not verify is reported as an error and applied to nothing.
- Signing out of a server, or removing it, empties its windows and locks its vault. Nothing of
  the account stays on this computer. Profiles that an earlier build copied into the list of
  this computer remain there as ordinary local connections; remove `hub-replica.json` from the
  state directory and any such connection you do not want.
- The recordings of sessions are kept on this computer in plain text, as for any session.

### Connections and passwords of a group

An administrator of an organization shares connections and vault passwords with a group by
creating the group and adding you with `hubctl`; Nocterm only shows and edits what the group
holds. A group you were added to appears only after you confirm its administrator's key: the
**Servers** page of the settings lists each administrator whose key is waiting, with the
fingerprint the server offers. Compare it with the fingerprint the administrator sees in
`hubctl`, in person or over a channel you trust, and press **Confirm key** only if they are
identical; the group then appears after the next synchronization, which starts at once. If the
administrator's key later changes, the page warns you, shows the old and the new fingerprints,
and the group stays hidden until you confirm the new key the same way.

- The Servers panel lists each organization in its own section below your own connections. A
  section marked **read-only** is one you are a Guest in: its connections open, but have no edit,
  delete or drag handles.
- As a Member, Admin or Owner you also see a **Space** choice in the connection editor, next to
  **Folder**. **Personal** keeps the connection to yourself; an organization's name shares it with
  the group. Choosing another space moves the connection: it is removed from the old one and
  written to the new one, and the move yields to a change someone else made in the meantime.
- Saved vault passwords and keys of a group are listed in the vault with the group's name, and
  **read-only** for a Guest. Changing one writes it back to its group, never to your own vault.
  A key or password you create for a connection starts in your own vault. It reaches a group only
  when you save a change to the connection from the editor with an organization's space chosen
  (the editor says so beside **Save**): it then moves into that group's vault, sealed with the group's key,
  so the other members can use it, and it is no longer in yours (unless a personal connection
  still uses it, below). A password saved while connecting stays in your own vault, even for an
  organization's connection; open the connection in the editor and save it to share it. Moving a connection from one
  space to another moves its keys and passwords with it, before the connection itself; a space
  you may not write refuses both, and nothing moves. A key or password that another connection
  staying behind still uses is copied instead of moved, under a new id: the connection that
  moves follows the copy, and the two vaults then hold independent entries, so editing or
  deleting one never reaches the other.
- When the server refuses a change (your role was lowered, or the group was closed), the connection
  returns to the version on the server and a single notice says so; a new connection the server
  refuses stays in your personal space. Nothing is retried in a loop.
- Renaming, disbanding or deleting a folder acts on the connections of the space you clicked it
  in; a folder of the same name in another space is left alone.

## Status

| Status | Meaning | What the Servers page offers |
| --- | --- | --- |
| Online | Connected | The status |
| Connecting | Reaching the server | The status |
| Offline · retry in N s | The server did not answer; Nocterm retries with growing delays | A **Try again** button |
| Sign-in required | No stored credentials | **Sign in** |
| Device revoked | The server removed this device; credentials were deleted | **Sign in** |
| Certificate changed | The certificate is not the one you trusted | Both fingerprints and removal |
| Update Nocterm | The server speaks a protocol this version does not | The versions |

A changed certificate is never accepted from the dialog. It can mean a reinstalled server or
someone intercepting the connection; verify with the server's administrator. If the change is
legitimate, remove the server and sign in again, comparing the new fingerprint with the one
the server prints.

## Reach a host through a server

A connection can reach its host through a server you are signed in to, which is useful when the
server can see hosts your computer cannot. In the connection editor, open **Proxy** and choose
**Nocterm hub**:

- **Hub address** is the server's address as in the sign-in dialog, for example
  `hub.example.com`. You must be signed in to that server on this computer; Nocterm does not
  sign in for you, and the connection says so when you are not.
- **Node (optional)** names one of the server's nodes. The connection is then opened from inside
  that node's network. Leave it empty to let the server open it.

The server only forwards bytes. The SSH session is still yours: the host key is checked against
`known_hosts` for the host you connect to, and you sign in to that host as usual. The server
sees the address you connect to, when, and how much data crosses, not what is inside. It may
refuse a destination, for example one on a private network, or a connection when you have
reached a limit; the connection then fails with that reason. With jump hosts configured, the
server carries only the first connection.

## Manage servers

**Settings → Servers** (or the command **Hubs: Manage servers**) shows the known servers in a
list on the left (colour mark, name and a dot in the colour of the status) with **Sign in to a
server…** below it. The first server is selected; choosing another shows it from its first tab.
The selected server has four tabs:

- **Overview**
  - **Name** is what the server is called. Edit it and press Enter, or leave the field, to
    save. An empty name shows the address again.
  - **Colour** marks the server: its icon takes the colour in the current theme. **None** clears
    it.
  - The status reads as above, with **Try again** while the server is offline, and the
    **certificate** below: the pinned fingerprint, or a note that a public authority vouches for
    the server. When the certificate changed, the fingerprint the server presents is shown next
    to the trusted one. Fingerprints are only shown; nothing on this page accepts a certificate.
  - **Sign out** deletes this device's credentials for the server and keeps it in the list, so
    the page offers to sign in again. It is offered while this device is signed in.
  - **Remove…** asks, then deletes the credentials and the entry. For a changed certificate this
    is the only way forward: remove the server and sign in again, comparing the new fingerprint
    with the one the server prints.
- **Devices** lists the devices signed in to your account on the server: name, kind, when it
  was last seen and a **This device** marker. **Revoke** asks first, then signs the device out of
  the account; revoking this device signs it out of the server like **Sign out**.
- **Keys & trust** offers **Issue a new recovery key** while the recovery key made with your
  account keys was never confirmed as saved, and the administrator keys that wait for your
  confirmation: compare each fingerprint with the one the administrator sees in `hubctl`, out of
  band, and confirm only when they are identical. A changed key is shown with a warning and
  both fingerprints.
- **Cloud agents** lists the agents the server offers to your account (personal, organisation
  and server agents) with their status. **New personal agent** opens a form for a name, a model
  provider (and optional endpoint), a model, your API key and the pool that runs it. The key is
  sealed to the pool and is never readable by the server; to edit an agent type the key again.
  **Delete** removes a personal agent. **Manage agents on the web** opens the server's panel.
  A server that does not manage agents says so.

### Cloud agents in the chat menu

In the window of a server, the **+** menu of the assistant panel has **Local** and **Cloud**
tabs. **Cloud** lists the agents of that server, with a search field, sections per scope and a
status chip per agent. The window of this computer has no **Cloud** tab: open the server's
window to chat with its agents. Pick one to start it: the steps are shown,
and the first start on a pool asks you to compare the pool's fingerprint with the one its
administrator gives you before it is trusted. **New personal agent…** opens the Servers page.
The tab is hidden while no server offers agents.

Earlier Cloud chats of the server are listed in the chat history (the clock button of the
assistant panel), after your local chats, and the search finds them by name, agent and model.
Opening one wakes its agent if it sleeps. Sending a message to a chat whose agent fell asleep
wakes it too; until it answers, the chat shows "Reconnecting to the agent". If a message cannot
reach the agent, send it again to retry it; if the agent refuses it, sending it again sends a
new message.

A Cloud chat can sign in to your own servers. The **Servers** bar above its composer shows the
servers the chat is bound to and where each stands: *waiting for an unlocked device*, *active
until* a time, *not reachable by this agent*, *refused*, *expired* or *revoked*. **Bind
servers…** lists the servers that trust your agent CA (see **Agent access (SSH)** below); tick
them, pick how long the binding lasts (1, 7, 14 or 30 days, 7 unless you choose) and press
**Bind**. A server you have not connected to since the application started cannot be ticked
yet: connect to it once so its port is known. **Prompts run on** picks the server your next
messages run on (**Default** is the agent's own place). **Unbind** ends the binding at once.
While this device is signed in and its account keys are unlocked, it renews the agent's short
certificates for every bound chat, also when the chat is not open; with no such device online,
access lapses when the last certificate ends. History rows name the servers a chat is bound to,
and the search finds them.

- **Agent access (SSH)** controls how cloud agents sign in to your own servers. A personal CA
  (shown with its fingerprint and date) signs certificates that are valid for hours at most,
  only from the runner's address, with a terminal and no forwarding. The CA key stays in your
  encrypted account data; the hub never reads it.
  - **Servers that trust the CA** lists each server with `user@host`, when it was added and by
    which device, when it was last checked, and its state: *Trusted*, *Removal pending* (access
    was denied while the server was out of reach; the line is removed the next time you
    connect to it), *Drifted* (the line vanished without you asking) or removed. **Deny
    access** acts on one row; tick several rows and use **Deny on selected**.
  - **Allow on servers…** lists the servers you are connected to now; ticked ones get the CA
    line in the `authorized_keys` of the session's user, over that session. Other lines are
    kept. Each server's line admits only that server: **Copy line** on its row copies it to add
    by hand. A server that still has the line an earlier version wrote says *Needs a CA update*;
    connecting to it once rewrites the line, after which chat certificates open it.
  - **Reissue CA…** asks, then replaces the CA; servers that can be reached move to the new one.
  - **Active chat accesses** lists the accesses agents hold, each with **Revoke**.

While this device is not signed in to a server, **Devices**, **Keys & trust** and **Agent
access (SSH)** only offer **Sign in**.

A change that cannot be saved to `settings.toml` is reported and leaves the list as it was.

The command palette also has **Hubs: Sign in to server** and **Hubs: Sign out of server** (it asks
which server in a list).
