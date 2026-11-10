# Hubs

A hub is a self-hosted Nocterm server. It keeps your saved servers and vault passwords so they
follow you to your other computers, it can carry the SSH connection to a host your computer
cannot reach, and it runs your agents: their chats run on the hub and continue when your
computer is off.

A window shows one environment: this computer, or the account of one hub. You sign in to a
server and manage it on the **Servers** page of the settings.

## Switch between this computer and a server

The title bar has an environment menu after the Help menu. It lists **Local**, every server you
have signed in to with its status, and **Sign in to server…**. Choosing a server shows it in
this window; the arrow beside it opens the server in a new window. If the window has open tabs,
Nocterm asks before replacing them. A server window shows only the connections of its account
and keeps their passwords in the account's vault; a local window shows only the connections of
this computer. Workspaces, files, containers, snippets and the other features work over the
connections of the window you are in. Finishing a sign-in opens the server in the window that
started it, or in a new window when that window has tabs.

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
     server printed, and choose **Trust and pin this certificate** only if they are the same.
     This is trust on first use, like an SSH host key.
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

[hubs]
key_lock_minutes = 0        # close the account keys after this many idle minutes; 0: never
```

The device key and the tokens of each server are in a private file (readable by you only) in
a `hubs` folder of Nocterm's state directory, apart from the settings. They stay when you
close Nocterm, so you sign in once per server and device. They are deleted when you sign out,
when the server revokes the device, and when you remove the server.

## The window of a hub

A window of a hub shows the connections and the vault of your account on that hub, and only
those. The window of this computer shows only the connections and the vault of this computer.
Workspaces, containers, files, the monitor, snippets and plugins are features of this computer
in every window; they work over the connections the window shows.

- A connection you create in a hub window is saved on the hub, encrypted with your account's
  keys, and the password you type in its **Password** field, or the **Vault key** it signs in
  with, goes into the account's vault on the hub. Your other computers open the server without
  asking once their account keys are open. A password prompt in a hub window keeps its answer
  there too (*Keep in the account's vault after sign in*, on by default). A password saved
  under a name can sign in to several servers: choose it under **Saved passwords** in the
  editor, and manage the names on the vault's **Credentials** tab or on the **Vault** page of
  the web panel. A hub window offers
  no *Key file*: a file stays on one computer, so import it with **Vault key** instead. None of
  this appears in the window of this computer.
- The server keeps only encrypted data. The keys come from your account password (or the vault
  passphrase) on your computers; the server cannot read your passwords, keys or connection
  details. If you lose both the password and the recovery key, that data cannot be recovered.
- The account keys follow their own lock policy, `key_lock_minutes` under `[hubs]`, not the
  auto-lock of this computer's vault. By default they stay open until you lock them on the
  **Keys** tab or sign out. Chats and agents never need them.
- When the account's keys are closed the list shows **Account locked - Unlock**; **Unlock** asks
  for the password of that hub's vault. If the server cannot be reached the list shows the last
  one received and cannot be changed; after you sign out it is empty and offers **Sign in**.
- Connections are copied both ways while the window is in step with the hub. A change on the
  server that does not verify is reported as an error and applied to nothing.
- Signing out of a server, or removing it, empties its windows and locks its vault. Nothing of
  the account stays on this computer.
- The recordings of sessions are kept on this computer in plain text, as for any session.

## Agents of a hub

The agents of your account run on the hub. Their chats look and work like the chats of a local
agent; only where the agent runs differs.

- **Start a chat.** In the window of a hub, the **+** menu of the assistant panel lists the
  hub's agents beside the agents of this computer. The window of this computer does not list
  them: open the hub's window to chat with its agents. Create agents on the **Agents** tab of
  the Servers page.
- **The chat runs on the hub.** A message you send is carried out there, with the agent's
  instructions, model and servers; closing the window or turning the computer off does not
  stop it. Files and images you attach are uploaded with the message. **Stop** ends the run.
- **Approvals.** An agent set to *Ask before changes* asks before it changes something. The
  request shows in the chat with **Allow**, **Allow in chat** (no more questions for the rest
  of this chat) and **Deny**. Any of your devices can answer it.
- **The model.** The model chooser of the composer lists the models of the agent's provider;
  choosing one changes the model of the chat on the hub.
- **Servers of the chat.** The agent signs in only to the servers attached to its chat. The
  attach menu (**+** beside the message field) lists the servers of the account with a check
  beside the attached ones; the attached servers also show as chips above the message field,
  and a chip's cross detaches its server. Servers attached before the first message are
  attached to the chat it creates. The choice is stored with the chat on the hub, so the web
  panel and your other computers show the same servers.
- **History.** The chat history of a hub window lists the hub's chats after the local ones;
  renaming, pinning and deleting a hub chat change it on the hub. Opening one reads its history
  from the hub, so the same chat continues on any of your computers.
- **Other devices.** When a message is sent to the chat from another device, the chat shows
  that a run is in progress and fills in as it advances; the composer waits until it ends.

### Agent access of a server

Agents sign in to your servers with the account's **agent key**, one key per account. A server
is open to them once its `authorized_keys` holds the key's line and the hub knows the server.

- In the window of a hub, open a saved server in the connection editor and turn on **Agent
  access** (under **Agents**). Nocterm adds the agent key's line to the `authorized_keys` of
  the user you connect as and records the server on the hub with the host key this computer
  recorded for it; agents refuse a server that presents another key. Other lines of the file
  are kept byte for byte.
- The line is written through the session you have open to the server. Without one, Nocterm
  connects in the background with the connection's saved credentials, as opening it from the
  sidebar would, and closes that session again when it is done. If the server asks you to sign
  in, confirm its host key or unlock the vault, its tab opens: finish there and access is added.
  A server that cannot be reached, or a sign-in that does not finish within a few minutes, is
  said under the switch and changes nothing.
- Turning **Agent access** off removes the server's record on the hub, so agents stop using it
  at once, and removes the line while a session to the server is open. Without a session the
  line stays in `authorized_keys`: connect and remove it by hand if the key should no longer
  sign in there.
- In the sidebar of a hub window, each saved server agents may use shows what they last found
  there beside its name: *Agents* (not checked yet), *Agents ✓*, *No access*, *Host key
  changed*, *Unreachable* or *Reconnect* (the agent key was replaced; turn Agent access off and
  on again).
- The **Agent access** tab of the Servers page lists the servers with their status (*Agents ✓*,
  *No access — reconnect*, *Host key changed*, *Unreachable*, *Reconnect needed*), **Check** to
  try the agent key again and **Disconnect** to remove one.
- A server connected in the hub's web panel is saved to the account like a connection saved
  here, with the password or key typed there sealed in the vault, so it is an ordinary saved
  server on every computer and opens without asking. A server added there with the agent key's
  line by hand is saved without a secret: the first prompt keeps it. A server connected in the
  web panel before it saved servers has no connection here; the panel marks it *Not in your
  vault* and **Save to vault** asks for its password once.

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
**Nocterm hub**, then enter the **Hub address** as in the sign-in dialog, for example
`hub.example.com`. You must be signed in to that server on this computer; Nocterm does not sign
in for you, and the connection says so when you are not.

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
The selected server has five tabs:

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
- **Keys** opens and locks the account keys on this device (**Unlock** with the password or
  vault passphrase, **Lock account keys**), and offers **Issue a new recovery key** while the
  recovery key made with your account keys was never confirmed as saved.
- **Agents** lists the account's agents and **Model providers**. **New agent** opens a form for
  a name, a provider and default model, instructions, how it asks before changes (*Auto* or
  *Ask before changes*) and the default servers it may use. **Add your own** adds a provider
  (Anthropic or OpenAI-compatible, with a base URL, an API key and its models); **Test** checks
  it. A provider's API key is kept by the hub and never shown again; leave the field empty to
  keep the stored one. Providers the hub's administrator set up are listed but cannot be edited.
- **Agent access** shows the agent key's fingerprint with **Copy line** (to add the line by
  hand) and **Replace key**, which asks first: servers need the new line before agents reach
  them again. Below it are the servers agents may use, as described above.

While this device is not signed in to a server, **Devices**, **Keys**, **Agents** and **Agent
access** only offer **Sign in**.

A change that cannot be saved to `settings.toml` is reported and leaves the list as it was.

The command palette also has **Hubs: Sign in to server** and **Hubs: Sign out of server** (it asks
which server in a list).
