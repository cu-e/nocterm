//! Immutable encoded image data: clones share their allocation and wire bytes stay a string.
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::{ops::Deref, sync::Arc};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ImageData(Arc<String>);
impl ImageData {
    /// The normalized encoded allocation and its owned String/Arc framing.
    pub fn retained_bytes(&self) -> usize {
        self.0
            .capacity()
            .saturating_add(std::mem::size_of::<String>())
            .saturating_add(2 * std::mem::size_of::<usize>())
    }
}
impl Deref for ImageData {
    type Target = str;
    fn deref(&self) -> &str {
        &self.0
    }
}
impl AsRef<[u8]> for ImageData {
    fn as_ref(&self) -> &[u8] {
        self.as_bytes()
    }
}
impl From<String> for ImageData {
    fn from(value: String) -> Self {
        Self(Arc::new(value.into_boxed_str().into_string()))
    }
}
impl From<ImageData> for String {
    fn from(value: ImageData) -> Self {
        Arc::try_unwrap(value.0).unwrap_or_else(|value| (*value).clone())
    }
}
impl PartialEq<String> for ImageData {
    fn eq(&self, other: &String) -> bool {
        self.deref() == other
    }
}
impl Serialize for ImageData {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self)
    }
}
impl<'de> Deserialize<'de> for ImageData {
    fn deserialize<D: Deserializer<'de>>(decoder: D) -> Result<Self, D::Error> {
        String::deserialize(decoder).map(Self::from)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn excess_owned_capacity_is_released_before_the_image_can_be_retained() {
        let mut input = String::with_capacity(4 * 1024 * 1024);
        input.push_str("eA==");
        let data = ImageData::from(input);
        assert!(data.retained_bytes() < 1024);
        assert!(data.retained_bytes() > data.len());
        assert_eq!(serde_json::to_string(&data).unwrap(), "\"eA==\"");
        let owned = String::from(data);
        assert_eq!(owned.capacity(), owned.len());
        assert_eq!(owned, "eA==");
    }
    #[test]
    fn cloning_shares_immutable_bytes_and_preserves_the_string_wire_format() {
        let original = ImageData::from("encoded image".to_owned());
        let copy = original.clone();
        assert_eq!(original.as_ptr(), copy.as_ptr());
        assert_eq!(serde_json::to_string(&copy).unwrap(), "\"encoded image\"");
        assert_eq!(
            serde_json::from_str::<ImageData>("\"encoded image\"").unwrap(),
            original
        );
        assert_eq!(String::from(copy), "encoded image");
    }
}
