//! Controlled canonical containers. Formula follows pinned indexmap2.14.2 /
//! hashbrown0.17.1: dense Bucket{hash,key,value}; buckets plus u8 controls+Group16.
use crate::model::diagnostic::ModelError;
use serde_json::{Map, Value};
use std::mem::size_of;
fn add(left: usize, right: usize) -> Result<usize, ModelError> {
    left.checked_add(right).ok_or(ModelError::JsonArithmetic)
}
fn mul(left: usize, right: usize) -> Result<usize, ModelError> {
    left.checked_mul(right).ok_or(ModelError::JsonArithmetic)
}
pub(super) fn object_bytes(entries: usize) -> Result<usize, ModelError> {
    if entries == 0 {
        return Ok(0);
    }
    let buckets = match entries {
        1..=3 => 4,
        4..=7 => 8,
        8..=14 => 16,
        _ => mul(entries, 8)?
            .checked_div(7)
            .and_then(usize::checked_next_power_of_two)
            .ok_or(ModelError::JsonArithmetic)?,
    };
    let dense = mul(
        entries,
        size_of::<usize>() + size_of::<String>() + size_of::<Value>(),
    )?;
    add(dense, add(mul(buckets, size_of::<usize>() + 1)?, 16)?)
}
fn string(value: String) -> Result<(String, usize), ModelError> {
    // Exact existing strings move. Sparse caller reserve is never adopted blindly.
    if value.capacity() == value.len() {
        let capacity = value.capacity();
        return Ok((value, capacity));
    }
    let mut normalized = String::new();
    normalized.try_reserve_exact(value.len()).map_err(|error| {
        ModelError::JsonAllocation(nocterm_core::SharedError::from_error(error))
    })?;
    normalized.push_str(&value);
    let capacity = normalized.capacity();
    Ok((normalized, capacity))
}
pub(super) fn normalize(value: Value) -> Result<(Value, usize), ModelError> {
    match value {
        Value::String(value) => {
            let (value, retained) = string(value)?;
            Ok((Value::String(value), retained))
        }
        Value::Array(values) => array(values),
        Value::Object(values) => object(values),
        other => Ok((other, 0)),
    }
}
fn array(values: Vec<Value>) -> Result<(Value, usize), ModelError> {
    let mut normalized = Vec::new();
    normalized
        .try_reserve_exact(values.len())
        .map_err(|error| {
            ModelError::JsonAllocation(nocterm_core::SharedError::from_error(error))
        })?;
    let mut retained = mul(normalized.capacity(), size_of::<Value>())?;
    for value in values {
        let (value, bytes) = normalize(value)?;
        retained = add(retained, bytes)?;
        normalized.push(value);
    }
    Ok((Value::Array(normalized), retained))
}
fn object(values: Map<String, Value>) -> Result<(Value, usize), ModelError> {
    let requested = values.len();
    let mut retained = object_bytes(requested)?;
    let mut normalized = Map::with_capacity(requested);
    for (key, value) in values {
        let (key, key_bytes) = string(key)?;
        let (value, value_bytes) = normalize(value)?;
        retained = add(retained, add(key_bytes, value_bytes)?)?;
        normalized.insert(key, value);
    }
    Ok((Value::Object(normalized), retained))
}
