//! The device key seed as hexadecimal text.

/// The seed is not 64 hexadecimal digits.
#[derive(Debug, thiserror::Error)]
#[error("the key seed is not 64 hexadecimal digits")]
pub(crate) struct SeedError;

pub(crate) fn encode(seed: &[u8; 32]) -> String {
    hex::encode(seed)
}

pub(crate) fn decode(text: &str) -> Result<[u8; 32], SeedError> {
    let mut seed = [0_u8; 32];
    hex::decode_to_slice(text, &mut seed).map_err(|_| SeedError)?;
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

    #[test]
    fn mixed_case_seeds_keep_the_same_bytes_and_encode_canonically() {
        let seed = decode(&"aF".repeat(32)).unwrap();
        assert_eq!(seed, [175; 32]);
        assert_eq!(encode(&seed), "af".repeat(32));
    }
}
