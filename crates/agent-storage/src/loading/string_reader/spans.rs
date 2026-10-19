//! Exact logical byte positions for stock-skipped string values; no unescaping.
use serde::de::{DeserializeSeed, IgnoredAny};
use std::io::{self, Read};
use std::{cell::RefCell, rc::Rc};
#[derive(Debug, thiserror::Error)]
#[error("JSON logical byte offset overflow")]
struct OffsetOverflow;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::loading) struct EncodedSpan {
    start: usize,
    end: usize,
}
impl EncodedSpan {
    pub(in crate::loading) fn start(self) -> usize {
        self.start
    }
    pub(in crate::loading) fn len(self) -> usize {
        self.end - self.start
    }
}
#[derive(Default)]
struct Capture {
    start: Option<usize>,
    end: Option<usize>,
    escaped: bool,
    invalid: bool,
}
#[derive(Default)]
struct Position {
    read: usize,
    capture: Option<Capture>,
}
#[derive(Clone, Default)]
pub(in crate::loading) struct SpanProbe(Rc<RefCell<Position>>);
impl SpanProbe {
    pub(in crate::loading) fn wrap<R: Read>(&self, source: R) -> ProbedReader<R> {
        ProbedReader {
            source,
            probe: self.clone(),
        }
    }
    pub(in crate::loading) fn seed(&self) -> SpanSeed<'_> {
        SpanSeed(self)
    }
    pub(in crate::loading) fn reset_pass(&self) {
        *self.0.borrow_mut() = Position::default();
    }
    fn observe(&self, bytes: &[u8]) -> io::Result<()> {
        let mut state = self.0.borrow_mut();
        for &byte in bytes {
            let offset = state.read;
            state.read = offset
                .checked_add(1)
                .ok_or_else(|| io::Error::other(OffsetOverflow))?;
            let Some(capture) = &mut state.capture else {
                continue;
            };
            if capture.invalid || capture.end.is_some() {
                continue;
            }
            if capture.start.is_none() {
                match byte {
                    b' ' | b'\n' | b'\r' | b'\t' => {}
                    b'"' => capture.start = Some(offset),
                    _ => capture.invalid = true,
                }
            } else if capture.escaped {
                capture.escaped = false;
            } else {
                match byte {
                    b'\\' => capture.escaped = true,
                    b'"' => capture.end = Some(offset + 1),
                    _ => {}
                }
            }
        }
        Ok(())
    }
}
pub(in crate::loading) struct ProbedReader<R> {
    source: R,
    probe: SpanProbe,
}
impl<R: Read> Read for ProbedReader<R> {
    fn read(&mut self, output: &mut [u8]) -> io::Result<usize> {
        let count = self.source.read(output)?;
        self.probe.observe(&output[..count])?;
        Ok(count)
    }
}
pub(in crate::loading) struct SpanSeed<'a>(&'a SpanProbe);
impl<'de> DeserializeSeed<'de> for SpanSeed<'_> {
    type Value = EncodedSpan;
    fn deserialize<D: serde::Deserializer<'de>>(self, decoder: D) -> Result<EncodedSpan, D::Error> {
        self.0.0.borrow_mut().capture = Some(Capture::default());
        let result = IgnoredAny::deserialize(decoder);
        let capture = self.0.0.borrow_mut().capture.take();
        result?;
        let Some(Capture {
            start: Some(start),
            end: Some(end),
            invalid: false,
            ..
        }) = capture
        else {
            return Err(serde::de::Error::custom(
                "expected an encoded JSON string span",
            ));
        };
        Ok(EncodedSpan { start, end })
    }
}
use serde::Deserialize;
#[cfg(test)]
mod tests {
    use super::super::JsonStringReader;
    use super::*;
    use std::io::{Cursor, Seek as _};
    #[test]
    fn stock_ignored_string_span_replays_exact_literal_after_whitespace() {
        let encoded = b" \n\t\"a\\\"b\\\\c\" \r\n";
        let probe = SpanProbe::default();
        let mut decoder = serde_json::Deserializer::from_reader(probe.wrap(Cursor::new(encoded)));
        let span = probe.seed().deserialize(&mut decoder).unwrap();
        decoder.end().unwrap();
        drop(decoder);
        assert_eq!(span.start(), 3);
        let mut replay = Cursor::new(encoded);
        replay
            .seek(io::SeekFrom::Start(span.start() as u64))
            .unwrap();
        let mut cursor = JsonStringReader::new(replay, span.len()).unwrap();
        let mut actual = String::new();
        cursor.read_to_string(&mut actual).unwrap();
        assert_eq!(actual, serde_json::from_slice::<String>(encoded).unwrap());
    }
    #[test]
    fn ignored_any_lone_surrogate_acceptance_never_becomes_value_acceptance() {
        let encoded = br#""\uD800""#;
        let probe = SpanProbe::default();
        let mut decoder = serde_json::Deserializer::from_reader(probe.wrap(Cursor::new(encoded)));
        let span = probe.seed().deserialize(&mut decoder).unwrap();
        decoder.end().unwrap();
        drop(decoder);
        assert!(serde_json::from_slice::<String>(encoded).is_err());
        let mut cursor = JsonStringReader::new(Cursor::new(encoded), span.len()).unwrap();
        assert!(cursor.read_to_end(&mut Vec::new()).is_err());
    }
}
