# 0042 — Explicit terminal installation and retained providers

Status: accepted. Complements [0007](0007-bootstrap.md),
[0032](0032-narrow-terminal-ports.md) and [0038](0038-private-terminal-state-owners.md).

## Context

Terminal transport, local factory and credential compatibility globals made
construction order implicit. Existing models could discover replacement providers
or settings instead of retaining the collaborators used for their connection.
Authentication requires the existing Send+Sync callback contract and an isolated
local command transport, while windows share recording and settings services.

## Decision

`nocterm-terminal` owns immutable `Deps`, `LocalSessions`, `CredentialProvider`
and `Installed` values with private fields. Installation requires `UiReady` and
retains its actual SettingsHandle along with transport, local factory, credential
and recording ports in an Arc. Models receive this installation explicitly;
views derive it from their model. A private SessionFactory retained by each
workspace delegates all four existing opening routes to the same installation.
There are no terminal globals or late provider setters.

Startup prepares native local transport construction and default-shell discovery
once. Root composition creates recording and vault providers before installation,
captures the actual Connections entity for typed credential association, and then
binds the existing authentication factory. Its Send+Sync bounds remain unchanged;
only naturally compatible handles cross that callback boundary. No App, window,
plaintext credential, unsafe cast or mutex-wrapped Rc is captured.

Model and view subscriptions retain weak owners and observe published settings
revisions. Terminal rendering uses `TerminalStyle::with_settings`; native/default
shell selection and launch settings are captured until reconnect. Command and
program transport overrides retain precedence. Credential connection/prompt
checks, live leases, completion events and shared recording ownership remain as
specified by their existing contracts.

## Consequences

All construction sites supply dependencies explicitly, including fixtures and
new windows. Absent optional providers are explicit and preserve their existing
unavailable behavior. Existing Agent authentication and workspace port APIs stay
bounded. Accepted earlier ADRs retain historical initialization details; current
installation uses `install` and `register`.
