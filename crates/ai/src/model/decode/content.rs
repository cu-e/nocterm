use super::{Decode, Tagged, field, missing, seeded_struct, unknown, value};
use crate::model::{content::*, extensions::Extensions, retention::RetentionAuthority};
use serde::Deserialize;
seeded_struct!(Text {text:String => plain required,annotations:Option<Annotations> => owned optional,extensions:Option<Extensions> => owned optional});
seeded_struct!(Media {kind:MediaKind => plain required,encoded:String => plain required,mime:String => plain required,uri:Option<String> => plain optional,annotations:Option<Annotations> => owned optional,extensions:Option<Extensions> => owned optional});
seeded_struct!(ResourceReference {uri:String => plain required,name:String => plain required,title:Option<String> => plain optional,description:Option<String> => plain optional,mime:Option<String> => plain optional,reported_size:Option<i64> => plain optional,annotations:Option<Annotations> => owned optional,extensions:Option<Extensions> => owned optional});
seeded_struct!(EmbeddedResource {body:ResourceBody => owned required,annotations:Option<Annotations> => owned optional,extensions:Option<Extensions> => owned optional});
seeded_struct!(ResourceText {uri:String => plain required,text:String => plain required,mime:Option<String> => plain optional,extensions:Option<Extensions> => owned optional});
seeded_struct!(ResourceBlob {uri:String => plain required,encoded:String => plain required,mime:Option<String> => plain optional,extensions:Option<Extensions> => owned optional});
seeded_struct!(Annotations {audience:Option<Vec<Audience>> => plain optional,modified:Option<String> => plain optional,priority:Option<f64> => plain optional,extensions:Option<Extensions> => owned optional});
impl Decode for Content {
    fn decode<'de, D: serde::Deserializer<'de>>(
        a: &RetentionAuthority,
        d: D,
    ) -> Result<Self, D::Error> {
        let t = Tagged::deserialize(d)?;
        Ok(match t.kind.as_str() {
            "text" => Self::Text(value(a, t.value)?),
            "media" => Self::Media(value(a, t.value)?),
            "resource_reference" => Self::ResourceReference(value(a, t.value)?),
            "embedded_resource" => Self::EmbeddedResource(value(a, t.value)?),
            _ => return Err(serde::de::Error::custom("unknown content kind")),
        })
    }
}
impl Decode for ResourceBody {
    fn decode<'de, D: serde::Deserializer<'de>>(
        a: &RetentionAuthority,
        d: D,
    ) -> Result<Self, D::Error> {
        let t = Tagged::deserialize(d)?;
        Ok(match t.kind.as_str() {
            "text" => Self::Text(value(a, t.value)?),
            "encoded" => Self::Encoded(value(a, t.value)?),
            _ => return Err(serde::de::Error::custom("unknown resource body")),
        })
    }
}
