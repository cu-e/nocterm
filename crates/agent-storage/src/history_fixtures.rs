//! Shared plain history fixtures, constructed through public owned document parts.
use nocterm_ai::{
    history::{ChatMetadata, ComposerData, ResumeDescriptor, SavedChat, TranscriptData},
    model::{
        content::{Content, Media, MediaKind, Text},
        ids::ToolId,
        tools::{ToolKind, ToolRecord, ToolStatus},
    },
    thread::Entry,
};
pub(crate) fn edited(
    chat: SavedChat,
    change: impl FnOnce(
        &mut ChatMetadata,
        &mut ResumeDescriptor,
        &mut TranscriptData,
        &mut ComposerData,
    ),
) -> SavedChat {
    let authority = chat.retention().clone();
    let (id, mut metadata, mut resume, mut transcript, mut composer, _) = chat.into_parts();
    change(&mut metadata, &mut resume, &mut transcript, &mut composer);
    SavedChat::from_parts(&authority, id, metadata, resume, transcript, composer).unwrap()
}
pub(crate) fn text(value: impl Into<String>) -> Content {
    Content::Text(Text {
        text: value.into(),
        annotations: None,
        extensions: None,
    })
}
pub(crate) fn image(data: impl Into<String>, mime: impl Into<String>) -> Content {
    Content::Media(Media {
        kind: MediaKind::Image,
        encoded: data.into(),
        mime: mime.into(),
        uri: None,
        annotations: None,
        extensions: None,
    })
}
pub(crate) fn tool(
    authority: &nocterm_ai::model::retention::RetentionAuthority,
    arguments: serde_json::Value,
) -> Entry {
    Entry::Tool(ToolRecord {
        id: ToolId::new("tool"),
        title: "amplification".into(),
        name: None,
        kind: ToolKind::Other,
        status: ToolStatus::Pending,
        output: Vec::new(),
        locations: Vec::new(),
        arguments: Some(
            nocterm_ai::model::json::OwnedJson::from_untracked(authority, arguments).unwrap(),
        ),
        result: None,
        extensions: None,
        claims: Default::default(),
        literal: Default::default(),
        client_display: None,
    })
}

/// Compatibility oracle uses the real supplied-context seed and rejects trailing input.
pub(crate) fn decoded<T: nocterm_ai::model::decode::Decode>(
    authority: &nocterm_ai::model::retention::RetentionAuthority,
    bytes: &[u8],
) -> Result<T, serde_json::Error> {
    use serde::de::DeserializeSeed as _;
    let mut decoder = serde_json::Deserializer::from_slice(bytes);
    let value = nocterm_ai::model::decode::Seed::<T>::new(authority).deserialize(&mut decoder)?;
    decoder.end()?;
    Ok(value)
}
pub(crate) fn decoded_value<T: nocterm_ai::model::decode::Decode>(
    authority: &nocterm_ai::model::retention::RetentionAuthority,
    value: serde_json::Value,
) -> Result<T, serde_json::Error> {
    use serde::de::{DeserializeSeed as _, IntoDeserializer as _};
    nocterm_ai::model::decode::Seed::<T>::new(authority).deserialize(value.into_deserializer())
}
