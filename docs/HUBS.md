# Hubs

A hub is a self-hosted Nocterm server. It keeps terminal sessions running on its nodes, so a
session you start from one computer is still there when you open it from another. Nocterm
reaches a hub from the title bar of any window.

A window is always in one environment: **Local**, the application as you know it, or one
hub. A hub window shows only that hub's sessions, never local tabs, connections or files.

## Sign in to a server

1. Open the environment menu at the left of the title bar (it reads "Local") and choose
   **Sign in to a server**.
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
     server printed, and choose **Trust and pin** only if they are the same. This is trust on
     first use, like an SSH host key.
5. Sign in with the account's name and password, or with **Sign in with** and the provider's
   name for single sign-on. Single sign-on opens your browser; finish the login there. The
   address is also shown in the dialog with a copy button, in case the browser did not open.
   Cancelling the dialog ends the wait.
6. Choose **Open in this window**, **Open in a new window** or **Done**.

The server is now in the menu. Nocterm keeps its address, how it is trusted and its name in
`settings.toml` under `[hubs]`, which contains no passwords or tokens:

```toml
[[hubs.servers]]
address = "hub.example.com:7443"
fingerprint = "ab:cd:..."   # or: trust = "public-ca"
label = "Home"
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
on the server. **New Window** from the menu always opens a local window. Windows are not
restored after a restart; choose the server again.

The window is named "Nocterm — server name".

## What a hub window offers

- The **Sessions** panel lists the hub's nodes and the sessions on each. Open a session with
  a double-click or Enter, and create one with **New session** or the **+** button on the tab
  bar, on a chosen node or on any available node.
- Each session is a terminal tab. It reconnects by itself after a network drop (the tab
  shows "Connecting") and shows the screen as the session holds it when you open it again.
- Snippets, Settings, Appearance, Keymap and Plugins work as in a local window.

Connections, Files, Containers, Workspaces, the Monitor, the AI agent and user plugins are
local features and do not appear in a hub window.

## Closing a tab and ending a session

Closing a tab **detaches** from the session: the program keeps running on the node, and you
can open it again from the Sessions panel, also from another computer. To stop the program,
use **End** next to the session and confirm; this ends the process on the node.

## Status in the menu

| Status | Meaning | What happens on click |
| --- | --- | --- |
| Online | Connected | Opens the window |
| Connecting | Reaching the server | Opens the window, which shows the status |
| No connection, retry in N s | The server did not answer; Nocterm retries with growing delays | Opens the window, which has a **Retry** button |
| Sign-in needed | No stored credentials | Opens the sign-in dialog |
| Device revoked | The server removed this device; credentials were deleted | Opens the sign-in dialog |
| Certificate changed | The certificate is not the one you trusted | Shows both fingerprints |
| New Nocterm version needed | The server speaks a protocol this version does not | Shows the versions |

A changed certificate is never accepted from the dialog. It can mean a reinstalled server or
someone intercepting the connection; verify with the server's administrator. If the change is
legitimate, remove the server and sign in again, comparing the new fingerprint with the one
the server prints.
