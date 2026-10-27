# Hubs

A hub is a self-hosted Nocterm server. It keeps terminal sessions running on its nodes, so a
session you start from one computer is still there when you open it from another. Nocterm
reaches a hub from the title bar of any window.

A window is always in one environment: **Local** (the menu calls it "This computer"), the
application as you know it, or one hub. A hub window shows only that hub's sessions, never local tabs, connections or files.

## Sign in to a server

1. Open the environment menu at the left of the title bar (it reads "This computer") and
   choose **Sign in to a server…**.
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
6. Choose **Open here**, **Open in a new window** or **Done**.

The server is now in the menu. Nocterm keeps its address, how it is trusted and its name in
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

## Open a hub

Choose the server in the environment menu. Enter opens it in this window; Ctrl+Enter, or the
button at the end of the row, opens it in a new window. If this window holds local tabs,
Nocterm asks first, because switching ends their sessions; **Open in a new window** is the
default. A hub window can be switched without a question, because its sessions keep running
on the server. **New Window** from the menu always opens a local window.

After a restart Nocterm opens a window for each environment the previous run left open, in the
same order. A server that was removed from the list is skipped; if no window can open, one
local window opens. The windows open on their environment, not on their tabs.

The window is named "Nocterm — server name".

## What a hub window offers

- The **Sessions** panel lists the hub's nodes and the sessions on each. Open a session with
  its **Open** button, a double-click, or by selecting it (click, or Up and Down) and pressing
  Enter. Create one with the **+** button in the panel header or on the tab bar, on a
  chosen node or on any available node.
- Each session is a terminal tab. It reconnects by itself after a network drop (the tab
  shows "Connecting") and shows the screen as the session holds it when you open it again.
- Snippets, Settings, Appearance, Keymap and Plugins work as in a local window.

Connections, Files, Containers, Workspaces, the Monitor, the AI agent and user plugins are
local features and do not appear in a hub window.

## Closing a tab and ending a session

Closing a tab **detaches** from the session: the program keeps running on the node, and you
can open it again from the Sessions panel, also from another computer. To stop the program,
use the **End session** button (the cross) next to the session and confirm; this ends the process on the node.

## Status in the menu

| Status | Meaning | What happens on click |
| --- | --- | --- |
| Online | Connected | Opens the window |
| Connecting | Reaching the server | Opens the window, which shows the status |
| Offline · retry in N s | The server did not answer; Nocterm retries with growing delays | Opens the window, which has a **Try again** button |
| Sign-in required | No stored credentials | Opens the sign-in dialog |
| Device revoked | The server removed this device; credentials were deleted | Opens the sign-in dialog |
| Certificate changed | The certificate is not the one you trusted | Shows both fingerprints |
| Update Nocterm | The server speaks a protocol this version does not | Shows the versions |

A changed certificate is never accepted from the dialog. It can mean a reinstalled server or
someone intercepting the connection; verify with the server's administrator. If the change is
legitimate, remove the server and sign in again, comparing the new fingerprint with the one
the server prints.

## Manage servers

**Settings → Servers** (or the command **Hubs: Manage servers**) lists every known server:

- **Name** is what the menu and the window title show. Edit it and press Enter, or leave the
  field, to save. An empty name shows the address again.
- **Colour** marks the server: the icon in the environment menu, and the icon of the menu button
  in a window of that server, take the colour in the current theme. **None** clears it.
- The status reads as in the menu, with the **certificate** below: the pinned fingerprint, or a
  note that a public authority vouches for the server. When the certificate changed, the
  fingerprint the server presents is shown next to the trusted one. Fingerprints are only shown;
  nothing on this page accepts a certificate.
- **Sign out** deletes this device's credentials for the server and keeps it in the list, so the
  next click on it asks for a sign-in. It is offered while this device is signed in.
- **Remove…** asks, then deletes the credentials and the entry. Sessions on the server keep
  running. For a changed certificate this is the only way forward: remove the server and sign in
  again, comparing the new fingerprint with the one the server prints.

A change that cannot be saved to `settings.toml` is reported and leaves the list as it was.

The command palette also has **Hubs: Sign in to server**, **Hubs: Open server in new window** and
**Hubs: Sign out of server**; the last two ask which server in a list.
