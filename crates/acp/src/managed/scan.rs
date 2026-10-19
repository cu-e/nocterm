//! Iterative JSON validation and source ranges without a materialized Value tree.
use super::error::DecodeError;
use agent_client_protocol::managed::{Budget, Buffer};
use std::ops::Range;

#[derive(Clone, Copy)]
enum Container {
    ArrayValue(bool),
    ArrayComma,
    ObjectKey(bool),
    ObjectColon,
    ObjectValue,
    ObjectComma,
}
pub(super) struct Scanner<'a, B: Budget> {
    input: &'a str,
    stack: Buffer<Container, B>,
    protocol_frame: bool,
}
impl<'a, B: Budget> Scanner<'a, B> {
    pub(super) fn new(input: &'a str, budget: B) -> Self {
        Self {
            input,
            stack: Buffer::new(budget),
            protocol_frame: false,
        }
    }
    pub(super) fn whitespace(&self, mut position: usize) -> usize {
        while self
            .input
            .as_bytes()
            .get(position)
            .is_some_and(|byte| matches!(byte, b' ' | b'\n' | b'\r' | b'\t'))
        {
            position += 1;
        }
        position
    }
    pub(super) fn complete(&mut self) -> Result<Range<usize>, DecodeError<B::Error>> {
        let start = self.whitespace(0);
        // The pinned SDK's outer serde_json::Value decoder starts with128
        // recursion credits and refuses its128th container. Actual differential
        // tests establish this protocol rule; no history decoder policy applies.
        self.protocol_frame = true;
        let end = self.value(start);
        self.protocol_frame = false;
        let end = end?;
        if self.whitespace(end) != self.input.len() {
            return Err(DecodeError::Syntax(end));
        }
        Ok(start..end)
    }
    pub(super) fn value(&mut self, start: usize) -> Result<usize, DecodeError<B::Error>> {
        self.stack.clear();
        let mut position = self.whitespace(start);
        self.atom(&mut position)?;
        while let Some(container) = self.stack.as_slice().last().copied() {
            position = self.whitespace(position);
            match container {
                Container::ArrayValue(first) => {
                    if first && self.byte(position) == Some(b']') {
                        position += 1;
                        self.stack.pop();
                        continue;
                    }
                    self.replace_top(Container::ArrayComma);
                    self.atom(&mut position)?;
                }
                Container::ArrayComma => match self.byte(position) {
                    Some(b',') => {
                        position += 1;
                        self.replace_top(Container::ArrayValue(false));
                    }
                    Some(b']') => {
                        position += 1;
                        self.stack.pop();
                    }
                    _ => return Err(DecodeError::Syntax(position)),
                },
                Container::ObjectKey(first) => {
                    if first && self.byte(position) == Some(b'}') {
                        position += 1;
                        self.stack.pop();
                        continue;
                    }
                    position = self.string(position)?;
                    self.replace_top(Container::ObjectColon);
                }
                Container::ObjectColon => {
                    if self.byte(position) != Some(b':') {
                        return Err(DecodeError::Syntax(position));
                    }
                    position += 1;
                    self.replace_top(Container::ObjectValue);
                }
                Container::ObjectValue => {
                    self.replace_top(Container::ObjectComma);
                    self.atom(&mut position)?;
                }
                Container::ObjectComma => match self.byte(position) {
                    Some(b',') => {
                        position += 1;
                        self.replace_top(Container::ObjectKey(false));
                    }
                    Some(b'}') => {
                        position += 1;
                        self.stack.pop();
                    }
                    _ => return Err(DecodeError::Syntax(position)),
                },
            }
        }
        Ok(position)
    }
    fn replace_top(&mut self, value: Container) {
        if let Some(top) = self.stack.as_mut_slice().last_mut() {
            *top = value;
        }
    }
    fn push(&mut self, value: Container) -> Result<(), DecodeError<B::Error>> {
        if self.protocol_frame && self.stack.len() + 1 >= 128 {
            return Err(DecodeError::Syntax(0));
        }
        self.stack
            .push(value)
            .map_err(|(error, _)| DecodeError::Allocation(error))
    }
    pub(super) fn input_byte(&self, position: usize) -> Option<u8> {
        self.byte(position)
    }
    fn byte(&self, position: usize) -> Option<u8> {
        self.input.as_bytes().get(position).copied()
    }
    fn atom(&mut self, position: &mut usize) -> Result<(), DecodeError<B::Error>> {
        *position = self.whitespace(*position);
        match self.byte(*position) {
            Some(b'{') => {
                *position += 1;
                self.push(Container::ObjectKey(true))?;
            }
            Some(b'[') => {
                *position += 1;
                self.push(Container::ArrayValue(true))?;
            }
            Some(b'"') => *position = self.string(*position)?,
            Some(b't') => *position = self.literal(*position, "true")?,
            Some(b'f') => *position = self.literal(*position, "false")?,
            Some(b'n') => *position = self.literal(*position, "null")?,
            Some(b'-' | b'0'..=b'9') => *position = self.number(*position)?,
            _ => return Err(DecodeError::Syntax(*position)),
        }
        Ok(())
    }
    fn literal(&self, start: usize, value: &str) -> Result<usize, DecodeError<B::Error>> {
        if self.input[start..].starts_with(value) {
            Ok(start + value.len())
        } else {
            Err(DecodeError::Syntax(start))
        }
    }
    fn number(&self, start: usize) -> Result<usize, DecodeError<B::Error>> {
        let mut position = start;
        if self.byte(position) == Some(b'-') {
            position += 1;
        }
        match self.byte(position) {
            Some(b'0') => position += 1,
            Some(b'1'..=b'9') => {
                position += 1;
                while self
                    .byte(position)
                    .is_some_and(|byte| byte.is_ascii_digit())
                {
                    position += 1;
                }
            }
            _ => return Err(DecodeError::Syntax(position)),
        }
        if self.byte(position) == Some(b'.') {
            position += 1;
            let digits = position;
            while self
                .byte(position)
                .is_some_and(|byte| byte.is_ascii_digit())
            {
                position += 1;
            }
            if digits == position {
                return Err(DecodeError::Syntax(position));
            }
        }
        if matches!(self.byte(position), Some(b'e' | b'E')) {
            position += 1;
            if matches!(self.byte(position), Some(b'+' | b'-')) {
                position += 1;
            }
            let digits = position;
            while self
                .byte(position)
                .is_some_and(|byte| byte.is_ascii_digit())
            {
                position += 1;
            }
            if digits == position {
                return Err(DecodeError::Syntax(position));
            }
        }
        if !self.input[start..position]
            .parse::<f64>()
            .is_ok_and(f64::is_finite)
        {
            return Err(DecodeError::Syntax(start));
        }
        Ok(position)
    }
    pub(super) fn string(&self, start: usize) -> Result<usize, DecodeError<B::Error>> {
        if self.byte(start) != Some(b'"') {
            return Err(DecodeError::Syntax(start));
        }
        let mut position = start + 1;
        loop {
            match self.byte(position) {
                Some(b'"') => return Ok(position + 1),
                Some(b'\\') => {
                    let (_, next) = self.escape(position)?;
                    position = next;
                }
                Some(0..=31) | None => return Err(DecodeError::Syntax(position)),
                Some(_) => position += 1,
            }
        }
    }
    pub(super) fn decoded_char(
        &self,
        position: usize,
    ) -> Result<(char, usize), DecodeError<B::Error>> {
        if self.byte(position) == Some(b'\\') {
            return self.escape(position);
        }
        let scalar = self
            .input
            .get(position..)
            .and_then(|text| text.chars().next())
            .ok_or(DecodeError::Syntax(position))?;
        Ok((scalar, position + scalar.len_utf8()))
    }
    fn escape(&self, start: usize) -> Result<(char, usize), DecodeError<B::Error>> {
        let escaped = self.byte(start + 1).ok_or(DecodeError::Syntax(start))?;
        let scalar = match escaped {
            b'"' => '"',
            b'\\' => '\\',
            b'/' => '/',
            b'b' => '\u{8}',
            b'f' => '\u{c}',
            b'n' => '\n',
            b'r' => '\r',
            b't' => '\t',
            b'u' => return self.unicode(start + 2),
            _ => return Err(DecodeError::Syntax(start)),
        };
        Ok((scalar, start + 2))
    }
    fn hex4(&self, start: usize) -> Result<u32, DecodeError<B::Error>> {
        let end = start.checked_add(4).ok_or(DecodeError::Syntax(start))?;
        let text = self
            .input
            .get(start..end)
            .ok_or(DecodeError::Syntax(start))?;
        let mut value = 0u32;
        for byte in text.bytes() {
            let digit = match byte {
                b'0'..=b'9' => byte - b'0',
                b'a'..=b'f' => byte - b'a' + 10,
                b'A'..=b'F' => byte - b'A' + 10,
                _ => return Err(DecodeError::Syntax(start)),
            };
            value = (value << 4) | u32::from(digit);
        }
        Ok(value)
    }
    fn unicode(&self, start: usize) -> Result<(char, usize), DecodeError<B::Error>> {
        let first = self.hex4(start)?;
        let (scalar, end) = if (0xd800..=0xdbff).contains(&first) {
            let next = start + 4;
            if self.input.get(next..next + 2) != Some("\\u") {
                return Err(DecodeError::Syntax(next));
            }
            let second = self.hex4(next + 2)?;
            if !(0xdc00..=0xdfff).contains(&second) {
                return Err(DecodeError::Syntax(next + 2));
            }
            (
                0x10000 + ((first - 0xd800) << 10) + (second - 0xdc00),
                next + 6,
            )
        } else {
            (first, start + 4)
        };
        let scalar = char::from_u32(scalar).ok_or(DecodeError::Syntax(start))?;
        Ok((scalar, end))
    }
    /// Compare a validated JSON string with a static schema key without decoding
    /// every unknown key into a separately allocated String.
    pub(super) fn key_equals(
        &self,
        range: Range<usize>,
        expected: &str,
    ) -> Result<bool, DecodeError<B::Error>> {
        if self.byte(range.start) != Some(b'"')
            || range.end <= range.start + 1
            || self.byte(range.end - 1) != Some(b'"')
        {
            return Ok(false);
        }
        let mut expected = expected.chars();
        let mut position = range.start + 1;
        let end = range
            .end
            .checked_sub(1)
            .ok_or(DecodeError::Syntax(range.start))?;
        let mut equal = true;
        while position < end {
            let (scalar, next) = if self.byte(position) == Some(b'\\') {
                self.escape(position)?
            } else {
                let scalar = self.input[position..]
                    .chars()
                    .next()
                    .ok_or(DecodeError::Syntax(position))?;
                (scalar, position + scalar.len_utf8())
            };
            if expected.next() != Some(scalar) {
                equal = false;
            }
            position = next;
        }
        Ok(equal && expected.next().is_none())
    }
    pub(super) fn decoded_string(
        &self,
        range: Range<usize>,
        mut append: impl FnMut(&str) -> Result<(), DecodeError<B::Error>>,
    ) -> Result<(), DecodeError<B::Error>> {
        let mut position = range.start + 1;
        let end = range
            .end
            .checked_sub(1)
            .ok_or(DecodeError::Syntax(range.start))?;
        while position < end {
            if self.byte(position) == Some(b'\\') {
                let (scalar, next) = self.escape(position)?;
                let mut buffer = [0u8; 4];
                append(scalar.encode_utf8(&mut buffer))?;
                position = next;
            } else {
                let start = position;
                while position < end && self.byte(position) != Some(b'\\') {
                    position += 1;
                }
                append(&self.input[start..position])?;
            }
        }
        Ok(())
    }
}
