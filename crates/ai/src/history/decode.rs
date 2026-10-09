//! Reject collection amplification before growing a saved document's vectors.
use serde::{
    Deserialize,
    de::{Error as _, SeqAccess, Visitor},
};
use std::marker::PhantomData;

pub(super) fn entries<'de, D: serde::Deserializer<'de>>(
    d: D,
) -> Result<Vec<crate::thread::Entry>, D::Error> {
    bounded::<_, _, 2000>(d)
}
pub(super) fn prompts<'de, D: serde::Deserializer<'de>>(
    d: D,
) -> Result<Vec<super::SavedPrompt>, D::Error> {
    bounded::<_, _, 1024>(d)
}
pub(super) fn attachments<'de, D: serde::Deserializer<'de>>(
    d: D,
) -> Result<Vec<super::SavedAttachment>, D::Error> {
    bounded::<_, _, 1024>(d)
}
pub(super) fn images<'de, D: serde::Deserializer<'de>>(
    d: D,
) -> Result<Vec<crate::acp::ContentBlock>, D::Error> {
    bounded::<_, _, 128>(d)
}
pub(super) fn times<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Vec<u64>, D::Error> {
    bounded::<_, _, 2000>(d)
}
fn bounded<'de, D, T, const LIMIT: usize>(d: D) -> Result<Vec<T>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de>,
{
    struct Collection<T, const LIMIT: usize>(PhantomData<T>);
    impl<'de, T: Deserialize<'de>, const LIMIT: usize> Visitor<'de> for Collection<T, LIMIT> {
        type Value = Vec<T>;
        fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            write!(f, "at most {LIMIT} elements")
        }
        fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Vec<T>, A::Error> {
            let mut values = Vec::new();
            while values.len() < LIMIT {
                let Some(value) = seq.next_element()? else {
                    return Ok(values);
                };
                values.push(value);
            }
            if seq.next_element::<serde::de::IgnoredAny>()?.is_some() {
                return Err(A::Error::custom(
                    "saved collection exceeds its element limit",
                ));
            }
            Ok(values)
        }
    }
    d.deserialize_seq(Collection::<T, LIMIT>(PhantomData))
}
