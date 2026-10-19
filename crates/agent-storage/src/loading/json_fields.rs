//! Key classification retains only an inline prefix; managed stock unit scratch is prepaid.
use super::string_reader::{JsonStringReader, StringReadError};
use std::io::Read;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::loading) enum Key {
    Version,
    Format,
    Output,
    Arguments,
    Result,
    Extensions,
    ClientDisplay,
    Presence,
    Tool,
    Destination,
    Redacted,
    Outcome,
    OutcomePending,
    OutcomeOk,
    OutcomeErr,
    Document,
    Metadata,
    Resume,
    Transcript,
    Composer,
    Id,
    AgentId,
    Updated,
    Entries,
    Queue,
    Attachments,
    Times,
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
    Kind,
    Value,
    Type,
    ToolCallId,
    Status,
    Content,
    Locations,
    RawInput,
    RawOutput,
    Meta,
    TerminalId,
    Path,
    OldText,
    NewText,
    Text,
    Data,
    MimeType,
    Resource,
    Annotations,
    Uri,
    Blob,
    Description,
    Size,
    Audience,
    Priority,
    LastModified,
    RawValue,
    Encoded,
    Mime,
    ReportedSize,
    Body,
    Modified,
    Previous,
    Replacement,
    Line,
    Images,
    Unknown,
}
impl Key {
    pub(in crate::loading) fn spelling(self) -> &'static str {
        match self {
            Self::Encoded => "encoded",
            Self::Mime => "mime",
            Self::ReportedSize => "reported_size",
            Self::Body => "body",
            Self::Modified => "modified",
            Self::Previous => "previous",
            Self::Replacement => "replacement",
            Self::Line => "line",
            Self::Images => "images",
            Self::Version => "version",
            Self::Format => "format",
            Self::Output => "output",
            Self::Arguments => "arguments",
            Self::Result => "result",
            Self::Extensions => "extensions",
            Self::ClientDisplay => "client_display",
            Self::Presence => "presence",
            Self::Tool => "tool",
            Self::Destination => "destination",
            Self::Redacted => "redacted",
            Self::Outcome => "outcome",
            Self::OutcomePending => "Pending",
            Self::OutcomeOk => "Ok",
            Self::OutcomeErr => "Err",
            Self::Document => "document",
            Self::Metadata => "metadata",
            Self::Resume => "resume",
            Self::Transcript => "transcript",
            Self::Composer => "composer",
            Self::Id => "id",
            Self::AgentId => "agent_id",
            Self::Updated => "updated",
            Self::Entries => "entries",
            Self::Queue => "queue",
            Self::Attachments => "attachments",
            Self::Times => "times",
            Self::Title => "title",
            Self::Name => "name",
            Self::Pinned => "pinned",
            Self::SessionId => "session_id",
            Self::Workdir => "workdir",
            Self::Model => "model",
            Self::Config => "config",
            Self::Mode => "mode",
            Self::PendingHistory => "pending_history",
            Self::Draft => "draft",
            Self::Kind => "kind",
            Self::Value => "value",
            Self::Type => "type",
            Self::ToolCallId => "toolCallId",
            Self::Status => "status",
            Self::Content => "content",
            Self::Locations => "locations",
            Self::RawInput => "rawInput",
            Self::RawOutput => "rawOutput",
            Self::Meta => "_meta",
            Self::TerminalId => "terminalId",
            Self::Path => "path",
            Self::OldText => "oldText",
            Self::NewText => "newText",
            Self::Text => "text",
            Self::Data => "data",
            Self::MimeType => "mimeType",
            Self::Resource => "resource",
            Self::Annotations => "annotations",
            Self::Uri => "uri",
            Self::Blob => "blob",
            Self::Description => "description",
            Self::Size => "size",
            Self::Audience => "audience",
            Self::Priority => "priority",
            Self::LastModified => "lastModified",
            Self::RawValue => "$serde_json::private::RawValue",
            Self::Unknown => "",
        }
    }
    fn classify(bytes: &[u8]) -> Self {
        match bytes {
            b"encoded" => Self::Encoded,
            b"mime" => Self::Mime,
            b"reported_size" => Self::ReportedSize,
            b"body" => Self::Body,
            b"modified" => Self::Modified,
            b"previous" => Self::Previous,
            b"replacement" => Self::Replacement,
            b"line" => Self::Line,
            b"images" => Self::Images,

            b"version" => Self::Version,
            b"format" => Self::Format,
            b"output" => Self::Output,
            b"arguments" => Self::Arguments,
            b"result" => Self::Result,
            b"extensions" => Self::Extensions,
            b"client_display" => Self::ClientDisplay,
            b"presence" => Self::Presence,
            b"tool" => Self::Tool,
            b"destination" => Self::Destination,
            b"redacted" => Self::Redacted,
            b"outcome" => Self::Outcome,
            b"Pending" => Self::OutcomePending,
            b"Ok" => Self::OutcomeOk,
            b"Err" => Self::OutcomeErr,
            b"document" => Self::Document,
            b"metadata" => Self::Metadata,
            b"resume" => Self::Resume,
            b"transcript" => Self::Transcript,
            b"composer" => Self::Composer,
            b"id" => Self::Id,
            b"agent_id" => Self::AgentId,
            b"updated" => Self::Updated,
            b"entries" => Self::Entries,
            b"queue" => Self::Queue,
            b"attachments" => Self::Attachments,
            b"times" => Self::Times,
            b"title" => Self::Title,
            b"name" => Self::Name,
            b"pinned" => Self::Pinned,
            b"session_id" => Self::SessionId,
            b"workdir" => Self::Workdir,
            b"model" => Self::Model,
            b"config" => Self::Config,
            b"mode" => Self::Mode,
            b"pending_history" => Self::PendingHistory,
            b"draft" => Self::Draft,
            b"kind" => Self::Kind,
            b"value" => Self::Value,
            b"type" => Self::Type,
            b"toolCallId" => Self::ToolCallId,
            b"status" => Self::Status,
            b"content" => Self::Content,
            b"locations" => Self::Locations,
            b"rawInput" => Self::RawInput,
            b"rawOutput" => Self::RawOutput,
            b"_meta" => Self::Meta,
            b"terminalId" => Self::TerminalId,
            b"path" => Self::Path,
            b"oldText" => Self::OldText,
            b"newText" => Self::NewText,
            b"text" => Self::Text,
            b"data" => Self::Data,
            b"mimeType" => Self::MimeType,
            b"resource" => Self::Resource,
            b"annotations" => Self::Annotations,
            b"uri" => Self::Uri,
            b"blob" => Self::Blob,
            b"description" => Self::Description,
            b"size" => Self::Size,
            b"audience" => Self::Audience,
            b"priority" => Self::Priority,
            b"lastModified" => Self::LastModified,
            b"$serde_json::private::RawValue" => Self::RawValue,
            _ => Self::Unknown,
        }
    }
}
/// Reads the entire current encoded key even after it becomes unknown: an invalid
/// trailing escape must remain a syntax failure, including in an ignored field.
/// Caller must authenticate the enclosing source after this replay completes.
#[cfg(test)]
pub(in crate::loading) fn read_key<R: Read>(
    source: R,
    span_bytes: usize,
) -> Result<(Key, R), StringReadError> {
    read_key_with_ledger(source, span_bytes, None)
}
pub(in crate::loading) fn read_key_with_ledger<R: Read>(
    source: R,
    span_bytes: usize,
    ledger: Option<&nocterm_ai::model::json::deferred::DecodeLedger>,
) -> Result<(Key, R), StringReadError> {
    let mut cursor = match ledger {
        Some(ledger) => JsonStringReader::new_managed(source, span_bytes, ledger)?,
        None => JsonStringReader::new(source, span_bytes)?,
    };
    let mut prefix = [0; 32];
    let mut used = 0;
    let mut overflow = false;
    while let Some(fragment) = cursor.next_fragment()? {
        let bytes = fragment.as_bytes();
        if bytes.len() <= prefix.len() - used && !overflow {
            prefix[used..used + bytes.len()].copy_from_slice(bytes);
            used += bytes.len();
        } else {
            overflow = true;
        }
    }
    let key = if overflow {
        Key::Unknown
    } else {
        Key::classify(&prefix[..used])
    };
    Ok((key, cursor.into_inner()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;
    #[test]
    fn escaped_reserved_and_legacy_keys_are_classified_from_actual_bytes() {
        for (encoded, expected) in [
            (br#""raw\u0049nput""#.as_slice(), Key::RawInput),
            (
                br#""\u0024serde_json::private::RawValue""#.as_slice(),
                Key::RawValue,
            ),
            (br#""toolCallId""#.as_slice(), Key::ToolCallId),
        ] {
            let (key, remainder) = read_key(Cursor::new(encoded), encoded.len()).unwrap();
            assert_eq!(key, expected);
            assert_eq!(remainder.position() as usize, encoded.len());
        }
    }
    #[test]
    fn large_unknown_key_is_fully_validated_with_fixed_owned_buffers() {
        let encoded = format!("\"{}\\uD834\\uDD1E\"", "x".repeat(1024 * 1024));
        let (key, source) = read_key(Cursor::new(encoded.as_bytes()), encoded.len()).unwrap();
        assert_eq!(key, Key::Unknown);
        assert_eq!(source.position() as usize, encoded.len());
    }
    #[test]
    fn unknown_key_does_not_hide_invalid_unicode_after_prefix_overflow() {
        let encoded = format!("\"{}\\uD800\"", "x".repeat(128));
        assert!(serde_json::from_str::<String>(&encoded).is_err());
        assert!(read_key(Cursor::new(encoded.as_bytes()), encoded.len()).is_err());
    }
}
