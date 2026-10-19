//! Bounded lexical unit framing delegates Unicode escape acceptance to pinned serde.
use crate::loading::string_reader::StringReadError;
use crate::loading::string_reader::unit;
use nocterm_ai::model::json::deferred::DecodeLedger;

enum State {
    Opening,
    Plain,
    Escape,
    Hex(u8),
    PairSlash,
    PairU,
    LowHex(u8),
    Utf8(u8),
    End,
}
pub(super) struct Decoder {
    state: State,
    lexical: [u8; 14],
    output: [u8; 4],
    start: u8,
    end: u8,
}
impl Decoder {
    pub(super) fn quoted() -> Self {
        Self::new(State::Opening)
    }
    pub(super) fn entered() -> Self {
        Self::new(State::Plain)
    }
    fn new(state: State) -> Self {
        Self {
            state,
            lexical: [0; 14],
            output: [0; 4],
            start: 0,
            end: 0,
        }
    }
    pub(super) fn finished(&self) -> bool {
        matches!(self.state, State::End)
    }
    pub(super) fn ready(&mut self) -> Option<Option<u8>> {
        if self.start != self.end {
            let value = self.output[self.start as usize];
            self.start += 1;
            Some(Some(value))
        } else if self.finished() {
            Some(None)
        } else {
            None
        }
    }
    fn oracle(&mut self, closing: usize, ledger: &DecodeLedger) -> Result<(), StringReadError> {
        self.lexical[closing] = b'"';
        let unit = unit::parse(&self.lexical[..closing + 1], Some(ledger))?;
        (self.output, self.end) = unit.into_parts();
        self.start = 0;
        self.state = State::Plain;
        Ok(())
    }
    fn emit(&mut self, byte: u8) {
        self.output[0] = byte;
        self.start = 0;
        self.end = 1;
    }
    pub(super) fn accept(
        &mut self,
        byte: u8,
        ledger: &DecodeLedger,
    ) -> Result<(), StringReadError> {
        match self.state {
            State::Opening => {
                if byte != b'"' {
                    return Err(StringReadError::OpeningQuote);
                }
                self.state = State::Plain;
            }
            State::Plain => self.plain(byte, ledger)?,
            State::Escape => {
                self.lexical[2] = byte;
                if byte == b'u' {
                    self.state = State::Hex(0);
                } else {
                    self.oracle(3, ledger)?;
                }
            }
            State::Hex(index) => {
                self.lexical[3 + index as usize] = byte;
                if index == 3 {
                    // Select the lexical unit width without constructing a
                    // failing oracle Error on a successful surrogate pair.
                    // Stock serde still decides every acceptance/codepoint.
                    let code = std::str::from_utf8(&self.lexical[3..7])
                        .ok()
                        .and_then(|hex| u16::from_str_radix(hex, 16).ok());
                    if code.is_some_and(|code| (0xD800..=0xDBFF).contains(&code)) {
                        self.state = State::PairSlash;
                    } else {
                        self.oracle(7, ledger)?;
                    }
                } else {
                    self.state = State::Hex(index + 1);
                }
            }
            State::PairSlash => {
                if byte != b'\\' {
                    return self.oracle(7, ledger);
                }
                self.lexical[7] = byte;
                self.state = State::PairU;
            }
            State::PairU => {
                if byte != b'u' {
                    self.lexical[7] = b'"';
                    return self.oracle(7, ledger);
                }
                self.lexical[8] = byte;
                self.state = State::LowHex(0);
            }
            State::LowHex(index) => {
                self.lexical[9 + index as usize] = byte;
                if index == 3 {
                    self.oracle(13, ledger)?;
                } else {
                    self.state = State::LowHex(index + 1);
                }
            }
            State::Utf8(length) => self.utf8(byte, length)?,
            State::End => return Err(StringReadError::Truncated),
        }
        Ok(())
    }
    fn plain(&mut self, byte: u8, ledger: &DecodeLedger) -> Result<(), StringReadError> {
        match byte {
            b'"' => self.state = State::End,
            b'\\' => {
                self.lexical[0] = b'"';
                self.lexical[1] = byte;
                self.state = State::Escape;
            }
            0..=31 => {
                self.lexical[0] = b'"';
                self.lexical[1] = byte;
                self.oracle(2, ledger)?;
            }
            32..=127 => self.emit(byte),
            _ => {
                self.output[0] = byte;
                self.utf8_state(1)?;
            }
        }
        Ok(())
    }
    fn utf8(&mut self, byte: u8, length: u8) -> Result<(), StringReadError> {
        self.output[length as usize] = byte;
        self.utf8_state(length + 1)
    }
    fn utf8_state(&mut self, length: u8) -> Result<(), StringReadError> {
        match std::str::from_utf8(&self.output[..length as usize]) {
            Ok(_) => {
                self.start = 0;
                self.end = length;
                self.state = State::Plain;
                Ok(())
            }
            Err(error) if error.error_len().is_none() && length < 4 => {
                self.state = State::Utf8(length);
                Ok(())
            }
            Err(error) => Err(StringReadError::Utf8(error)),
        }
    }
}
