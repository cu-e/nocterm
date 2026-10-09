# 0003. Terminals reach saved credentials through a port

## Context

The terminal depended on the vault directly and polled it while a password
prompt waited for an unlock.

## Decision

`nocterm-session` declares a `CredentialStore` port. `nocterm-vault-ui` adapts
the vault to it, maps requests to vault bindings, and publishes vault status
changes to waiting listeners instead of being polled. The composition root
installs the adapter with `nocterm_terminal::init_credentials`.

## Alternatives rejected

- Moving credential bindings into session: they are vault policy.

## Consequences

- There is no terminal → vault edge; the terminal's tests use a fake store.
- `CredentialBinding` stays in the vault crate.
