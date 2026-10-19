use super::{Decode, Tagged, field, missing, seeded_struct, unknown, value};
use crate::model::{
    content::Content, controls::Presence, extensions::Extensions, ids::*, json::OwnedJson,
    retention::RetentionAuthority, tools::*,
};
use serde::{Deserialize, de::DeserializeSeed};
use std::path::PathBuf;
seeded_struct!(ToolRecord {id:ToolId => owned required,title:String => plain required,name:Option<String> => plain optional,kind:ToolKind => plain required,status:ToolStatus => plain required,output:Vec<ToolOutput> => owned required,locations:Vec<Location> => owned required,arguments:Option<OwnedJson> => present required,result:Option<OwnedJson> => present required,extensions:Option<Extensions> => owned optional,client_display:Option<crate::model::shared_display::SharedDisplay> => owned required}; ignored{claims,literal});
seeded_struct!(ToolPatch {id:ToolId => owned required,changes:ToolChanges => owned required,extensions:Option<Extensions> => owned optional}; ignored{claims,literal});
seeded_struct!(ToolChanges {kind:Option<ToolKind> => plain optional,status:Option<ToolStatus> => plain optional,title:Option<String> => plain optional,name:Option<String> => plain optional,output:Option<Vec<ToolOutput>> => owned optional,locations:Option<Vec<Location>> => owned optional,arguments:Option<OwnedJson> => present required,result:Option<OwnedJson> => present required});
seeded_struct!(FileDiff {path:PathBuf => plain required,previous:Option<String> => plain optional,replacement:String => plain required,extensions:Option<Extensions> => owned optional});
seeded_struct!(Location {path:PathBuf => plain required,line:Option<u32> => plain optional,extensions:Option<Extensions> => owned optional});
seeded_struct!(ClientDisplay {version:u8 => plain required,tool:String => plain required,arguments:OwnedJson => owned required,destination:Presence<String> => plain required,redacted:Presence<bool> => plain required,outcome:Presence<DisplayOutcome> => owned required});
// Keep the original enum grammar: a struct with a defaulted `value` accepts
// missing newtype content and non-unit content on unit branches incorrectly.
#[derive(Deserialize)]
#[serde(tag = "presence", content = "value", rename_all = "snake_case")]
enum JsonPresence {
    Absent,
    Supplied(serde_json::Value),
}
pub(super) struct JsonPresenceSeed<'a>(pub &'a RetentionAuthority);
impl<'de> DeserializeSeed<'de> for JsonPresenceSeed<'_> {
    type Value = Option<OwnedJson>;
    fn deserialize<D: serde::Deserializer<'de>>(self, d: D) -> Result<Self::Value, D::Error> {
        match JsonPresence::deserialize(d)? {
            JsonPresence::Absent => Ok(None),
            JsonPresence::Supplied(payload) => value(self.0, payload).map(Some),
        }
    }
}
impl<T: Decode> Decode for Presence<T> {
    fn decode<'de, D: serde::Deserializer<'de>>(
        a: &RetentionAuthority,
        d: D,
    ) -> Result<Self, D::Error> {
        Ok(match Presence::<serde_json::Value>::deserialize(d)? {
            Presence::Absent => Self::Absent,
            Presence::Null => Self::Null,
            Presence::Value(payload) => Self::Value(value(a, payload)?),
        })
    }
}
impl Decode for DisplayOutcome {
    fn decode<'de, D: serde::Deserializer<'de>>(
        a: &RetentionAuthority,
        d: D,
    ) -> Result<Self, D::Error> {
        // Let serde own external-enum arity, unit and duplicate-key rules.
        #[derive(Deserialize)]
        enum Outcome {
            Pending,
            Ok(serde_json::Value),
            Err(String),
        }
        Ok(match Outcome::deserialize(d)? {
            Outcome::Pending => Self::Pending,
            Outcome::Ok(payload) => Self::Succeeded(value(a, payload)?),
            Outcome::Err(message) => Self::Failed(message),
        })
    }
}

impl Decode for ToolOutput {
    fn decode<'de, D: serde::Deserializer<'de>>(
        a: &RetentionAuthority,
        d: D,
    ) -> Result<Self, D::Error> {
        let t = Tagged::deserialize(d)?;
        Ok(match t.kind.as_str() {
            "content" => {
                struct Body {
                    content: Box<Content>,
                    extensions: Option<Extensions>,
                }
                seeded_struct!(Body{content:Box<Content> => owned required,extensions:Option<Extensions> => owned optional});
                let v: Body = value(a, t.value)?;
                Self::Content {
                    content: v.content,
                    extensions: v.extensions,
                }
            }
            "file_diff" => Self::FileDiff(value(a, t.value)?),
            "terminal_reference" => {
                struct Terminal {
                    id: ProviderTerminalId,
                    extensions: Option<Extensions>,
                }
                seeded_struct!(Terminal{id:ProviderTerminalId => owned required,extensions:Option<Extensions> => owned optional});
                let v: Terminal = value(a, t.value)?;
                Self::TerminalReference {
                    id: v.id,
                    extensions: v.extensions,
                }
            }
            _ => return Err(serde::de::Error::custom("unknown tool output")),
        })
    }
}
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_presence_keeps_original_unit_and_required_content_grammar() {
        let authority = RetentionAuthority::new(usize::MAX).unwrap();
        for (input, accepted) in [
            (r#"{"presence":"absent"}"#, true),
            (r#"{"presence":"absent","value":null}"#, true),
            (r#"["absent",null]"#, true),
            (r#"["absent"]"#, false),
            (r#"{"presence":"absent","value":7}"#, false),
            (r#"{"presence":"supplied"}"#, false),
            (r#"{"presence":"supplied","value":null}"#, true),
        ] {
            let result = JsonPresenceSeed(&authority)
                .deserialize(&mut serde_json::Deserializer::from_str(input));
            assert_eq!(result.is_ok(), accepted, "{input}");
        }
    }

    #[test]
    fn display_outcome_keeps_original_external_enum_grammar() {
        let authority = RetentionAuthority::new(usize::MAX).unwrap();
        for (input, accepted) in [
            (r#""Pending""#, true),
            (r#"{"Pending":null}"#, true),
            (r#"{"Pending":{}}"#, false),
            (r#"{"Ok":null}"#, true),
            (r#"{"Err":"failure"}"#, true),
            (r#"{"Err":null}"#, false),
            (r#"{"Ok":null,"Ok":null}"#, false),
            (r#"{"Ok":null,"Err":"failure"}"#, false),
        ] {
            let result =
                DisplayOutcome::decode(&authority, &mut serde_json::Deserializer::from_str(input));
            assert_eq!(result.is_ok(), accepted, "{input}");
        }
    }

    #[test]
    fn display_presence_keeps_original_adjacent_enum_grammar() {
        let authority = RetentionAuthority::new(usize::MAX).unwrap();
        for (input, accepted) in [
            (r#"{"presence":"absent"}"#, true),
            (r#"["null",null]"#, true),
            (r#"["absent"]"#, false),
            (r#"{"presence":"absent","value":7}"#, false),
            (r#"{"presence":"value"}"#, false),
            (r#"{"presence":"value","value":"Pending"}"#, true),
            (r#"{"presence":"value","value":{"Pending":null}}"#, true),
            (
                r#"{"presence":"value","value":"Pending","presence":"value"}"#,
                false,
            ),
        ] {
            let result = Presence::<DisplayOutcome>::decode(
                &authority,
                &mut serde_json::Deserializer::from_str(input),
            );
            assert_eq!(result.is_ok(), accepted, "{input}");
        }
    }
}
