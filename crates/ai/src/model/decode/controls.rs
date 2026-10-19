use super::{Decode, Tagged, field, missing, seeded_struct, unknown, value};
use crate::model::{controls::*, extensions::Extensions, ids::*, retention::RetentionAuthority};
use serde::Deserialize;
seeded_struct!(ConfigOption {id:ConfigId => plain required,name:String => plain required,description:Option<String> => plain optional,category:Option<ConfigCategory> => plain optional,kind:ConfigKind => owned required,extensions:Option<Extensions> => owned optional});
seeded_struct!(ConfigChoice {id:ConfigChoiceId => plain required,name:String => plain required,description:Option<String> => plain optional,extensions:Option<Extensions> => owned optional});
seeded_struct!(ConfigGroup {id:ConfigGroupId => plain required,name:String => plain required,choices:Vec<ConfigChoice> => owned required,extensions:Option<Extensions> => owned optional});
seeded_struct!(Modes {current:ModeId => plain required,choices:Vec<ModeChoice> => owned required,extensions:Option<Extensions> => owned optional});
seeded_struct!(ModeChoice {id:ModeId => plain required,name:String => plain required,description:Option<String> => plain optional,extensions:Option<Extensions> => owned optional});
impl Decode for SelectChoices {
    fn decode<'de, D: serde::Deserializer<'de>>(
        a: &RetentionAuthority,
        d: D,
    ) -> Result<Self, D::Error> {
        let t = Tagged::deserialize(d)?;
        Ok(match t.kind.as_str() {
            "flat" => Self::Flat(value(a, t.value)?),
            "groups" => Self::Groups(value(a, t.value)?),
            _ => return Err(serde::de::Error::custom("unknown choice grouping")),
        })
    }
}
impl Decode for ConfigKind {
    fn decode<'de, D: serde::Deserializer<'de>>(
        a: &RetentionAuthority,
        d: D,
    ) -> Result<Self, D::Error> {
        let t = Tagged::deserialize(d)?;
        Ok(match t.kind.as_str() {
            "boolean" => Self::Boolean(super::plain(t.value)?),
            "unsupported" => {
                #[derive(Deserialize)]
                struct Unsupported {
                    label: String,
                }
                Self::Unsupported {
                    label: super::plain::<Unsupported, D::Error>(t.value)?.label,
                }
            }
            "select" => {
                struct Selection {
                    current: ConfigChoiceId,
                    choices: SelectChoices,
                }
                seeded_struct!(Selection{current:ConfigChoiceId => plain required,choices:SelectChoices => owned required});
                let v: Selection = value(a, t.value)?;
                Self::Select {
                    current: v.current,
                    choices: v.choices,
                }
            }
            _ => return Err(serde::de::Error::custom("unknown configuration kind")),
        })
    }
}
