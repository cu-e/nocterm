# Nocterm modifications

Base: the authenticated crates.io publication `agent-client-protocol` 2.2.0,
SHA-256 `13f780597b2c88ab83d02509bcc3e86366751f86fa12114005e35c55ed691763`.
Its `.cargo_vcs_info.json` records upstream revision
`2a78849d3eb3dcb140dade3b8fc938cf1e2b9ce5`, path `src/agent-client-protocol`.
The published archive, rather than a raw Git checkout, is the exact replay base.
The original Apache-2.0 license and notices remain unchanged.

The additive `managed` module supplies explicitly budgeted physical source,
collection, weak-header, request, response and dispatch ownership for a native
controlled codec. It creates no SDK executor task or hidden futures channel.
The typed SDK connection remains available with its original behavior.
Payload-independent pending and batch lifecycle transitions were extracted into
`jsonrpc/routing.rs` and shared by the existing typed owners and managed owners.
Native JSON validation, method decoding/encoding, process I/O, task ownership and
source-lease installation belong to the adapter using the managed API. No
uncontrolled typed decoder is used by the managed module.

Each new allocation requests its concrete layout through an explicit Budget
before allocation. Reservations remain with original physical owners; shared
header reservations survive final strong release when weak handles remain.
Managed public futures/receivers carry original raw payload ownership through
cancellation, response consumption and ordered dispatch acknowledgement.

Source replay and native behavioral verification remain separate obligations.
The existence of this module does not itself prove an application's total peak,
its native adapter integration, or an externally supplied Budget implementation.

The next ordered patch adds provisional managed request admission. It pays the
same pending registry, outgoing FIFO position and original reply slot before a
coordinator commits document state, then activates publication on first poll
without allocation. Unpolled cancellation, duplicate IDs and premature replies
preserve the original correlation owner. The responder also exposes a borrowed
original ID so native reply encoding does not fabricate response authority.
