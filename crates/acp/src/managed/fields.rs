//! Winning native field spans without allocating overwritten/unknown values.
use super::{error::DecodeError, scan::Scanner};
use agent_client_protocol::managed::Budget;
use std::ops::Range;

pub(super) struct Fields<'a, B: Budget> {
    scan: Scanner<'a, B>,
    position: usize,
    end: usize,
    object: bool,
    first: bool,
    ordinal: usize,
}
pub(super) struct Field {
    key: Option<Range<usize>>,
    value: Range<usize>,
    ordinal: usize,
}
impl<'a, B: Budget> Fields<'a, B> {
    pub(super) fn new(
        input: &'a str,
        range: Range<usize>,
        budget: B,
    ) -> Result<Self, DecodeError<B::Error>> {
        let object = match input.as_bytes().get(range.start) {
            Some(b'{') => true,
            Some(b'[') => false,
            _ => return Err(DecodeError::Field("object or positional array")),
        };
        Ok(Self {
            scan: Scanner::new(input, budget),
            position: range.start + 1,
            end: range.end,
            object,
            first: true,
            ordinal: 0,
        })
    }
    fn next(&mut self) -> Result<Option<Field>, DecodeError<B::Error>> {
        self.position = self.scan.whitespace(self.position);
        if self.position >= self.end {
            return Err(DecodeError::Syntax(self.position));
        }
        let closing = if self.object { b'}' } else { b']' };
        if self.scan.input_byte(self.position) == Some(closing) {
            return Ok(None);
        }
        if !self.first {
            if self.scan.input_byte(self.position) != Some(b',') {
                return Err(DecodeError::Syntax(self.position));
            }
            self.position = self.scan.whitespace(self.position + 1);
        }
        self.first = false;
        let key = if self.object {
            let start = self.position;
            self.position = self.scan.string(start)?;
            let key = start..self.position;
            self.position = self.scan.whitespace(self.position);
            if self.scan.input_byte(self.position) != Some(b':') {
                return Err(DecodeError::Syntax(self.position));
            }
            self.position = self.scan.whitespace(self.position + 1);
            Some(key)
        } else {
            None
        };
        let start = self.position;
        self.position = self.scan.value(start)?;
        let ordinal = self.ordinal;
        self.ordinal += 1;
        Ok(Some(Field {
            key,
            value: start..self.position,
            ordinal,
        }))
    }
    /// Object lookup follows the outer SDK Value's last-winning semantics;
    /// positional inputs follow the actual supplied schema field order.
    pub(super) fn select<const N: usize>(
        &mut self,
        keys: [&str; N],
    ) -> Result<[Option<Range<usize>>; N], DecodeError<B::Error>> {
        let mut found = std::array::from_fn(|_| None);
        while let Some(field) = self.next()? {
            if let Some(key) = field.key {
                for (index, expected) in keys.iter().enumerate() {
                    if self.scan.key_equals(key.clone(), expected)? {
                        found[index] = Some(field.value.clone());
                        break;
                    }
                }
            } else {
                if field.ordinal >= N {
                    return Err(DecodeError::Field("too many positional fields"));
                }
                found[field.ordinal] = Some(field.value);
            }
        }
        Ok(found)
    }
}
