//! The device key seed as hexadecimal text.

use std::fmt::Write as _;

/// The seed is not 64 hexadecimal digits.
#[derive(Debug, thiserror::Error)]
#[error("the key seed is not 64 hexadecimal digits")]
pub(crate) struct SeedError;

pub(crate) fn encode(seed: &[u8; 32]) -> String {
    seed.iter()
        .fold(String::with_capacity(64), |mut text, byte| {
            let _ = write!(text, "{byte:02x}");
            text
        })
}

pub(crate) fn decode(text: &str) -> Result<[u8; 32], SeedError> {
    let (pairs, rest) = text.as_bytes().as_chunks::<2>();
    if pairs.len() != 32 || !rest.is_empty() {
        return Err(SeedError);
    }
    let mut seed = [0_u8; 32];
    for (byte, pair) in seed.iter_mut().zip(pairs) {
        // `from_str_radix` also takes a leading sign, which is no digit.
        if !pair.iter().all(u8::is_ascii_hexdigit) {
            return Err(SeedError);
        }
        let pair = std::str::from_utf8(pair).map_err(|_| SeedError)?;
        *byte = u8::from_str_radix(pair, 16).map_err(|_| SeedError)?;
    }
    Ok(seed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_seed_round_trips_through_hex() {
        let mut seed = [0_u8; 32];
        for (index, byte) in seed.iter_mut().enumerate() {
            *byte = u8::try_from(index * 8).unwrap();
        }
        assert_eq!(decode(&encode(&seed)).unwrap(), seed);
    }

    #[test]
    fn text_that_is_not_a_seed_is_refused() {
        assert!(decode("").is_err());
        assert!(decode(&"a".repeat(63)).is_err());
        assert!(decode(&"g".repeat(64)).is_err());
        assert!(decode(&"é".repeat(32)).is_err());
        assert!(decode(&"+f".repeat(32)).is_err());
    }
}
