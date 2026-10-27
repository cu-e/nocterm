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
6. Choose **Done**.

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
