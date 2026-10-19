//! Display formatting streams borrowed fragments through the same string metric.
use super::{MeasurementError, string_len};
use std::fmt::{self, Display, Write};
pub(super) struct Size {
    pub(super) encoded: usize,
    pub(super) decoded_utf8: usize,
}
struct Counter {
    size: Size,
    overflow: bool,
}
impl Write for Counter {
    fn write_str(&mut self, value: &str) -> fmt::Result {
        let measured = string_len(value)
            .and_then(|encoded| {
                self.size
                    .encoded
                    .checked_add(encoded - 2)
                    .ok_or(MeasurementError::ArithmeticOverflow)
            })
            .and_then(|encoded| {
                self.size
                    .decoded_utf8
                    .checked_add(value.len())
                    .map(|decoded| (encoded, decoded))
                    .ok_or(MeasurementError::ArithmeticOverflow)
            });
        match measured {
            Ok((encoded, decoded)) => {
                self.size.encoded = encoded;
                self.size.decoded_utf8 = decoded;
                Ok(())
            }
            Err(_) => {
                self.overflow = true;
                Err(fmt::Error)
            }
        }
    }
}
pub(super) fn measure(value: &(impl Display + ?Sized)) -> Result<Size, MeasurementError> {
    let mut counter = Counter {
        size: Size {
            encoded: 2,
            decoded_utf8: 0,
        },
        overflow: false,
    };
    let result = write!(counter, "{value}");
    if counter.overflow {
        return Err(MeasurementError::ArithmeticOverflow);
    }
    result.map_err(MeasurementError::Formatting)?;
    Ok(counter.size)
}
