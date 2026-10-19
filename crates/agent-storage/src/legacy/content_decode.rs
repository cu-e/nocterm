//! Private tagged content grammar bypasses only the embedded resource retry clone.
use super::content;
use super::resource::Resource;
use agent_client_protocol_schema::v1 as wire;
use nocterm_ai::model::{
    content::{Content, EmbeddedResource},
    diagnostic::ModelError,
};
use serde::{Deserialize, Serialize};
use serde_with::{DefaultOnError, serde_as};

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub(super) enum Block {
    Text(wire::TextContent),
    Image(wire::ImageContent),
    Audio(wire::AudioContent),
    ResourceLink(wire::ResourceLink),
    Resource(Embedded),
}
#[serde_as]
#[derive(Debug, Serialize, Deserialize)]
pub(super) struct Embedded {
    #[serde_as(deserialize_as = "DefaultOnError")]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    annotations: Option<wire::Annotations>,
    resource: Resource,
    #[serde_as(deserialize_as = "DefaultOnError")]
    #[serde(default, rename = "_meta", skip_serializing_if = "Option::is_none")]
    meta: Option<wire::Meta>,
}
impl Block {
    pub(super) fn into_owned(
        self,
        authority: &nocterm_ai::model::retention::RetentionAuthority,
    ) -> Result<Content, ModelError> {
        match self {
            Self::Resource(value) => Ok(Content::EmbeddedResource(EmbeddedResource {
                body: value.resource.into_owned(authority)?,
                annotations: content::annotations(authority, value.annotations)?,
                extensions: content::extensions(authority, value.meta)?,
            })),
            Self::Text(value) => content::moving(authority, wire::ContentBlock::Text(value)),
            Self::Image(value) => content::moving(authority, wire::ContentBlock::Image(value)),
            Self::Audio(value) => content::moving(authority, wire::ContentBlock::Audio(value)),
            Self::ResourceLink(value) => {
                content::moving(authority, wire::ContentBlock::ResourceLink(value))
            }
        }
    }
}
