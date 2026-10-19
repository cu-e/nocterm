//! Explicit-context compatibility decoding. Controlled native loaders provide
//! their own allocation plans; these serde seeds do not establish parser peak bounds.
use super::retention::RetentionAuthority;
use serde::{
    Deserialize,
    de::{DeserializeSeed, SeqAccess, Visitor},
};
use std::{fmt, marker::PhantomData};
mod auth;
mod content;
mod controls;
mod tools;

pub trait Decode: Sized {
    fn decode<'de, D: serde::Deserializer<'de>>(
        authority: &RetentionAuthority,
        decoder: D,
    ) -> Result<Self, D::Error>;
}
pub struct Seed<'a, T>(pub &'a RetentionAuthority, pub PhantomData<T>);
impl<'a, T> Seed<'a, T> {
    pub fn new(authority: &'a RetentionAuthority) -> Self {
        Self(authority, PhantomData)
    }
}
impl<'de, T: Decode> DeserializeSeed<'de> for Seed<'_, T> {
    type Value = T;
    fn deserialize<D: serde::Deserializer<'de>>(self, decoder: D) -> Result<T, D::Error> {
        T::decode(self.0, decoder)
    }
}
impl<T: Decode> Decode for Option<T> {
    fn decode<'de, D: serde::Deserializer<'de>>(
        authority: &RetentionAuthority,
        decoder: D,
    ) -> Result<Self, D::Error> {
        struct Optional<'a, T>(&'a RetentionAuthority, PhantomData<T>);
        impl<'de, T: Decode> Visitor<'de> for Optional<'_, T> {
            type Value = Option<T>;
            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("an optional retained value")
            }
            fn visit_none<E: serde::de::Error>(self) -> Result<Self::Value, E> {
                Ok(None)
            }
            fn visit_unit<E: serde::de::Error>(self) -> Result<Self::Value, E> {
                Ok(None)
            }
            fn visit_some<D: serde::Deserializer<'de>>(
                self,
                d: D,
            ) -> Result<Self::Value, D::Error> {
                T::decode(self.0, d).map(Some)
            }
        }
        decoder.deserialize_option(Optional(authority, PhantomData))
    }
}
impl<T: Decode> Decode for Vec<T> {
    fn decode<'de, D: serde::Deserializer<'de>>(
        authority: &RetentionAuthority,
        decoder: D,
    ) -> Result<Self, D::Error> {
        struct Sequence<'a, T>(&'a RetentionAuthority, PhantomData<T>);
        impl<'de, T: Decode> Visitor<'de> for Sequence<'_, T> {
            type Value = Vec<T>;
            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("retained values")
            }
            fn visit_seq<A: SeqAccess<'de>>(self, mut a: A) -> Result<Self::Value, A::Error> {
                let mut out = Vec::new();
                while let Some(value) = a.next_element_seed(Seed::<T>::new(self.0))? {
                    out.push(value);
                }
                Ok(out)
            }
        }
        decoder.deserialize_seq(Sequence(authority, PhantomData))
    }
}
pub trait VecItem {
    type Item: Decode;
}
impl<T: Decode> VecItem for Vec<T> {
    type Item = T;
}
pub(crate) struct BoundedSeed<'a, T, const LIMIT: usize>(
    pub &'a RetentionAuthority,
    pub PhantomData<T>,
);
impl<'de, T: Decode, const LIMIT: usize> DeserializeSeed<'de> for BoundedSeed<'_, T, LIMIT> {
    type Value = Vec<T>;
    fn deserialize<D: serde::Deserializer<'de>>(self, d: D) -> Result<Self::Value, D::Error> {
        struct Sequence<'a, T, const N: usize>(&'a RetentionAuthority, PhantomData<T>);
        impl<'de, T: Decode, const N: usize> Visitor<'de> for Sequence<'_, T, N> {
            type Value = Vec<T>;
            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(f, "at most {N} elements")
            }
            fn visit_seq<A: SeqAccess<'de>>(self, mut a: A) -> Result<Self::Value, A::Error> {
                use serde::de::Error as _;
                let mut out = Vec::new();
                while out.len() < N {
                    let Some(v) = a.next_element_seed(Seed::<T>::new(self.0))? else {
                        return Ok(out);
                    };
                    out.push(v);
                }
                if a.next_element::<serde::de::IgnoredAny>()?.is_some() {
                    return Err(A::Error::custom(
                        "saved collection exceeds its element limit",
                    ));
                }
                Ok(out)
            }
        }
        d.deserialize_seq(Sequence::<T, LIMIT>(self.0, PhantomData))
    }
}
impl Decode for u64 {
    fn decode<'de, D: serde::Deserializer<'de>>(
        _: &RetentionAuthority,
        d: D,
    ) -> Result<Self, D::Error> {
        Self::deserialize(d)
    }
}
impl<T: Decode> Decode for Box<T> {
    fn decode<'de, D: serde::Deserializer<'de>>(
        a: &RetentionAuthority,
        d: D,
    ) -> Result<Self, D::Error> {
        T::decode(a, d).map(Box::new)
    }
}
impl<T: Decode> Decode for std::sync::Arc<T> {
    fn decode<'de, D: serde::Deserializer<'de>>(
        a: &RetentionAuthority,
        d: D,
    ) -> Result<Self, D::Error> {
        T::decode(a, d).map(std::sync::Arc::new)
    }
}
impl Decode for super::json::OwnedJson {
    fn decode<'de, D: serde::Deserializer<'de>>(
        a: &RetentionAuthority,
        d: D,
    ) -> Result<Self, D::Error> {
        super::json::OwnedJsonSeed(a).deserialize(d)
    }
}
impl Decode for super::extensions::Extensions {
    fn decode<'de, D: serde::Deserializer<'de>>(
        a: &RetentionAuthority,
        d: D,
    ) -> Result<Self, D::Error> {
        super::extensions::ExtensionsSeed(a).deserialize(d)
    }
}
#[derive(Deserialize)]
struct Tagged {
    kind: String,
    value: serde_json::Value,
}
fn value<T: Decode, E: serde::de::Error>(
    a: &RetentionAuthority,
    v: serde_json::Value,
) -> Result<T, E> {
    T::decode(a, v).map_err(E::custom)
}
fn plain<T: serde::de::DeserializeOwned, E: serde::de::Error>(
    v: serde_json::Value,
) -> Result<T, E> {
    serde_json::from_value(v).map_err(E::custom)
}

macro_rules! field {
    ($map:ident,$a:ident,context,$ty:ty) => {{
        let _: serde::de::IgnoredAny = $map.next_value()?;
        Ok::<$ty, _>($a.clone())
    }};
    ($map:ident,$a:ident,bound2000,$ty:ty) => {
        $map.next_value_seed($crate::model::decode::BoundedSeed::<
            <$ty as $crate::model::decode::VecItem>::Item,
            2000,
        >($a, std::marker::PhantomData))
    };
    ($map:ident,$a:ident,bound1024,$ty:ty) => {
        $map.next_value_seed($crate::model::decode::BoundedSeed::<
            <$ty as $crate::model::decode::VecItem>::Item,
            1024,
        >($a, std::marker::PhantomData))
    };
    ($map:ident,$a:ident,bound128,$ty:ty) => {
        $map.next_value_seed($crate::model::decode::BoundedSeed::<
            <$ty as $crate::model::decode::VecItem>::Item,
            128,
        >($a, std::marker::PhantomData))
    };
    ($map:ident,$a:ident,plain,$ty:ty) => {
        $map.next_value::<$ty>()
    };
    ($map:ident,$a:ident,owned,$ty:ty) => {
        $map.next_value_seed($crate::model::decode::Seed::<$ty>::new($a))
    };
    ($map:ident,$a:ident,present,$ty:ty) => {
        $map.next_value_seed($crate::model::decode::tools::JsonPresenceSeed($a))
    };
}
macro_rules! sequence_field {
    ($seq:ident,$a:ident,context,$ty:ty) => {
        Ok::<Option<$ty>, _>(Some($a.clone()))
    };
    ($seq:ident,$a:ident,bound2000,$ty:ty) => {
        $seq.next_element_seed($crate::model::decode::BoundedSeed::<
            <$ty as $crate::model::decode::VecItem>::Item,
            2000,
        >($a, std::marker::PhantomData))
    };
    ($seq:ident,$a:ident,bound1024,$ty:ty) => {
        $seq.next_element_seed($crate::model::decode::BoundedSeed::<
            <$ty as $crate::model::decode::VecItem>::Item,
            1024,
        >($a, std::marker::PhantomData))
    };
    ($seq:ident,$a:ident,bound128,$ty:ty) => {
        $seq.next_element_seed($crate::model::decode::BoundedSeed::<
            <$ty as $crate::model::decode::VecItem>::Item,
            128,
        >($a, std::marker::PhantomData))
    };
    ($seq:ident,$a:ident,plain,$ty:ty) => {
        $seq.next_element::<$ty>()
    };
    ($seq:ident,$a:ident,owned,$ty:ty) => {
        $seq.next_element_seed($crate::model::decode::Seed::<$ty>::new($a))
    };
    ($seq:ident,$a:ident,present,$ty:ty) => {
        $seq.next_element_seed($crate::model::decode::tools::JsonPresenceSeed($a))
    };
}
macro_rules! sequence_missing {
    ($v:ident,required,$a:ident,$index:ident,$this:ident) => {
        $v.ok_or_else(|| A::Error::invalid_length($index, &$this))?
    };
    ($v:ident,default,$a:ident,$index:ident,$this:ident) => {
        $v.unwrap_or_default()
    };
    ($v:ident,optional,$a:ident,$index:ident,$this:ident) => {
        $v.ok_or_else(|| A::Error::invalid_length($index, &$this))?
    };
    ($v:ident,context,$a:ident,$index:ident,$this:ident) => {
        $v.unwrap_or_else(|| $a.clone())
    };
}
macro_rules! missing {
    ($v:ident,context,$a:ident) => {
        $v.unwrap_or_else(|| $a.clone())
    };
    ($v:ident,$mode:ident,$a:ident) => {
        missing!($v, $mode)
    };
    ($v:ident,required) => {
        $v.ok_or_else(|| A::Error::missing_field(stringify!($v)))?
    };
    ($v:ident,default) => {
        $v.unwrap_or_default()
    };
    ($v:ident,optional) => {
        $v.unwrap_or_default()
    };
}
macro_rules! unknown {
 ($map:ident,$key:ident,[$($f:ident),*],allow)=>{{$map.next_value::<serde::de::IgnoredAny>()?;}};
 ($map:ident,$key:ident,[$($f:ident),*],deny)=>{return Err(A::Error::unknown_field(&$key,&[$(stringify!($f)),*]))};
}
macro_rules! seeded_struct {
 ($name:ident {$($f:ident:$ty:ty=>$mode:ident $presence:ident),* $(,)?};deny_unknown)=>{seeded_struct!(@build $name{$($f:$ty=>$mode $presence),*}; [];deny);};
 ($name:ident {$($f:ident:$ty:ty=>$mode:ident $presence:ident),* $(,)?};ignored{$($sk:ident),* $(,)?})=>{seeded_struct!(@build $name{$($f:$ty=>$mode $presence),*}; [$($sk),*];allow);};
 ($name:ident {$($f:ident:$ty:ty=>$mode:ident $presence:ident),* $(,)?})=>{seeded_struct!(@build $name{$($f:$ty=>$mode $presence),*}; [];allow);};
 (@build $name:ident { $($f:ident : $ty:ty => $mode:ident $presence:ident),* $(,)? }; [$($sk:ident),*];$unknown:ident) => {
        impl $crate::model::decode::Decode for $name {
            fn decode<'de,D:serde::Deserializer<'de>>(authority:&$crate::model::retention::RetentionAuthority,decoder:D)->Result<Self,D::Error>{
                struct Fields<'a>(&'a $crate::model::retention::RetentionAuthority);
                impl<'de> serde::de::Visitor<'de> for Fields<'_> {
                    type Value=$name;
                    fn expecting(&self,f:&mut std::fmt::Formatter<'_>)->std::fmt::Result{f.write_str(stringify!($name))}
                    fn visit_map<A:serde::de::MapAccess<'de>>(self,mut map:A)->Result<$name,A::Error>{
                        use serde::de::Error as _;
                        let authority=self.0;
                        $(let mut $f:Option<$ty>=None;)*
                        while let Some(key)=map.next_key::<String>()? {
                            match key.as_str() {
                                $(stringify!($f)=>{if $f.is_some(){return Err(A::Error::duplicate_field(stringify!($f)));}$f=Some(field!(map,authority,$mode,$ty)?);})*
                                _=>{unknown!(map,key,[$($f),*],$unknown);}
                            }
                        }
                        Ok($name{$($f:missing!($f,$presence,authority),)* $($sk:Default::default(),)*})
                    }
                    fn visit_seq<A:serde::de::SeqAccess<'de>>(self,mut seq:A)->Result<$name,A::Error>{
                        use serde::de::Error as _;
                        let authority=self.0;
                        let mut index=0usize;
                        $(
                            let $f:Option<$ty>=$crate::model::decode::sequence_field!(seq,authority,$mode,$ty)?;
                            let $f=$crate::model::decode::sequence_missing!($f,$presence,authority,index,self);
                            if stringify!($mode)!="context" { index+=1; }
                        )*
                        if seq.next_element::<serde::de::IgnoredAny>()?.is_some() {
                            return Err(A::Error::invalid_length(index+1,&self));
                        }
                        Ok($name{$($f,)* $($sk:Default::default(),)*})
                    }
                }
                decoder.deserialize_struct(stringify!($name),&[$(stringify!($f)),*],Fields(authority))
            }
        }
    };
}
pub(crate) use {field, missing, seeded_struct, sequence_field, sequence_missing, unknown};

impl Decode for crate::thread::Entry {
    fn decode<'de, D: serde::Deserializer<'de>>(
        a: &RetentionAuthority,
        d: D,
    ) -> Result<Self, D::Error> {
        let t = Tagged::deserialize(d)?;
        Ok(match t.kind.as_str() {
            "user" => Self::User(value(a, t.value)?),
            "agent" => Self::Agent(plain(t.value)?),
            "thought" => Self::Thought(plain(t.value)?),
            "content" => Self::Content(value(a, t.value)?),
            "tool" => Self::Tool(value(a, t.value)?),
            _ => return Err(serde::de::Error::custom("unknown transcript entry")),
        })
    }
}
impl Decode for crate::history::SavedAttachment {
    fn decode<'de, D: serde::Deserializer<'de>>(
        _: &RetentionAuthority,
        d: D,
    ) -> Result<Self, D::Error> {
        Self::deserialize(d)
    }
}
