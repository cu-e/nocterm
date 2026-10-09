# Adapter layer

Part of the [architecture overview](../ARCHITECTURE.md).

Adapters implement domain contracts with real I/O and depend only on the
domain and foundation: `nocterm-ssh`, `nocterm-local`, `nocterm-acp` and
`nocterm-device-unlock`. The composition root chooses and injects them
([app layer](app.md)).

## SSH and local sessions

`nocterm-ssh` implements
that contract using russh and lazily opened SFTP channels on a dedicated Tokio
runtime.
`nocterm-local` adapts portable-pty to
the same session events and launch contract. Integration supplies bounded OSC 7
cwd and prompt markers without parsing rendered prompt text.

## Bounded I/O

Input and output channels are bounded. An independent broadcast close signal
interrupts blocked SSH I/O and local PTY writes/output delivery. Unix local PTY
reads/writes are nonblocking and independently cancellable, including when a
background job retains its slave descriptor. Local closure terminates the shell
and current foreground process groups and reaps the owned child; it does not
enumerate and kill arbitrary detached jobs. Explicit input refusal
is surfaced in the terminal instead of silently discarding text.
HTTP CONNECT/SOCKS5 tunnel creation belongs to the SSH adapter, inside the same
connection timeout and target host-key policy as a direct connection.

## Device unlock

`nocterm-vault` declares the device-unlock provider contract and owns envelope,
binding and cancellation rules. The composition root injects
`nocterm-device-unlock`; native adapters remain outside the vault and its UI.
An optional `nocterm-vault-broker` Linux system service enforces fingerprint
verification for user-bound memory-only keys. See [device unlock](../DEVICE_UNLOCK.md)
for the platform boundaries and explicit installation.
