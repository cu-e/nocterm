# Hubs

A hub is a self-hosted Nocterm server. It keeps terminal sessions running on its nodes, so a
session you start from one computer is still there when you open it from another. Nocterm
reaches a hub from the Sessions panel of any window.

Hubs are an addition to the window you already use: its **Sessions** panel lists the sessions of
a hub, and each one opens as a terminal tab beside your local tabs. A window has no hub of its own.

## Sign in to a server

1. Run **Hubs: Sign in to server** from the command palette, or press **Sign in** in the
   Sessions panel when no server is known.
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
     is neither kept on this device nor shown again.
   - After single sign-on, the password of the provider does not open the keys, so Nocterm asks
     for a **vault passphrase** of at least eight characters. The first time, this sets it.
   - If the account's password was reset on the server, Nocterm asks for the **recovery key**; the
     new password opens the keys from then on.
7. Choose **Done**.

The server is now in the Sessions panel and on the Servers page. Nocterm keeps its address, how it is trusted and its name in
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

## Keep your servers and vault passwords on a server

One server can keep your saved servers (connections) and your vault passwords so they follow you
to your other computers. On **Settings → Servers**, press **Use for my account** on a server you
are online with; **Stop keeping them here** returns both to this computer alone. The choice is
saved as `account = "hub.example.com:7443"` under `[hubs]` and takes effect at once.

- The server keeps only encrypted data. The keys come from your account password (or the vault
  passphrase) on your computers; the server cannot read your passwords, keys or connection
  details. If you lose both the password and the recovery key, that data cannot be recovered.
- While a server is chosen, the vault is the account's: unlock it with the account password (or
  the vault passphrase). It cannot be created or have its password changed here, and device unlock
  is not offered. The vault of this computer is locked and comes back when you stop using the
  account. Credentials can be read while the server is out of reach; changing them needs the
  server.
- Saved connections are copied both ways. Something other clients put on the server that this
  version does not understand is left untouched.
- A change on the server that does not verify is reported as an error and applied to nothing.
- Choosing another server starts the sync again from that server's own data; connections are never
  copied from one account to another.

### Connections and passwords of a group

An administrator of an organization shares connections and vault passwords with a group by
creating the group and adding you with `hubctl`; Nocterm only shows and edits what the group
holds. Groups you were added to appear after the next synchronization (a reconnect or the next
change).

- The Servers panel lists each organization in its own section below your own connections. A
  section marked **read-only** is one you are a Guest in: its connections open, but have no edit,
  delete or drag handles.
- As a Member, Admin or Owner you also see a **Space** choice in the connection editor, next to
  **Folder**. **Personal** keeps the connection to yourself; an organization's name shares it with
  the group. Choosing another space moves the connection: it is removed from the old one and
  written to the new one, and the move yields to a change someone else made in the meantime.
- Saved vault passwords and keys of a group are listed in the vault with the group's name, and
  **read-only** for a Guest. Changing one writes it back to its group, never to your own vault.
  A key or password you create for a connection starts in your own vault; when the connection is
  saved in an organization's space it moves into that group's vault, sealed with the group's key,
  so the other members can use it, and it is no longer in yours. Moving a connection from one
  space to another moves its keys and passwords with it, before the connection itself; a space
  you may not write refuses both, and nothing moves. A key or password that a connection shares
  with another one moves with whichever connection moves first.
- When the server refuses a change (your role was lowered, or the group was closed), the connection
  returns to the version on the server and a single notice says so; a new connection the server
  refuses stays in your personal space. Nothing is retried in a loop.
- Renaming, disbanding or deleting a folder acts on the connections of the space you clicked it
  in; a folder of the same name in another space is left alone.

## Sessions panel

The **Sessions** panel (command **Hubs: Open sessions**; it is offered in the tray of the window)
lists the nodes of a hub and the sessions on each. With several known servers it shows a button
for each; the panel keeps the one you picked, and otherwise lists the first server that
answers. Open a session with its **Open** button, a double-click, or by selecting it (click, or
Up and Down) and pressing Enter. Create one with the **+** button in the panel header, or the one
beside an online node. A new session starts on any available node or on that node.

Each session is a terminal tab of the window. It reconnects by itself after a network drop (the
tab shows "Connecting") and shows the screen as the session holds it when you open it again.
Quitting Nocterm detaches the tabs; the sessions keep running on their nodes, and Nocterm does not
reopen them at the next start. Open them again from the panel.

## Closing a tab and ending a session

Closing a tab **detaches** from the session: the program keeps running on the node, and you
can open it again from the Sessions panel, also from another computer. To stop the program,
use the **End session** button (the cross) next to the session and confirm; this ends the process on the node.

## Status

| Status | Meaning | What the panel offers |
| --- | --- | --- |
| Online | Connected | The nodes and sessions |
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

**Settings → Servers** (or the command **Hubs: Manage servers**) lists every known server:

- **Name** is what the Sessions panel shows. Edit it and press Enter, or leave the
  field, to save. An empty name shows the address again.
- **Colour** marks the server: its icon in the Sessions panel takes the colour in the current
  theme. **None** clears it.
- The status reads as above, with the **certificate** below: the pinned fingerprint, or a
  note that a public authority vouches for the server. When the certificate changed, the
  fingerprint the server presents is shown next to the trusted one. Fingerprints are only shown;
  nothing on this page accepts a certificate.
- **Sign out** deletes this device's credentials for the server and keeps it in the list, so the
  panel offers to sign in again. It is offered while this device is signed in.
- **Remove…** asks, then deletes the credentials and the entry. Sessions on the server keep
  running. For a changed certificate this is the only way forward: remove the server and sign in
  again, comparing the new fingerprint with the one the server prints.

A change that cannot be saved to `settings.toml` is reported and leaves the list as it was.

The command palette also has **Hubs: Sign in to server**, **Hubs: Sign out of server** (it asks
which server in a list) and **Hubs: Open sessions**.
