//! Queue serialization keeps complete media and original prompt identities.
use super::super::{AttachmentSlot, ComposerSlot, PromptSlot};
use super::{Read, content::Content};
use serde::{
    Serialize, Serializer,
    ser::{SerializeSeq, SerializeStruct},
};
pub(super) struct Composer<'a>(pub &'a ComposerSlot);
impl Serialize for Composer<'_> {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        fields!(s,"ComposerData";"draft"=self.0.draft().map(Read),"queue"=Prompts(self.0.queue()),"attachments"=Attachments(self.0.attachments()))
    }
}
struct Prompts<'a>(&'a [PromptSlot]);
impl Serialize for Prompts<'_> {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        let mut seq = s.serialize_seq(Some(self.0.len()))?;
        for prompt in self.0 {
            seq.serialize_element(&Prompt(prompt))?;
        }
        seq.end()
    }
}
pub(super) struct Prompt<'a>(pub &'a PromptSlot);
impl Serialize for Prompt<'_> {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        fields!(s,"SavedPrompt";"id"=self.0.id(),"text"=Read(self.0.text()),"images"=Images(self.0),"attachments"=Attachments(self.0.attachments()))
    }
}
pub(super) struct Framing<'a>(pub &'a ComposerSlot);
impl Serialize for Framing<'_> {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        fields!(s,"ComposerData";"draft"=self.0.draft().map(Read),"queue"=&[] as &[()],"attachments"=Attachments(self.0.attachments()))
    }
}
struct Images<'a>(&'a PromptSlot);
impl Serialize for Images<'_> {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        let images = self.0.images();
        let mut seq = s.serialize_seq(Some(images.len()))?;
        for value in images {
            seq.serialize_element(&Content {
                value,
                omit_images: false,
            })?;
        }
        seq.end()
    }
}
struct Attachments<'a>(&'a [AttachmentSlot]);
impl Serialize for Attachments<'_> {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        let mut seq = s.serialize_seq(Some(self.0.len()))?;
        for attachment in self.0 {
            seq.serialize_element(&Attachment(attachment))?;
        }
        seq.end()
    }
}
struct Attachment<'a>(&'a AttachmentSlot);
impl Serialize for Attachment<'_> {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        let kind = match self.0 {
            AttachmentSlot::Connection(_) => "connection",
            AttachmentSlot::Group(_) => "group",
            AttachmentSlot::LocalTerminal(_) => "local_terminal",
        };
        // SavedAttachment uses `id` as its content key.
        fields!(s,"SavedAttachment";"kind"=kind,"id"=Read(self.0.id()))
    }
}
