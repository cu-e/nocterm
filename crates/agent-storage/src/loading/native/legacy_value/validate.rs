//! Every occurrence is semantically read before retained winners are constructed.
//! Ordinary strings and keys have no owned text or fact entries in this pass.
use super::{ValueError, frames::Frames, pump::BytePump, source::SemanticSpan};
use crate::loading::{
    literal_reader::{LiteralReadError, input::Input, scalar},
    string_reader::JsonStringReader,
};
use nocterm_ai::model::json::deferred::DecodeLedger;
use std::io::Read;
const MAGIC: &[u8] = b"$serde_json::private::RawValue";
pub(super) struct ValidatedValue {
    root: SemanticSpan,
}
impl ValidatedValue {
    pub(super) fn into_span(self) -> SemanticSpan {
        self.root
    }
}
enum Action {
    Value { depth: usize, map: bool },
    ArrayAfter(usize),
    ObjectAfter(usize),
    RawEnd,
}
struct Validator<'a, 'r> {
    ledger: &'a DecodeLedger,
    input: Input<BytePump<'a, &'r mut dyn Read>>,
    actions: Frames<'a, Action>,
    limit: Option<usize>,
}
impl<'a, 'r> Validator<'a, 'r> {
    fn new(
        reader: &'r mut dyn Read,
        ledger: &'a DecodeLedger,
        root: &SemanticSpan,
        limit: Option<usize>,
    ) -> Result<Self, ValueError> {
        let mut result = Self {
            ledger,
            input: Input::new_managed_marker(BytePump::new(reader, ledger)?),
            actions: Frames::new(ledger)?,
            limit,
        };
        let context = root.context();
        result.actions.push(Action::Value {
            depth: context.depth(),
            map: context.map(),
        })?;
        Ok(result)
    }
    fn run(mut self) -> Result<(), ValueError> {
        let result = self.drive();
        match self.input.source_mut().take_failure() {
            Some(error) => Err(ValueError::Literal(error.into())),
            None => result,
        }
    }
    fn drive(&mut self) -> Result<(), ValueError> {
        while let Some(action) = self.actions.pop() {
            self.step(action)?;
        }
        self.finish_source()
    }
    fn finish_source(&mut self) -> Result<(), ValueError> {
        if self.input.nonwhite()?.is_some() {
            return Err(ValueError::Punctuation);
        }
        Ok(self.input.check()?)
    }
    fn step(&mut self, action: Action) -> Result<(), ValueError> {
        match action {
            Action::Value { depth, map } => self.value(depth, map),
            Action::ArrayAfter(depth) => self.array_after(depth),
            Action::ObjectAfter(depth) => self.object_after(depth),
            Action::RawEnd => {
                self.finish_source()?;
                let parent_position = self.input.source_mut().leave()?;
                self.input.reset_position(parent_position)?;
                if self.required()? != b'}' {
                    return Err(ValueError::RawShape);
                }
                Ok(())
            }
        }
    }
    fn required(&mut self) -> Result<u8, ValueError> {
        self.input.nonwhite()?.ok_or(ValueError::Punctuation)
    }
    fn unread(&mut self, byte: u8) -> Result<(), ValueError> {
        Ok(self.input.unread(byte)?)
    }
    fn value(&mut self, depth: usize, map: bool) -> Result<(), ValueError> {
        let byte = self.required()?;
        if map && byte != b'{' {
            return Err(ValueError::RawShape);
        }
        match byte {
            b'[' => self.array(depth),
            b'{' => self.object(depth, !map),
            b'"' => {
                self.unread(byte)?;
                self.string()?;
                Ok(())
            }
            b'n' | b't' | b'f' | b'-' | b'0'..=b'9' => {
                self.unread(byte)?;
                scalar::read(&mut self.input, self.ledger)?;
                Ok(())
            }
            _ => Err(ValueError::Punctuation),
        }
    }
    fn array(&mut self, depth: usize) -> Result<(), ValueError> {
        if self.limit.is_some_and(|limit| depth >= limit) {
            return Err(ValueError::Depth);
        }
        let byte = self.required()?;
        if byte == b']' {
            return Ok(());
        }
        self.unread(byte)?;
        self.actions.push(Action::ArrayAfter(depth))?;
        self.actions.push(Action::Value {
            depth: depth + 1,
            map: false,
        })
    }
    fn array_after(&mut self, depth: usize) -> Result<(), ValueError> {
        match self.required()? {
            b']' => Ok(()),
            b',' => {
                self.actions.push(Action::ArrayAfter(depth))?;
                self.actions.push(Action::Value {
                    depth: depth + 1,
                    map: false,
                })
            }
            _ => Err(ValueError::Punctuation),
        }
    }
    fn object(&mut self, depth: usize, raw: bool) -> Result<(), ValueError> {
        if self.limit.is_some_and(|limit| depth >= limit) {
            return Err(ValueError::Depth);
        }
        let byte = self.required()?;
        if byte == b'}' {
            return Ok(());
        }
        if byte != b'"' {
            return Err(ValueError::Punctuation);
        }
        self.unread(byte)?;
        let magic = self.string()?;
        if self.required()? != b':' {
            return Err(ValueError::Punctuation);
        }
        if raw && magic {
            return self.raw();
        }
        self.actions.push(Action::ObjectAfter(depth))?;
        self.actions.push(Action::Value {
            depth: depth + 1,
            map: false,
        })
    }
    fn object_after(&mut self, depth: usize) -> Result<(), ValueError> {
        match self.required()? {
            b'}' => Ok(()),
            b',' => {
                if self.required()? != b'"' {
                    return Err(ValueError::Punctuation);
                }
                self.unread(b'"')?;
                self.string()?;
                if self.required()? != b':' {
                    return Err(ValueError::Punctuation);
                }
                self.actions.push(Action::ObjectAfter(depth))?;
                self.actions.push(Action::Value {
                    depth: depth + 1,
                    map: false,
                })
            }
            _ => Err(ValueError::Punctuation),
        }
    }
    fn string(&mut self) -> Result<bool, ValueError> {
        let mut cursor = JsonStringReader::new_managed(
            &mut self.input,
            nocterm_ai::history::MAX_FILE_BYTES as usize,
            self.ledger,
        )
        .map_err(LiteralReadError::from)?;
        let mut index = 0usize;
        let mut same = true;
        while let Some(fragment) = cursor.next_fragment().map_err(LiteralReadError::from)? {
            for byte in fragment.bytes() {
                same &= MAGIC.get(index) == Some(&byte);
                index = index
                    .checked_add(1)
                    .ok_or(nocterm_ai::model::json::deferred::JsonDecodeError::Arithmetic)?;
            }
        }
        Ok(same && index == MAGIC.len())
    }
    fn raw(&mut self) -> Result<(), ValueError> {
        if self.required()? != b'"' {
            return Err(ValueError::RawShape);
        }
        self.input.source_mut().enter()?;
        self.input.reset_position(0)?;
        self.actions.push(Action::RawEnd)?;
        self.actions.push(Action::Value {
            depth: 0,
            map: false,
        })
    }
}
pub(super) fn read(
    reader: &mut dyn Read,
    ledger: &DecodeLedger,
    root: SemanticSpan,
    limit: Option<usize>,
) -> Result<ValidatedValue, ValueError> {
    Validator::new(reader, ledger, &root, limit)?.run()?;
    Ok(ValidatedValue { root })
}
