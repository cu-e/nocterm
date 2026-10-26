//! Encoding and decoding of calls and replies, with the size bounds of the ABI.

use alloc::vec::Vec;

use serde::{Serialize, de::DeserializeOwned};
use thiserror::Error;

use crate::{Call, MAX_INPUT_BYTES, MAX_REPLY_BYTES, Reply};

/// Why bytes are not a call or a reply.
#[derive(Clone, Debug, PartialEq, Eq, Error)]
pub enum AbiError {
    #[error("the value could not be encoded")]
    Encode(#[source] postcard::Error),
    #[error("the bytes are not a valid value of the ABI")]
    Decode(#[source] postcard::Error),
    #[error("the encoding is larger than {max} bytes")]
    TooLarge { max: usize },
    #[error("bytes remain after the value")]
    Trailing,
}

fn encode<T: Serialize>(value: &T, max: usize) -> Result<Vec<u8>, AbiError> {
    let bytes = postcard::to_allocvec(value).map_err(AbiError::Encode)?;
    if bytes.len() > max {
        return Err(AbiError::TooLarge { max });
    }
    Ok(bytes)
}

fn decode<T: DeserializeOwned>(bytes: &[u8], max: usize) -> Result<T, AbiError> {
    if bytes.len() > max {
        return Err(AbiError::TooLarge { max });
    }
    let (value, rest) = postcard::take_from_bytes(bytes).map_err(AbiError::Decode)?;
    if rest.is_empty() {
        Ok(value)
    } else {
        Err(AbiError::Trailing)
    }
}

/// The bytes the host writes into guest memory, at most [`MAX_INPUT_BYTES`].
pub fn encode_call(call: &Call) -> Result<Vec<u8>, AbiError> {
    encode(call, MAX_INPUT_BYTES)
}

/// A call as the guest reads it.
pub fn decode_call(bytes: &[u8]) -> Result<Call, AbiError> {
    decode(bytes, MAX_INPUT_BYTES)
}

/// The bytes a guest returns, at most [`MAX_REPLY_BYTES`].
pub fn encode_reply(reply: &Reply) -> Result<Vec<u8>, AbiError> {
    encode(reply, MAX_REPLY_BYTES)
}

/// A reply as the host reads it.
pub fn decode_reply(bytes: &[u8]) -> Result<Reply, AbiError> {
    decode(bytes, MAX_REPLY_BYTES)
}
