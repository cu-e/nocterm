//! Native durable storage for headless Cloud runs.
//!
//! Owns: SQLite journal transactions, writer fencing, command deduplication,
//! checkpoint/outbox atomicity, bounded replay, complete authenticated account
//! catalog membership, root-protected current heads, atomic signed-baseline
//! staging/receipt/activation and explicit native creation/opening.
//! CurrentCloudHeads reads complete revision-bound commitments without writer
//! authority or content keys. Its optional receipt collaborator is weak, so a
//! metadata reader cannot prolong content-key retention.
//! Does not own: cryptography, signed hub formats, SSH, provider SDKs, GPUI,
//! device credentials or remote execution. Ports provided: RunJournalWriter and
//! JournalReader and CurrentCloudHeads. Ports used: JournalCipher, account/runner-bound
//! JournalCatalogCipher and independent JournalChatHeadCipher, injected explicitly. Its implementation
//! must check full native account/runner pins before mapping contract coordinates.
//! Settings sections: none. Globals: none.
//! Invariants: one known account/runner per private physical journal; existing
//! databases require their authenticated complete catalog; success follows a
//! synchronous durable transaction; account plus
//! chat is the unique stream key; stale writers never append; ciphertext records
//! retain their original storage context. The supervisor supplies a private
//! trusted directory with trusted ancestry; hostile ancestor replacement is not
//! protected by SQLite's pathname VFS. Native archives never stand in for a real
//! signed public Snapshot; missing publication anchors refuse public retention.
//! Snapshot publication keeps covered native prefix P, signed marker S and relay
//! cursor T separate. FIFO uploads preserve committed delivery IDs on retries;
//! network/purge awaits release the common native write gate. Activation retains
//! later history, current writer and tombstones. Charged public/native generations
//! remain owned through blocking completion and the last returned carrier.
//! Command input/publication share actual ordinary/urgent quota owners; immutable
//! admitted control commands keep urgent publication capacity, and terminal facts
//! use a separate finite cleanup reserve that snapshots cannot consume.
//! All blocking work is polled off UI threads.
//! Initial sessions use an injected native broker. Durable Established precedes
//! actual physical publication, current-source/writer activation and original-run
//! wake. Native recovery restores descriptions as Lost; no receipt restores live
//! execution authority. Terminal session loss uses the same SQLite writer/gate
//! through a separate fact-only cleanup port, without changing control revision.

mod codec;
pub mod coordinator;
mod journal;
mod publication_budget;
mod runtime;
mod storage;
pub use publication_budget::CoordinatorPublicationBudgets;
pub use runtime::NativeToolTimer;

pub use journal::{
    DurableCurrentCloudHeads, DurableJournalCleanup, DurableRunJournal, DurableSnapshotPublication,
    JournalLimits,
};
