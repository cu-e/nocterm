//! Install-time resolution: every placeholder against the declared capabilities, their fields
//! and the plugin's settings.

use alloc::string::ToString;

use nocterm_plugin_model::{FieldKind, SettingsSchema};

use super::{Filter, Placeholder, Source, Template, TemplateError};
use crate::capability::{
    Capability, Fields, ValueType, clock::Now, fields_of, notifications::OwnNotice,
};

/// What a template may name.
#[derive(Clone, Copy, Debug)]
pub struct Scope<'a> {
    /// The capability ids the package declares, required or optional.
    pub declared: &'a [&'a str],
    pub settings: &'a SettingsSchema,
    /// Whether `item.` names are allowed here, because an enclosing list has `each`.
    pub item: bool,
}

/// What an expression gives.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Output {
    Text,
    Number,
    Bool,
    Tone,
}

impl Output {
    const fn name(self) -> &'static str {
        match self {
            Self::Text => "text",
            Self::Number => "a number",
            Self::Bool => "a switch value",
            Self::Tone => "a tone",
        }
    }
}

#[derive(Clone, Copy)]
enum Ty {
    Value(ValueType),
    Tone,
}

const fn describe(ty: ValueType) -> &'static str {
    match ty {
        ValueType::Number => "a number",
        ValueType::Text => "text",
        ValueType::Bool => "a switch value",
        ValueType::Time => "a clock instant",
    }
}

impl Template {
    /// Checks that every placeholder resolves and gives something displayable.
    pub fn check(&self, scope: &Scope<'_>) -> Result<(), TemplateError> {
        for hole in self.placeholders() {
            match hole.output(scope)? {
                Output::Tone => {
                    return Err(TemplateError::Output {
                        expected: "text, a number or a switch value",
                        found: "a tone",
                    });
                }
                Output::Text | Output::Number | Output::Bool => {}
            }
        }
        Ok(())
    }
}

impl Placeholder {
    /// What the expression gives after its filters, if every name resolves.
    pub fn output(&self, scope: &Scope<'_>) -> Result<Output, TemplateError> {
        let mut ty = Ty::Value(source_type(&self.source, scope)?);
        for filter in &self.filters {
            ty = apply(*filter, ty)?;
        }
        match ty {
            Ty::Tone => Ok(Output::Tone),
            Ty::Value(ValueType::Number) => Ok(Output::Number),
            Ty::Value(ValueType::Bool) => Ok(Output::Bool),
            Ty::Value(ValueType::Text) => Ok(Output::Text),
            Ty::Value(ValueType::Time) => Err(TemplateError::Output {
                expected: "text, a number or a switch value",
                found: describe(ValueType::Time),
            }),
        }
    }

    /// Checks that the expression gives `wanted`, with no filter when `wanted` is a number or a
    /// switch value.
    pub fn expect(&self, scope: &Scope<'_>, wanted: Output) -> Result<(), TemplateError> {
        let found = self.output(scope)?;
        let plain = !matches!(wanted, Output::Number | Output::Bool) || self.filters.is_empty();
        if found == wanted && plain {
            Ok(())
        } else {
            Err(TemplateError::Output {
                expected: wanted.name(),
                found: found.name(),
            })
        }
    }
}

fn source_type(source: &Source, scope: &Scope<'_>) -> Result<ValueType, TemplateError> {
    match source {
        Source::Capability { capability, field } => {
            if !scope.declared.contains(&capability.id) {
                return Err(TemplateError::Undeclared(capability.id.to_string()));
            }
            let Some(field) = field else {
                return if capability.id == Now::ID {
                    Ok(ValueType::Time)
                } else {
                    Err(TemplateError::NeedsField(capability.id.to_string()))
                };
            };
            lookup(fields_of(capability.id), field).ok_or_else(|| {
                TemplateError::UnknownField(alloc::format!("{}.{field}", capability.id))
            })
        }
        Source::Setting(key) => scope
            .settings
            .field(key)
            .and_then(|field| match field.kind {
                FieldKind::Toggle { .. } => Some(ValueType::Bool),
                FieldKind::Choice { .. } | FieldKind::Text { .. } => Some(ValueType::Text),
                FieldKind::Number { .. } => Some(ValueType::Number),
                FieldKind::MultiChoice { .. } => None,
            })
            .ok_or_else(|| TemplateError::UnknownSetting(key.clone())),
        Source::Item(field) => scope
            .item
            .then(|| lookup(Some(OwnNotice::FIELDS), field))
            .flatten()
            .ok_or_else(|| TemplateError::Item(field.clone())),
    }
}

fn lookup(fields: Option<&[(&str, ValueType)]>, name: &str) -> Option<ValueType> {
    fields?
        .iter()
        .find(|(field, _)| *field == name)
        .map(|(_, ty)| *ty)
}

fn apply(filter: Filter, ty: Ty) -> Result<Ty, TemplateError> {
    let refuse = |input| {
        Err(TemplateError::FilterInput {
            filter: filter.name(),
            input,
        })
    };
    match (filter, ty) {
        (Filter::Round(_) | Filter::Percent, Ty::Value(ValueType::Number)) => {
            Ok(Ty::Value(ValueType::Text))
        }
        (Filter::HhMm, Ty::Value(ValueType::Time)) => Ok(Ty::Value(ValueType::Text)),
        (Filter::Threshold { .. }, Ty::Value(ValueType::Number)) => Ok(Ty::Tone),
        (_, Ty::Tone) => refuse("a tone"),
        (_, Ty::Value(value)) => refuse(describe(value)),
    }
}
