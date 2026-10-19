//! Managed history payloads with genuine weak physical topology.
//!
//! Owns: the sole actual typed row/prompt slabs and their payload retirement.
//! Does not own native parsing, scheduling, authority lookup or file I/O.
//! Projection handles retain structure/receipts but never the payload-life token.
mod bundle;
mod composer;
mod composition;
pub use composition::ComposerInputEdit;
mod dispatch;
pub(crate) use dispatch::DispatchProjection;
pub use dispatch::{DispatchPayload, DispatchStaging};
mod content;
mod extras;
mod framing;
mod presentation;
pub use presentation::{
    CategorySlot, CommandSlot, ConfigChoiceSlot, ConfigGroupSlot, ConfigKindSlot, ConfigOptionSlot,
    ConfigSelectionSlot, LiveControls, LivePresentation, LiveUsage, PresentationUpdate,
    RateLimitSlot, SelectChoicesSlot,
};
mod inputs;
pub(crate) mod local;
mod parts;
mod policy;
mod rows;
mod slots;
mod snapshot;
mod source_pool;
mod tools;
pub use composer::{AttachmentSlot, ComposerSlot, PromptSlot, QueueSelection};
pub use content::{
    AnnotationSlot, ContentSlot, EmbeddedContent, MediaContent, ReferenceContent, ResourceBlobSlot,
    ResourceSlot, ResourceTextSlot, TextContent,
};
pub use framing::{ChoiceSlot, ChoiceText, ChoiceValueSlot, MetadataSlot, ResumeSlot};
pub use inputs::{ImageBinaryGuard, LiveAttachmentSlot, LiveInputs, ManagedImage};
pub use parts::{ManagedInstallError, ManagedParts};
pub use rows::{ChangedRows, RowBody, RowSlot, TranscriptSlot, UserSlot};
pub use slots::{
    ExtensionSlot, PathGuard, PathSlot, PayloadGuard, PhysicalError, TextGuard, TextSlot,
};
pub(crate) use snapshot::SnapshotProjection;
pub use snapshot::{PreparedGuard, SnapshotBuilder, SnapshotOwner, SnapshotStaging};
pub use tools::{
    ClaimSlot, ClaimsSlot, DiffSlot, LocationSlot, OutputSlot, TerminalSlot, ToolSlot,
};

pub(crate) mod wire;
pub use wire::SnapshotWire;
