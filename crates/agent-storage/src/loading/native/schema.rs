//! Occurrence-sensitive typed scopes over authenticated source spans.
use super::Error;
mod native;
use crate::loading::json_frames::RootFrames;
use crate::loading::{
    held_file::{
        HeldFile,
        plan::{AuthenticatedPlan, NativeSpan},
    },
    json_fields::Key,
    json_frames::{Element, Member},
};
pub(in crate::loading) use native::NativeScope;
use nocterm_ai::model::json::deferred::native::NativeKind;
use nocterm_ai::model::json::deferred::native::NativeNode;
use std::cell::{Cell, RefCell};
use std::io::Read;

pub(super) enum Field {
    Named(Member),
    Positional(Element),
}
impl Field {
    pub(super) fn span(&self) -> NativeSpan {
        match self {
            Self::Named(v) => v.native_span(),
            Self::Positional(v) => v.native_span(),
        }
    }
    pub(super) fn start(&self) -> usize {
        match self {
            Self::Named(v) => v.value().start(),
            Self::Positional(v) => v.value().start(),
        }
    }
}
/// A schema description has no caller-sized heap or caller-supplied allocation hint.
#[derive(Clone, Copy)]
pub(super) struct Schema {
    keys: &'static [Key],
    required: &'static [Key],
    positional: usize,
    deny_unknown: bool,
    defer_duplicates: bool,
    sequence_max: usize,
    map_only: bool,
}
impl Schema {
    pub(super) const fn new(
        keys: &'static [Key],
        required: &'static [Key],
        positional: usize,
    ) -> Self {
        Self {
            keys,
            required,
            positional,
            deny_unknown: false,
            defer_duplicates: false,
            sequence_max: keys.len(),
            map_only: false,
        }
    }
    pub(super) const fn max_sequence(mut self, count: usize) -> Self {
        self.sequence_max = count;
        self
    }
    pub(super) const fn map_only(mut self) -> Self {
        self.map_only = true;
        self
    }
    pub(super) const fn deferred(mut self) -> Self {
        self.defer_duplicates = true;
        self
    }
    pub(super) const fn strict(mut self) -> Self {
        self.deny_unknown = true;
        self
    }
}
pub(in crate::loading) struct Scope {
    schema: Schema,
    fields: [Option<Field>; 18],
    duplicates: u32,
    native: Option<NativeScope>,
    semantic: RefCell<Option<NativeNode>>,
    captured: Cell<bool>,
}
impl Scope {
    #[cfg(test)]
    pub(super) fn scan(
        source: &mut dyn Read,
        start: usize,
        schema: Schema,
    ) -> std::result::Result<Self, Error> {
        Self::scan_with_ledger(source, start, schema, None)
    }
    pub(super) fn scan_with_ledger(
        source: &mut dyn Read,
        start: usize,
        schema: Schema,
        ledger: Option<&nocterm_ai::model::json::deferred::DecodeLedger>,
    ) -> std::result::Result<Self, Error> {
        let mut frames = (if ledger.is_some() {
            RootFrames::new_literal_managed(source, start)
        } else {
            RootFrames::new_literal(source, start)
        })
        .map_err(|error| match error {
            crate::loading::json_frames::FrameError::Punctuation => Error::Type,
            other => Error::Frame(other),
        })?;
        let mut scope = Self {
            schema,
            fields: std::array::from_fn(|_| None),
            duplicates: 0,
            native: None,
            semantic: RefCell::new(None),
            captured: Cell::new(false),
        };
        match &mut frames {
            RootFrames::Object(map) => {
                while let Some(field) = map.next_member_with_ledger(ledger)? {
                    let key = field.kind();
                    let Some(index) = schema.keys.iter().position(|k| *k == key) else {
                        if schema.deny_unknown {
                            return Err(Error::UnknownField);
                        }
                        continue;
                    };
                    if scope.fields[index].is_some() {
                        if !schema.defer_duplicates {
                            return Err(Error::Duplicate(key));
                        }
                        scope.duplicates |= 1 << index;
                    }
                    scope.fields[index] = Some(Field::Named(field));
                }
                for key in schema.required {
                    scope.required(*key)?;
                }
            }
            RootFrames::Array(array) => {
                if schema.map_only {
                    return Err(Error::Type);
                }
                let mut count = 0;
                while let Some(field) = array.next_element()? {
                    if count >= schema.sequence_max {
                        return Err(Error::Positional);
                    }
                    scope.fields[count] = Some(Field::Positional(field));
                    count += 1;
                }
                if count < schema.positional {
                    return Err(Error::Positional);
                }
            }
        }
        frames.finish()?;
        Ok(scope)
    }
    pub(in crate::loading) fn adopt(native: NativeScope) -> Self {
        Self {
            schema: native.schema(),
            fields: std::array::from_fn(|_| None),
            duplicates: 0,
            native: Some(native),
            semantic: RefCell::new(None),
            captured: Cell::new(false),
        }
    }
    pub(super) fn is_native(&self) -> bool {
        self.native.is_some()
    }
    pub(super) fn has(&self, key: Key) -> bool {
        if let Some(native) = &self.native {
            return native.has(key);
        }
        if key == Key::Value && self.captured.get() {
            return true;
        }
        self.field(key).is_some()
    }
    pub(super) fn take_native(&self, key: Key) -> Option<std::result::Result<NativeNode, Error>> {
        if let Some(native) = &self.native {
            return Some(native.take(key));
        }
        if key == Key::Value && self.captured.get() {
            return Some(
                self.semantic
                    .try_borrow_mut()
                    .map_err(|_| Error::Consumed)
                    .and_then(|mut v| v.take().ok_or(Error::Consumed)),
            );
        }
        None
    }
    pub(super) fn native_kind(&self, key: Key) -> Option<std::result::Result<NativeKind, Error>> {
        if let Some(native) = &self.native {
            return Some(native.kind(key));
        }
        if key == Key::Value && self.captured.get() {
            return Some(
                self.semantic
                    .try_borrow()
                    .map_err(|_| Error::Consumed)
                    .and_then(|v| v.as_ref().map(|n| n.kind()).ok_or(Error::Consumed)),
            );
        }
        None
    }
    pub(super) fn capture(&self, node: NativeNode) -> std::result::Result<(), Error> {
        if self.captured.replace(true) {
            return Err(Error::Consumed);
        }
        *self
            .semantic
            .try_borrow_mut()
            .map_err(|_| Error::Consumed)? = Some(node);
        Ok(())
    }
    pub(super) fn validate(
        &self,
        keys: &[Key],
        required: &[Key],
    ) -> std::result::Result<(), Error> {
        for key in keys {
            if let Some(i) = self.schema.keys.iter().position(|k| k == key)
                && self.duplicates & (1 << i) != 0
            {
                return Err(Error::Duplicate(*key));
            }
        }
        for key in required {
            self.required(*key)?;
        }
        Ok(())
    }
    pub(super) fn field(&self, key: Key) -> Option<&Field> {
        self.schema
            .keys
            .iter()
            .position(|k| *k == key)
            .and_then(|i| self.fields[i].as_ref())
    }
    pub(super) fn required(&self, key: Key) -> std::result::Result<&Field, Error> {
        self.field(key).ok_or(Error::Missing(key))
    }
}
pub(super) fn nested(
    parent: &AuthenticatedPlan<Scope>,
    held: &mut HeldFile,
    key: Key,
    schema: Schema,
    ledger: &nocterm_ai::model::json::deferred::DecodeLedger,
) -> std::result::Result<AuthenticatedPlan<Scope>, Error> {
    if let Some(node) = parent.payload().take_native(key) {
        return Ok(parent.native_scope(NativeScope::read(node?, schema)?));
    }
    let field = parent.payload().required(key)?;
    let start = field.start();
    parent
        .derive(
            held,
            |_| field.span(),
            |reader| Scope::scan_with_ledger(reader, start, schema, Some(ledger)),
        )?
        .transpose()
}
use Key::*;
pub(super) const TAG: Schema = Schema::new(&[Kind, Value], &[Kind, Value], 2);
pub(super) const ENVELOPE: Schema = Schema::new(
    &[Format, Version, Document],
    &[Format, Version, Document],
    3,
)
.strict();
pub(super) const DOCUMENT: Schema = Schema::new(
    &[Id, Metadata, Resume, Transcript, Composer],
    &[Id, Metadata, Resume, Transcript, Composer],
    5,
);
pub(super) const METADATA: Schema = Schema::new(
    &[AgentId, Title, Name, Pinned, Updated, Model],
    &[AgentId, Pinned, Updated],
    6,
);
pub(super) const RESUME: Schema = Schema::new(
    &[SessionId, Workdir, Config, Mode, PendingHistory],
    &[Config, PendingHistory],
    5,
);
pub(super) const TRANSCRIPT: Schema = Schema::new(&[Entries, Times], &[Entries], 1);
pub(super) const COMPOSER: Schema = Schema::new(&[Draft, Queue, Attachments], &[], 1);
pub(super) const PROMPT: Schema = Schema::new(&[Id, Text, Images, Attachments], &[Id, Text], 2);
pub(super) const TEXT: Schema = Schema::new(&[Text, Annotations, Extensions], &[Text], 3);
pub(super) const MEDIA: Schema = Schema::new(
    &[Kind, Encoded, Mime, Uri, Annotations, Extensions],
    &[Kind, Encoded, Mime],
    6,
);
pub(super) const REFERENCE: Schema = Schema::new(
    &[
        Uri,
        Name,
        Title,
        Description,
        Mime,
        ReportedSize,
        Annotations,
        Extensions,
    ],
    &[Uri, Name],
    8,
);
pub(super) const EMBEDDED: Schema = Schema::new(&[Body, Annotations, Extensions], &[Body], 3);
pub(super) const RESOURCE_TEXT: Schema =
    Schema::new(&[Uri, Text, Mime, Extensions], &[Uri, Text], 4);
pub(super) const RESOURCE_BLOB: Schema =
    Schema::new(&[Uri, Encoded, Mime, Extensions], &[Uri, Encoded], 4);
pub(super) const ANNOTATIONS: Schema =
    Schema::new(&[Audience, Modified, Priority, Extensions], &[], 4);
pub(super) const TOOL: Schema = Schema::new(
    &[
        Id,
        Title,
        Name,
        Kind,
        Status,
        Output,
        Locations,
        Arguments,
        Result,
        Extensions,
        ClientDisplay,
    ],
    &[
        Id, Title, Kind, Status, Output, Locations, Arguments, Result,
    ],
    11,
);
pub(super) const DIFF: Schema = Schema::new(
    &[Path, Previous, Replacement, Extensions],
    &[Path, Replacement],
    4,
);
pub(super) const LOCATION: Schema = Schema::new(&[Path, Line, Extensions], &[Path], 3);
pub(super) const OUTPUT_CONTENT: Schema = Schema::new(&[Content, Extensions], &[Content], 2);
pub(super) const OUTPUT_TERMINAL: Schema = Schema::new(&[Id, Extensions], &[Id], 2);
pub(super) const PRESENCE: Schema = Schema::new(&[Presence, Value], &[Presence], 2);
pub(super) const DISPLAY: Schema = Schema::new(
    &[Version, Tool, Arguments, Destination, Redacted, Outcome],
    &[Version, Tool, Arguments, Destination, Redacted, Outcome],
    6,
);
pub(super) const OUTCOME: Schema =
    Schema::new(&[OutcomePending, OutcomeOk, OutcomeErr], &[], 0).strict();

pub(super) const ATTACHMENT: Schema = Schema::new(&[Kind, Id], &[Kind, Id], 2);

pub(super) const LEGACY_DOCUMENT: Schema = Schema::new(
    &[
        Version,
        Id,
        AgentId,
        Title,
        Name,
        Pinned,
        SessionId,
        Workdir,
        Model,
        Config,
        Mode,
        PendingHistory,
        Draft,
        Updated,
        Entries,
        Queue,
        Attachments,
        Times,
    ],
    &[Version, Id, AgentId, Updated, Entries],
    18,
);
pub(super) const LEGACY_CALL: Schema = Schema::new(
    &[
        ToolCallId, Title, Name, Kind, Status, Content, Locations, RawInput, RawOutput, Meta,
    ],
    &[ToolCallId, Title],
    2,
);
pub(super) const LEGACY_BLOCK: Schema = Schema::new(
    &[
        Type,
        Text,
        Data,
        MimeType,
        Uri,
        Name,
        Title,
        Description,
        Size,
        Resource,
        Annotations,
        Meta,
    ],
    &[Type],
    1,
)
.deferred()
.map_only();
pub(super) const LEGACY_RESOURCE: Schema =
    Schema::new(&[MimeType, Text, Uri, Meta, Blob], &[Uri], 3)
        .deferred()
        .max_sequence(4);
pub(super) const LEGACY_OUTPUT: Schema = Schema::new(
    &[Type, Content, Path, OldText, NewText, TerminalId, Meta],
    &[Type],
    1,
)
.deferred()
.map_only();
pub(super) const LEGACY_LOCATION: Schema = Schema::new(&[Path, Line, Meta], &[Path], 1);
pub(super) const LEGACY_ANNOTATIONS: Schema =
    Schema::new(&[Audience, Priority, LastModified, Meta], &[], 0);
