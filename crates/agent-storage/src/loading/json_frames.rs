//! Structural framing only. Stock serde remains the syntax/number/Unicode oracle.
//! No skipped span is a semantic-validation receipt or materialization permission.
use super::json_fields::Key;
use super::json_fields::read_key_with_ledger;
use super::string_reader::StringReadError;
use nocterm_core::SharedError;
use sha2::{Digest, Sha256};
use std::io::{self, Read};
#[derive(Clone, Debug, thiserror::Error)]
pub(in crate::loading) enum FrameError {
    #[error("JSON replay source failed: {0}")]
    Source(#[source] SharedError<io::Error>),
    #[error("JSON replay source has an original guarded failure")]
    ManagedSource,
    #[error("JSON replay ended within a structural frame")]
    Truncated,
    #[error("JSON replay structural punctuation changed")]
    Punctuation,
    #[error("JSON replay nesting exceeds its fixed frame stack")]
    Depth,
    #[error("JSON replay offset overflow")]
    Arithmetic,
    #[error("JSON key validation failed: {0}")]
    Key(#[source] StringReadError),
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::loading) struct Span {
    start: usize,
    end: usize,
}
impl Span {
    pub(in crate::loading) fn start(self) -> usize {
        self.start
    }
    pub(in crate::loading) fn len(self) -> usize {
        self.end - self.start
    }
}
pub(in crate::loading) struct Member {
    #[cfg(test)]
    key: Span,
    kind: Key,
    value: Span,
    receipt: SpanDigest,
}
/// Exact current-stream bytes covered by the value frame (including one scalar
/// delimiter consumed for lookahead). It is UNAUTHENTICATED until its full parent
/// stream has matched a held receipt; a bare SpanDigest authorizes no adoption.
pub(in crate::loading) struct SpanDigest {
    digest: [u8; 32],
    covered_end: usize,
}
impl SpanDigest {
    #[cfg(test)]
    pub(in crate::loading) fn digest(&self) -> [u8; 32] {
        self.digest
    }
    #[cfg(test)]
    pub(in crate::loading) fn covered_end(&self) -> usize {
        self.covered_end
    }
}
impl Member {
    pub(in crate::loading) fn native_span(&self) -> super::held_file::plan::NativeSpan {
        super::held_file::plan::NativeSpan::from_scan(
            self.value.start,
            self.value.len(),
            self.receipt.covered_end,
            self.receipt.digest,
        )
    }
    pub(in crate::loading) fn kind(&self) -> Key {
        self.kind
    }
    #[cfg(test)]
    pub(in crate::loading) fn receipt(&self) -> &SpanDigest {
        &self.receipt
    }
    #[cfg(test)]
    pub(in crate::loading) fn key(&self) -> Span {
        self.key
    }
    pub(in crate::loading) fn value(&self) -> Span {
        self.value
    }
}

/// One logical input position and actual consumed-value digest, shared by map
/// and sequence framing. It owns no list of fields or decoded strings.
struct FrameState<R> {
    source: R,
    offset: usize,
    pending: Option<(u8, usize)>,
    value_digest: Option<Sha256>,
    literal: bool,
    managed_marker: bool,
    marker_failed: bool,
}
impl<R: Read> FrameState<R> {
    fn new(source: R, start: usize) -> Self {
        Self {
            source,
            offset: start,
            pending: None,
            value_digest: None,
            literal: false,
            managed_marker: false,
            marker_failed: false,
        }
    }
    fn framed_value(&mut self, first: (u8, usize)) -> Result<(Span, SpanDigest), FrameError> {
        let mut digest = Sha256::new();
        digest.update([first.0]);
        self.value_digest = Some(digest);
        let value = self.value_span(first);
        let digest = self
            .value_digest
            .take()
            .ok_or(FrameError::Punctuation)?
            .finalize()
            .into();
        Ok((
            value?,
            SpanDigest {
                digest,
                covered_end: self.offset,
            },
        ))
    }
    fn byte(&mut self) -> Result<Option<(u8, usize)>, FrameError> {
        if self.marker_failed {
            return Err(FrameError::ManagedSource);
        }
        if let Some(next) = self.pending.take() {
            return Ok(Some(next));
        }
        let mut byte = [0];
        loop {
            match self.source.read(&mut byte) {
                Ok(0) => return Ok(None),
                Ok(_) => {
                    let offset = self.offset;
                    self.offset = offset.checked_add(1).ok_or(FrameError::Arithmetic)?;
                    if let Some(digest) = &mut self.value_digest {
                        digest.update(byte);
                    }
                    return Ok(Some((byte[0], offset)));
                }
                Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
                Err(error) if self.managed_marker => {
                    drop(error);
                    self.marker_failed = true;
                    return Err(FrameError::ManagedSource);
                }
                Err(error) => return Err(FrameError::Source(SharedError::from_error(error))),
            }
        }
    }
    fn required(&mut self) -> Result<(u8, usize), FrameError> {
        self.byte()?.ok_or(FrameError::Truncated)
    }
    fn nonwhite(&mut self) -> Result<(u8, usize), FrameError> {
        loop {
            let next = self.required()?;
            if !matches!(next.0, b' ' | b'\r' | b'\n' | b'\t') {
                return Ok(next);
            }
        }
    }
    fn string_span(&mut self, start: usize) -> Result<Span, FrameError> {
        let mut escaped = false;
        loop {
            let (byte, offset) = self.required()?;
            if escaped {
                escaped = false;
            } else if byte == b'\\' {
                escaped = true;
            } else if byte == b'"' {
                return Ok(Span {
                    start,
                    end: offset + 1,
                });
            }
        }
    }
    fn value_span(&mut self, first: (u8, usize)) -> Result<Span, FrameError> {
        let (byte, start) = first;
        if byte == b'"' {
            return self.string_span(start);
        }
        if matches!(byte, b'{' | b'[') {
            return self.container_span(byte, start);
        }
        if matches!(byte, b',' | b'}' | b']') {
            return Err(FrameError::Punctuation);
        }
        loop {
            let Some(next) = self.byte()? else {
                return Err(FrameError::Truncated);
            };
            if matches!(next.0, b',' | b'}' | b']' | b' ' | b'\r' | b'\n' | b'\t') {
                self.pending = Some(next);
                return Ok(Span { start, end: next.1 });
            }
        }
    }
    fn container_span(&mut self, opening: u8, start: usize) -> Result<Span, FrameError> {
        if self.literal {
            // Framing alone is not syntax validation. A checked scalar counter
            // can locate a deep literal span without retaining a per-depth
            // stack; current replay must still validate every bracket/type.
            let mut depth = 1usize;
            loop {
                let (byte, offset) = self.required()?;
                match byte {
                    b'"' => {
                        self.string_span(offset)?;
                    }
                    b'{' | b'[' => {
                        depth = depth.checked_add(1).ok_or(FrameError::Arithmetic)?;
                    }
                    b'}' | b']' => {
                        depth -= 1;
                        if depth == 0 {
                            return Ok(Span {
                                start,
                                end: offset + 1,
                            });
                        }
                    }
                    _ => {}
                }
            }
        }
        let mut closers = [0; 128];
        closers[0] = if opening == b'{' { b'}' } else { b']' };
        let mut depth = 1;
        loop {
            let (byte, offset) = self.required()?;
            match byte {
                b'"' => {
                    self.string_span(offset)?;
                }
                b'{' | b'[' => {
                    let slot = closers.get_mut(depth).ok_or(FrameError::Depth)?;
                    *slot = if byte == b'{' { b'}' } else { b']' };
                    depth += 1;
                }
                b'}' | b']' => {
                    if byte != closers[depth - 1] {
                        return Err(FrameError::Punctuation);
                    }
                    depth -= 1;
                    if depth == 0 {
                        return Ok(Span {
                            start,
                            end: offset + 1,
                        });
                    }
                }
                _ => {}
            }
        }
    }
}

pub(in crate::loading) enum RootFrames<R> {
    Object(ObjectFrames<R>),
    Array(ArrayFrames<R>),
}
impl<R: Read> RootFrames<R> {
    /// Held pass/span/BytePump owns every original native fault outside this
    /// synchronous structural cursor and checks it after parser locals retire.
    pub(in crate::loading) fn new_literal_managed(
        source: R,
        start: usize,
    ) -> Result<Self, FrameError> {
        Self::new_mode(source, start, true)
    }
    pub(in crate::loading) fn new_literal(source: R, start: usize) -> Result<Self, FrameError> {
        let mut frames = Self::new(source, start)?;
        match &mut frames {
            Self::Object(frame) => frame.state.literal = true,
            Self::Array(frame) => frame.state.literal = true,
        }
        Ok(frames)
    }
    pub(in crate::loading) fn finish(self) -> Result<(), FrameError> {
        let (mut state, finished) = match self {
            Self::Object(frame) => (frame.state, frame.finished),
            Self::Array(frame) => (frame.state, frame.finished),
        };
        if !finished {
            return Err(FrameError::Truncated);
        }
        while let Some((byte, _)) = state.byte()? {
            if !matches!(byte, b' ' | b'\r' | b'\n' | b'\t') {
                return Err(FrameError::Punctuation);
            }
        }
        Ok(())
    }

    pub(in crate::loading) fn new(source: R, start: usize) -> Result<Self, FrameError> {
        Self::new_mode(source, start, false)
    }
    fn new_mode(source: R, start: usize, managed: bool) -> Result<Self, FrameError> {
        let mut state = FrameState::new(source, start);
        state.managed_marker = managed;
        state.literal = managed;
        match state.nonwhite()?.0 {
            b'{' => Ok(Self::Object(ObjectFrames {
                state,
                first: true,
                finished: false,
            })),
            b'[' => Ok(Self::Array(ArrayFrames {
                state,
                first: true,
                finished: false,
                #[cfg(test)]
                index: 0,
            })),
            _ => Err(FrameError::Punctuation),
        }
    }
}
/// A current-pass object. Semantic validation/adoption still requires its actual
/// original scope and authenticated parent and replay receipts.
pub(in crate::loading) struct ObjectFrames<R> {
    state: FrameState<R>,
    first: bool,
    finished: bool,
}
impl<R: Read> ObjectFrames<R> {
    pub(in crate::loading) fn new_literal_managed(
        source: R,
        start: usize,
    ) -> Result<Self, FrameError> {
        match RootFrames::new_literal_managed(source, start)? {
            RootFrames::Object(map) => Ok(map),
            _ => Err(FrameError::Punctuation),
        }
    }
    #[cfg(test)]
    pub(in crate::loading) fn new(source: R, start: usize) -> Result<Self, FrameError> {
        match RootFrames::new(source, start)? {
            RootFrames::Object(map) => Ok(map),
            _ => Err(FrameError::Punctuation),
        }
    }
    #[cfg(test)]
    pub(in crate::loading) fn next_member(&mut self) -> Result<Option<Member>, FrameError> {
        self.next_member_with_ledger(None)
    }
    pub(in crate::loading) fn next_member_with_ledger(
        &mut self,
        ledger: Option<&nocterm_ai::model::json::deferred::DecodeLedger>,
    ) -> Result<Option<Member>, FrameError> {
        if ledger.is_some() {
            self.state.managed_marker = true;
        }
        if self.finished {
            return Ok(None);
        }
        let mut next = self.state.nonwhite()?;
        if next.0 == b'}' {
            self.finished = true;
            return Ok(None);
        }
        if !self.first {
            if next.0 != b',' {
                return Err(FrameError::Punctuation);
            }
            next = self.state.nonwhite()?;
        }
        self.first = false;
        if next.0 != b'"' {
            return Err(FrameError::Punctuation);
        }
        self.state.pending = Some(next);
        let (kind, _) = read_key_with_ledger(
            FrameInput(&mut self.state),
            nocterm_ai::history::MAX_FILE_BYTES as usize,
            ledger,
        )
        .map_err(FrameError::Key)?;
        #[cfg(test)]
        let key = Span {
            start: next.1,
            end: self.state.offset,
        };
        if self.state.nonwhite()?.0 != b':' {
            return Err(FrameError::Punctuation);
        }
        let first = self.state.nonwhite()?;
        let (value, receipt) = self.state.framed_value(first)?;
        Ok(Some(Member {
            #[cfg(test)]
            key,
            kind,
            value,
            receipt,
        }))
    }
    /// Guarded key construction for a semantic-key plan; ordinary framing keeps
    /// its fixed-buffer classifier and allocates no decoded key.
    pub(in crate::loading) fn next_guarded_member(
        &mut self,
        ledger: &nocterm_ai::model::json::deferred::DecodeLedger,
    ) -> Result<
        Option<(
            nocterm_ai::model::json::deferred::literal::LiteralText,
            Member,
        )>,
        super::native::legacy_value::ValueError,
    > {
        use nocterm_ai::model::json::deferred::literal::LiteralText;
        if self.finished {
            return Ok(None);
        }
        let mut next = self.state.nonwhite()?;
        if next.0 == b'}' {
            self.finished = true;
            return Ok(None);
        }
        if !self.first {
            if next.0 != b',' {
                return Err(FrameError::Punctuation.into());
            }
            next = self.state.nonwhite()?;
        }
        self.first = false;
        if next.0 != b'"' {
            return Err(FrameError::Punctuation.into());
        }
        self.state.pending = Some(next);
        let mut text = LiteralText::new(ledger)?;
        let mut string = super::string_reader::JsonStringReader::new_managed(
            FrameInput(&mut self.state),
            nocterm_ai::history::MAX_FILE_BYTES as usize,
            ledger,
        )
        .map_err(super::literal_reader::LiteralReadError::from)?;
        while let Some(fragment) = string
            .next_fragment()
            .map_err(super::literal_reader::LiteralReadError::from)?
        {
            text.append(fragment)?;
        }
        drop(string);
        #[cfg(test)]
        let key = Span {
            start: next.1,
            end: self.state.offset,
        };
        if self.state.nonwhite()?.0 != b':' {
            return Err(FrameError::Punctuation.into());
        }
        let first = self.state.nonwhite()?;
        let (value, value_receipt) = self.state.framed_value(first)?;
        let member = Member {
            #[cfg(test)]
            key,
            kind: Key::Unknown,
            value,
            receipt: value_receipt,
        };
        Ok(Some((text, member)))
    }
}
pub(in crate::loading) struct Element {
    #[cfg(test)]
    index: usize,
    value: Span,
    receipt: SpanDigest,
}
impl Element {
    #[cfg(test)]
    pub(in crate::loading) fn index(&self) -> usize {
        self.index
    }
    pub(in crate::loading) fn value(&self) -> Span {
        self.value
    }
    #[cfg(test)]
    pub(in crate::loading) fn receipt(&self) -> &SpanDigest {
        &self.receipt
    }
    pub(in crate::loading) fn native_span(&self) -> super::held_file::plan::NativeSpan {
        super::held_file::plan::NativeSpan::from_scan(
            self.value.start,
            self.value.len(),
            self.receipt.covered_end,
            self.receipt.digest,
        )
    }
}
/// Positional struct input, without fabricated named keys or a retained row Vec.
pub(in crate::loading) struct ArrayFrames<R> {
    state: FrameState<R>,
    first: bool,
    finished: bool,
    #[cfg(test)]
    index: usize,
}
impl<R: Read> ArrayFrames<R> {
    pub(in crate::loading) fn next_element(&mut self) -> Result<Option<Element>, FrameError> {
        if self.finished {
            return Ok(None);
        }
        let mut next = self.state.nonwhite()?;
        if next.0 == b']' {
            self.finished = true;
            return Ok(None);
        }
        if !self.first {
            if next.0 != b',' {
                return Err(FrameError::Punctuation);
            }
            next = self.state.nonwhite()?;
        }
        self.first = false;
        let (value, receipt) = self.state.framed_value(next)?;
        #[cfg(test)]
        let index = self.index;
        #[cfg(test)]
        {
            self.index = index.checked_add(1).ok_or(FrameError::Arithmetic)?;
        }
        Ok(Some(Element {
            #[cfg(test)]
            index,
            value,
            receipt,
        }))
    }
}
/// Current-byte key cursor reads through the same offset/digest observation.
struct FrameInput<'a, R>(&'a mut FrameState<R>);
impl<R: Read> Read for FrameInput<'_, R> {
    fn read(&mut self, output: &mut [u8]) -> io::Result<usize> {
        if output.is_empty() {
            return Ok(0);
        }
        let next = self.0.byte().map_err(|error| {
            if self.0.managed_marker {
                io::Error::from(io::ErrorKind::InvalidData)
            } else {
                io::Error::other(error)
            }
        })?;
        match next {
            Some((byte, _)) => {
                output[0] = byte;
                Ok(1)
            }
            None => Ok(0),
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;
    #[test]
    fn fields_preserve_duplicate_order_and_exact_nested_value_spans() {
        let input = br#" {"rawInput":{"x":"}\\\"["},"rawInput":null,"tail":[1, {"z":2}]} "#;
        assert!(serde_json::from_slice::<serde_json::Value>(input).is_ok());
        let mut frames = ObjectFrames::new(Cursor::new(input), 0).unwrap();
        let first = frames.next_member().unwrap().unwrap();
        let second = frames.next_member().unwrap().unwrap();
        let third = frames.next_member().unwrap().unwrap();
        for field in [&first, &second, &third] {
            let key = field.key();
            assert!(
                serde_json::from_slice::<String>(&input[key.start()..key.start() + key.len()])
                    .is_ok()
            );
            let value = field.value();
            assert!(
                serde_json::from_slice::<serde_json::Value>(
                    &input[value.start()..value.start() + value.len()]
                )
                .is_ok()
            );
        }
        assert_eq!(&input[second.value.start..second.value.end], b"null");
        assert!(frames.next_member().unwrap().is_none());
    }
    #[test]
    fn framing_is_never_mistaken_for_stock_semantic_acceptance() {
        let input = br#"{"rawInput":{"$serde_json::private::RawValue":"not json"}}"#;
        let mut frames = ObjectFrames::new(Cursor::new(input), 0).unwrap();
        let field = frames.next_member().unwrap().unwrap();
        let span = field.value();
        assert!(
            serde_json::from_slice::<serde_json::Value>(
                &input[span.start()..span.start() + span.len()]
            )
            .is_err()
        );
        assert!(frames.next_member().unwrap().is_none());
    }
    #[test]
    fn raw_expansion_resets_stock_recursion_instead_of_imposing_global_depth() {
        let inner = format!("{{\"payload\":{}0{}}}", "[".repeat(100), "]".repeat(100));
        let marker = format!(
            "{{\"$serde_json::private::RawValue\":{}}}",
            serde_json::to_string(&inner).unwrap()
        );
        let outer = format!(
            "{{\"payload\":{}{}{} }}",
            "[".repeat(100),
            marker,
            "]".repeat(100)
        );
        // Combined semantic nesting exceeds128, but the two stock JSON parser
        // invocations each have an independent original recursion limit.
        assert!(serde_json::from_str::<serde_json::Value>(&outer).is_ok());
        for document in [&outer, &inner] {
            let mut frame = ObjectFrames::new(Cursor::new(document.as_bytes()), 0).unwrap();
            assert!(frame.next_member().unwrap().is_some());
            assert!(frame.next_member().unwrap().is_none());
        }
    }
    #[test]
    fn per_field_digest_covers_actual_scan_bytes_including_scalar_lookahead() {
        let input = br#"{"s":"text","n":42 ,"o":{"x":true},"a":[1,2]}"#;
        let mut frames = ObjectFrames::new(Cursor::new(input), 0).unwrap();
        while let Some(field) = frames.next_member().unwrap() {
            let span = field.value();
            let receipt = field.receipt();
            let actual: [u8; 32] =
                Sha256::digest(&input[span.start()..receipt.covered_end()]).into();
            assert_eq!(actual, receipt.digest());
            assert!(receipt.covered_end() >= span.start() + span.len());
            assert!(receipt.covered_end() <= span.start() + span.len() + 1);
        }
    }
}
