//! The declarative template language.
//!
//! A template is literal text with `{placeholder}` holes. A placeholder is a source and filters:
//! `{clock.now|hh-mm}`, `{monitor.usage.cpu_percent|round(1)}`, `{settings.city}`. There is no
//! concatenation, repetition, regex or function. Values are inserted as literal text and are
//! never parsed again. `{{` and `}}` write a brace.
//!
//! Every placeholder is resolved against the declared capabilities, their fields and the
//! settings keys when the package is read, so a mistake is a manifest error shown before consent.

use alloc::{string::String, string::ToString, vec::Vec};

use thiserror::Error;

mod check;
mod eval;
mod view;

pub use check::{Output, Scope};
pub use eval::Context;
pub use view::{NodeDto, NodeKindName, RenderError, ViewDto, ViewTemplate, ViewTemplateError};

use crate::{CapabilityEntry, Family, capability::capability};

/// Longest template, in characters.
pub const MAX_TEMPLATE_LEN: usize = 512;
/// Most placeholders in one view.
pub const MAX_PLACEHOLDERS: usize = 64;
/// Most decimals `round` keeps.
pub const MAX_ROUND: u8 = 2;

/// Where a placeholder reads from.
#[derive(Clone, Debug, PartialEq)]
pub enum Source {
    /// A field of an observed capability, or the whole `clock.now` instant when `field` is `None`.
    Capability {
        capability: &'static CapabilityEntry,
        field: Option<String>,
    },
    /// A setting of the plugin.
    Setting(String),
    /// A field of the current item of the nearest enclosing list with `each`.
    Item(String),
}

/// A step that turns a value into text, or into a tone.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Filter {
    /// A number with this many decimals.
    Round(u8),
    /// A number rounded to a whole percent, with a percent sign.
    Percent,
    /// A clock instant as local `HH:MM`.
    HhMm,
    /// A number as a tone: below `low` is `success`, from `low` below `high` is `default`, and
    /// `high` or more is `caution`.
    Threshold { low: f64, high: f64 },
}

/// One hole of a template.
#[derive(Clone, Debug, PartialEq)]
pub struct Placeholder {
    pub source: Source,
    pub filters: Vec<Filter>,
}

#[derive(Clone, Debug, PartialEq)]
enum Segment {
    Literal(String),
    Hole(Placeholder),
}

/// Parsed template text.
#[derive(Clone, Debug, PartialEq)]
pub struct Template {
    segments: Vec<Segment>,
}

/// A template, expression or filter the language refuses.
#[derive(Clone, Debug, PartialEq, Eq, Error)]
pub enum TemplateError {
    #[error("a template has at most {MAX_TEMPLATE_LEN} characters")]
    TooLong,
    #[error("a `{{` is not closed, or a `}}` has no opening brace")]
    Braces,
    #[error("`{0}` is not a placeholder like `capability.field`, `settings.key` or `item.field`")]
    Source(String),
    #[error("`{0}` is not an observing capability of this API")]
    UnknownCapability(String),
    #[error("capability `{0}` is not declared by the package")]
    Undeclared(String),
    #[error("`{0}` has no such field")]
    UnknownField(String),
    #[error("`{0}` needs a field")]
    NeedsField(String),
    #[error("there is no setting `{0}`, or it holds several values")]
    UnknownSetting(String),
    #[error("`item.{0}` is outside a list with `each`, or the item has no such field")]
    Item(String),
    #[error("`{0}` is not a filter: use round(n), percent, hh-mm or threshold(low, high)")]
    Filter(String),
    #[error("round keeps at most {MAX_ROUND} decimals")]
    Round,
    #[error("filter `{filter}` does not take {input}")]
    FilterInput {
        filter: &'static str,
        input: &'static str,
    },
    #[error("this place takes {expected}, but the expression gives {found}")]
    Output {
        expected: &'static str,
        found: &'static str,
    },
}

impl Template {
    pub fn parse(text: &str) -> Result<Self, TemplateError> {
        if text.chars().count() > MAX_TEMPLATE_LEN {
            return Err(TemplateError::TooLong);
        }
        let mut segments = Vec::new();
        let mut literal = String::new();
        let mut chars = text.chars().peekable();
        while let Some(c) = chars.next() {
            match (c, chars.peek()) {
                ('{', Some('{')) | ('}', Some('}')) => {
                    literal.push(c);
                    chars.next();
                }
                ('{', _) => {
                    let mut expression = String::new();
                    loop {
                        match chars.next() {
                            Some('}') => break,
                            Some('{') | None => return Err(TemplateError::Braces),
                            Some(c) => expression.push(c),
                        }
                    }
                    if !literal.is_empty() {
                        segments.push(Segment::Literal(core::mem::take(&mut literal)));
                    }
                    segments.push(Segment::Hole(Placeholder::parse(&expression)?));
                }
                ('}', _) => return Err(TemplateError::Braces),
                _ => literal.push(c),
            }
        }
        if !literal.is_empty() {
            segments.push(Segment::Literal(literal));
        }
        Ok(Self { segments })
    }

    /// The placeholders, in order.
    pub fn placeholders(&self) -> impl Iterator<Item = &Placeholder> {
        self.segments.iter().filter_map(|segment| match segment {
            Segment::Hole(hole) => Some(hole),
            Segment::Literal(_) => None,
        })
    }
}

impl Placeholder {
    /// Parses `source|filter|filter`, without braces.
    pub fn parse(expression: &str) -> Result<Self, TemplateError> {
        let mut parts = expression.split('|').map(str::trim);
        let source = Source::parse(parts.next().unwrap_or_default())?;
        let filters = parts.map(Filter::parse).collect::<Result<_, _>>()?;
        Ok(Self { source, filters })
    }
}

impl Source {
    fn parse(text: &str) -> Result<Self, TemplateError> {
        let parts: Vec<&str> = text.split('.').collect();
        if parts.iter().any(|part| part.is_empty()) {
            return Err(TemplateError::Source(text.to_string()));
        }
        match parts.as_slice() {
            ["settings", key] => Ok(Self::Setting((*key).to_string())),
            ["item", field] => Ok(Self::Item((*field).to_string())),
            [domain, name] => Self::capability(domain, name, None),
            [domain, name, field] => Self::capability(domain, name, Some(field)),
            _ => Err(TemplateError::Source(text.to_string())),
        }
    }

    fn capability(domain: &str, name: &str, field: Option<&str>) -> Result<Self, TemplateError> {
        let id = alloc::format!("{domain}.{name}");
        let entry = capability(&id)
            .filter(|entry| entry.family == Family::Observe)
            .ok_or(TemplateError::UnknownCapability(id))?;
        Ok(Self::Capability {
            capability: entry,
            field: field.map(ToString::to_string),
        })
    }
}

impl Filter {
    fn parse(text: &str) -> Result<Self, TemplateError> {
        let bad = || TemplateError::Filter(text.to_string());
        let (name, arguments) = match text.split_once('(') {
            Some((name, rest)) => (name.trim(), Some(rest.strip_suffix(')').ok_or_else(bad)?)),
            None => (text, None),
        };
        let numbers = |arguments: &str| -> Result<Vec<f64>, TemplateError> {
            arguments
                .split(',')
                .map(|number| number.trim().parse::<f64>().map_err(|_| bad()))
                .collect()
        };
        match (name, arguments) {
            ("percent", None) => Ok(Self::Percent),
            ("hh-mm", None) => Ok(Self::HhMm),
            ("round", Some(arguments)) => match numbers(arguments)?.as_slice() {
                &[digits] if digits >= 0.0 && digits == (digits as u8) as f64 => {
                    if digits > f64::from(MAX_ROUND) {
                        return Err(TemplateError::Round);
                    }
                    Ok(Self::Round(digits as u8))
                }
                _ => Err(bad()),
            },
            ("threshold", Some(arguments)) => match numbers(arguments)?.as_slice() {
                &[low, high] if low.is_finite() && high.is_finite() && low <= high => {
                    Ok(Self::Threshold { low, high })
                }
                _ => Err(bad()),
            },
            _ => Err(bad()),
        }
    }

    pub(crate) const fn name(self) -> &'static str {
        match self {
            Self::Round(_) => "round",
            Self::Percent => "percent",
            Self::HhMm => "hh-mm",
            Self::Threshold { .. } => "threshold",
        }
    }
}
