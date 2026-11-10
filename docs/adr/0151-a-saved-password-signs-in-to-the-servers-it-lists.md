# 0151. A saved password signs in to the servers it lists

Status: accepted. Extends [0150](0150-an-accounts-servers-keep-their-secrets-in-its-vault.md);
keeps the binding rule of [0016](0016-vault-storage-boundary.md) that a secret answers only what
it is bound to.

## Context

Every password record was bound to one address (`CredentialBinding::Password { target }`), so
the same password used on several machines was saved once per machine, under the address as
its name, and renaming, adding one on its own or seeing which servers use it was not possible
in the app or the web panel. Simply letting any connection name any password would undo the
binding rule: a connection whose address was changed, by a person or by a tampered profile,
would be answered with a password typed for another machine.

## Decision

1. **A password lists its addresses.** `CredentialBinding::Password { targets }` answers a
   request only for an address among `targets` (`CredentialBinding::answers`); a password saved
   from a prompt lists its own address, one saved under a name lists none until a connection
   chooses it. The list is inside the record, sealed with it, so only someone who can write the
   vault can extend it.
2. **The stored form stays readable.** A password of one address is written as `target`, as
   before; any other as `targets` (`{"kind":"password","targets":[…]}`). Older clients read the
   first and leave the second unread, never overwriting it.
3. **Records change without their secret.** `VaultAccess::update(id, label, binding)` renames a
   record and, for a password, replaces its addresses, with the secret unchanged
   (`CredentialBinding::may_become`); a key and a key passphrase keep their binding. The local
   vault, the account's vault and the test vault implement it.
4. **Where people see it.** The app's Vault page lists records by name with what each signs in
   to: add a named password, rename, detach a server, delete. The connection editor offers the
   vault's passwords under **Saved passwords**; saving a connection that chose one adds its
   address (`SavedPasswords::attach`), and a password typed for it replaces the secret of the
   record it names, which every server it lists then uses. The web panel's **Vault** page does
   the same for the account: the contract's WebAssembly opens a record only to return its name
   and binding, and seals it again with its secret for a rename or new addresses.

## Alternatives rejected

- **Unbound named passwords.** Any connection could ask for any password, so a changed address
  would receive a password meant for another machine.
- **A separate record of a password's servers.** Two objects to keep consistent, and the hub
  would have to be trusted with which servers share a password, or a second sealed index kept.
- **Replace a shared password per server when a new one is typed.** It turns a shared password
  into copies that drift apart; rotating the shared password once is what sharing is for.

## Consequences

- One password can be rotated once for every server that uses it; detaching a server from it
  or deleting it makes that server ask again.
- A password shared by several servers, or saved for none, is not visible to versions before
  this one.
