//! Images and files of a prompt, uploaded to the chat in parts before the message is sent.

use base64::Engine as _;
use nocterm_ai::model::content::Content;
use nocterm_hub_client::HubClient;
use nocterm_hub_proto::v2;
use sha2::{Digest as _, Sha256};

use super::connector::CallFailure;

/// The largest part sent when the hub names none.
const PART_BYTES: usize = 1024 * 1024;

/// The text of a prompt and the attachments its other parts became.
pub(super) async fn prepare(
    client: &HubClient,
    chat: &str,
    content: Vec<Content>,
) -> Result<(String, Vec<String>), CallFailure> {
    let mut text = String::new();
    let mut attachments = Vec::new();
    for (index, part) in content.into_iter().enumerate() {
        match part {
            Content::Text(value) => {
                if !text.is_empty() {
                    text.push('\n');
                }
                text.push_str(&value.text);
            }
            Content::Image(image) => {
                let bytes = base64::engine::general_purpose::STANDARD
                    .decode(image.data.as_bytes())
                    .map_err(|_| CallFailure::Refused("The image could not be read.".into()))?;
                let extension = image.mime_type.rsplit('/').next().unwrap_or("png");
                let name = format!("image-{}.{extension}", index + 1);
                attachments.push(upload(client, chat, &name, &image.mime_type, &bytes).await?);
            }
            // Audio and resources are not something a hub agent reads.
            _ => {}
        }
    }
    Ok((text, attachments))
}

async fn upload(
    client: &HubClient,
    chat: &str,
    name: &str,
    mime: &str,
    bytes: &[u8],
) -> Result<String, CallFailure> {
    let sha256 = hex::encode(Sha256::digest(bytes));
    let begun = client
        .chats()
        .begin_upload(v2::BeginUploadRequest {
            chat_id: chat.to_owned(),
            name: name.to_owned(),
            mime: mime.to_owned(),
            size: bytes.len() as u64,
            sha256,
        })
        .await?
        .into_inner();
    let part = match usize::try_from(begun.part_size) {
        Ok(0) | Err(_) => PART_BYTES,
        Ok(part) => part.min(PART_BYTES),
    };
    let mut offset = usize::try_from(begun.received)
        .unwrap_or(0)
        .min(bytes.len());
    while offset < bytes.len() {
        let end = (offset + part).min(bytes.len());
        client
            .chats()
            .upload_part(v2::UploadPartRequest {
                upload_id: begun.upload_id.clone(),
                offset: offset as u64,
                data: bytes[offset..end].to_vec(),
            })
            .await?;
        offset = end;
    }
    let finished = client
        .chats()
        .finish_upload(v2::IdRequest {
            id: begun.upload_id,
        })
        .await?
        .into_inner();
    Ok(finished.id)
}
