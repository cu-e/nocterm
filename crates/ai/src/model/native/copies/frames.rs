//! A paid traversal frontier copies inert JSON iteratively without a depth cap.
use crate::model::{
    extensions::Extensions,
    json::deferred::{
        DecodeLedger, JsonDecodeError,
        ledger::Reservation,
        literal::{LiteralBuilder, LiteralText},
    },
    native::NativeOwned,
};
use serde_json::Value;
enum Frame<'a> {
    Object(serde_json::map::Iter<'a>),
    Array(std::slice::Iter<'a, Value>),
}
pub(super) struct Frames<'a, 'l> {
    values: Vec<Frame<'a>>,
    allocation: Reservation,
    ledger: &'l DecodeLedger,
}
fn text(builder: &LiteralBuilder, value: &str) -> Result<LiteralText, JsonDecodeError> {
    let mut text = builder.exact_text(value.len())?;
    text.append(value)?;
    text.finish()
}
impl<'a, 'l> Frames<'a, 'l> {
    pub(super) fn copy(
        ledger: &'l DecodeLedger,
        value: Option<&'a Extensions>,
    ) -> Result<NativeOwned<Option<Extensions>>, JsonDecodeError> {
        let Some(value) = value else {
            return NativeOwned::absent(ledger);
        };
        let mut frames = Self {
            values: Vec::new(),
            allocation: ledger.reserve(0)?,
            ledger,
        };
        let mut builder = LiteralBuilder::new(ledger)?;
        builder.start_object()?;
        for (key, value) in value.iter() {
            builder.key(text(&builder, key)?)?;
            frames.emit(value, &mut builder)?;
            while let Some(frame) = frames.values.last_mut() {
                let next = match frame {
                    Frame::Object(entries) => match entries.next() {
                        Some((key, value)) => {
                            builder.key(text(&builder, key)?)?;
                            Some(value)
                        }
                        None => None,
                    },
                    Frame::Array(values) => values.next(),
                };
                if let Some(value) = next {
                    frames.emit(value, &mut builder)?;
                } else {
                    builder.end_container()?;
                    frames.values.pop();
                }
            }
        }
        builder.end_container()?;
        Ok(NativeOwned::supplied(
            crate::model::native::content::extensions(builder.finish_native()?, ledger)?,
        ))
    }
    pub(super) fn json(
        ledger: &'l DecodeLedger,
        value: &'a crate::model::json::OwnedJson,
    ) -> Result<NativeOwned<crate::model::json::OwnedJson>, JsonDecodeError> {
        let mut frames = Self {
            values: Vec::new(),
            allocation: ledger.reserve(0)?,
            ledger,
        };
        let mut builder = LiteralBuilder::new(ledger)?;
        frames.emit(value.as_value(), &mut builder)?;
        while let Some(frame) = frames.values.last_mut() {
            let next = match frame {
                Frame::Object(entries) => match entries.next() {
                    Some((key, value)) => {
                        builder.key(text(&builder, key)?)?;
                        Some(value)
                    }
                    None => None,
                },
                Frame::Array(values) => values.next(),
            };
            if let Some(value) = next {
                frames.emit(value, &mut builder)?;
            } else {
                builder.end_container()?;
                frames.values.pop();
            }
        }
        crate::model::native::content::json(builder.finish_native()?, ledger)
    }
    fn emit(
        &mut self,
        value: &'a Value,
        builder: &mut LiteralBuilder,
    ) -> Result<(), JsonDecodeError> {
        let frame = match value {
            Value::Null => {
                builder.null()?;
                None
            }
            Value::Bool(value) => {
                builder.boolean(*value)?;
                None
            }
            Value::Number(value) => {
                builder.number(value.clone())?;
                None
            }
            Value::String(value) => {
                builder.string(text(builder, value)?)?;
                None
            }
            Value::Array(value) => {
                builder.start_array()?;
                Some(Frame::Array(value.iter()))
            }
            Value::Object(value) => {
                builder.start_object()?;
                Some(Frame::Object(value.iter()))
            }
        };
        if let Some(frame) = frame {
            self.push(frame)?;
        }
        Ok(())
    }
    fn push(&mut self, frame: Frame<'a>) -> Result<(), JsonDecodeError> {
        if self.values.len() == self.values.capacity() {
            let capacity = self
                .values
                .capacity()
                .max(1)
                .checked_mul(2)
                .ok_or(JsonDecodeError::Arithmetic)?;
            let allocation = self.ledger.reserve(
                capacity
                    .checked_mul(std::mem::size_of::<Frame<'a>>())
                    .ok_or(JsonDecodeError::Arithmetic)?,
            )?;
            let mut values = Vec::new();
            values.try_reserve_exact(capacity)?;
            if values.capacity() != capacity {
                return Err(JsonDecodeError::CapacityProfile);
            }
            values.append(&mut self.values);
            let previous = std::mem::replace(&mut self.values, values);
            drop(previous);
            self.allocation = allocation;
        }
        self.values.push(frame);
        Ok(())
    }
}
