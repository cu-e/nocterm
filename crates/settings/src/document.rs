//! The settings file as a set of independently read sections.

use std::{
    any::{Any, TypeId},
    collections::{BTreeMap, HashMap},
    fmt,
    sync::{Mutex, OnceLock, PoisonError},
};

use crate::SettingsSection;

/// The tables of `settings.toml` and the sections read from them.
///
/// A section is read when the code that owns it registers it. A table that
/// does not read as its section runs on that section's defaults, is reported
/// by [`errors`](Self::errors) and is written back unchanged until the
/// section is changed. Tables that no registered section claims are kept as
/// they are.
#[derive(Default)]
pub struct SettingsDocument {
    raw: toml::Table,
    sections: BTreeMap<&'static str, Box<dyn AnySection>>,
    /// Tables that could not be read and have not been changed since.
    broken: BTreeMap<&'static str, String>,
}

impl SettingsDocument {
    /// A document over the tables of a settings file. No section is read yet.
    pub fn from_table(raw: toml::Table) -> Self {
        Self {
            raw,
            ..Self::default()
        }
    }

    /// Reads section `S` from its table, once. A table that does not read
    /// leaves `S` at its defaults and is reported.
    pub fn register<S: SettingsSection>(&mut self) {
        if self.sections.contains_key(S::KEY) {
            return;
        }
        let mut section = match self.raw.get(S::KEY) {
            None => S::default(),
            Some(value) => value.clone().try_into::<S>().unwrap_or_else(|error| {
                self.broken
                    .insert(S::KEY, error.to_string().trim().to_owned());
                S::default()
            }),
        };
        section.sanitize();
        self.sections.insert(S::KEY, Box::new(section));
    }

    /// Whether section `S` was registered.
    pub fn is_registered<S: SettingsSection>(&self) -> bool {
        self.sections.contains_key(S::KEY)
    }

    /// Section `S`.
    ///
    /// A section nobody registered is at its defaults, which is right only
    /// while the file has no table for it; debug builds check that.
    pub fn get<S: SettingsSection>(&self) -> &S {
        match self.sections.get(S::KEY) {
            Some(section) => section
                .as_any()
                .downcast_ref()
                .unwrap_or_else(|| panic!("two settings sections share the key `{}`", S::KEY)),
            None => {
                debug_assert!(
                    !self.raw.contains_key(S::KEY),
                    "settings section `{}` was read before it was registered",
                    S::KEY
                );
                default_of::<S>()
            }
        }
    }

    /// Changes section `S`, then sanitizes it. A broken table that is
    /// changed is rewritten, so its error is cleared. Returns whether the
    /// section changed.
    pub fn update<S: SettingsSection>(&mut self, edit: impl FnOnce(&mut S)) -> bool {
        self.register::<S>();
        let section = self
            .sections
            .get_mut(S::KEY)
            .and_then(|section| section.as_any_mut().downcast_mut::<S>())
            .unwrap_or_else(|| panic!("two settings sections share the key `{}`", S::KEY));
        let before = section.clone();
        edit(section);
        section.sanitize();
        let changed = *section != before;
        if changed {
            self.broken.remove(S::KEY);
        }
        changed
    }

    /// Replaces section `S`.
    pub fn set<S: SettingsSection>(&mut self, value: S) -> bool {
        self.update::<S>(|section| *section = value)
    }

    /// Tables that could not be read and have not been changed since,
    /// followed by tables no registered section claims, by name.
    pub fn errors(&self) -> Vec<SectionError> {
        let mut errors: Vec<_> = self
            .broken
            .iter()
            .map(|(key, error)| SectionError {
                key: (*key).to_owned(),
                error: error.clone(),
            })
            .collect();
        if !self.sections.is_empty() {
            let known = self
                .sections
                .keys()
                .copied()
                .collect::<Vec<_>>()
                .join("`, `");
            errors.extend(
                self.raw
                    .keys()
                    .filter(|key| !self.sections.contains_key(key.as_str()))
                    .map(|key| SectionError {
                        key: key.clone(),
                        error: format!("unknown section, expected one of `{known}`"),
                    }),
            );
        }
        errors.sort_by(|left, right| left.key.cmp(&right.key));
        errors
    }

    /// The tables to write: every readable section without its defaults,
    /// and every other table as it was read.
    pub fn to_table(&self) -> toml::Table {
        let mut table = toml::Table::new();
        for (key, section) in &self.sections {
            if self.broken.contains_key(key) {
                continue;
            }
            let values = section.to_pruned_table();
            if !values.is_empty() {
                table.insert((*key).to_owned(), toml::Value::Table(values));
            }
        }
        for (key, value) in &self.raw {
            if !self.owns(key) {
                table.insert(key.clone(), value.clone());
            }
        }
        table
    }

    /// Whether `key` is a readable registered section, written from its value.
    pub(crate) fn owns(&self, key: &str) -> bool {
        self.sections.contains_key(key) && !self.broken.contains_key(key)
    }
}

impl Clone for SettingsDocument {
    fn clone(&self) -> Self {
        Self {
            raw: self.raw.clone(),
            sections: self
                .sections
                .iter()
                .map(|(key, section)| (*key, section.clone_box()))
                .collect(),
            broken: self.broken.clone(),
        }
    }
}

impl PartialEq for SettingsDocument {
    fn eq(&self, other: &Self) -> bool {
        self.broken == other.broken
            && self.sections.len() == other.sections.len()
            && self.sections.iter().all(|(key, section)| {
                other
                    .sections
                    .get(key)
                    .is_some_and(|other| section.same(other.as_ref()))
            })
    }
}

impl fmt::Debug for SettingsDocument {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SettingsDocument")
            .field("sections", &self.sections)
            .field("broken", &self.broken)
            .finish_non_exhaustive()
    }
}

/// A top-level table of `settings.toml` that could not be read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SectionError {
    /// The table's name, such as `monitor`.
    pub key: String,
    pub error: String,
}

impl fmt::Display for SectionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "[{}]: {}", self.key, self.error)
    }
}

/// A section behind a type-erased box.
trait AnySection: Any + Send + Sync + fmt::Debug {
    fn clone_box(&self) -> Box<dyn AnySection>;
    fn same(&self, other: &dyn AnySection) -> bool;
    /// The section's values that differ from its defaults.
    fn to_pruned_table(&self) -> toml::Table;
    fn as_any(&self) -> &dyn Any;
    fn as_any_mut(&mut self) -> &mut dyn Any;
}

impl<S: SettingsSection> AnySection for S {
    fn clone_box(&self) -> Box<dyn AnySection> {
        Box::new(self.clone())
    }

    fn same(&self, other: &dyn AnySection) -> bool {
        other.as_any().downcast_ref::<S>() == Some(self)
    }

    fn to_pruned_table(&self) -> toml::Table {
        let mut table = to_table(self);
        prune_defaults(&mut table, &to_table(&S::default()));
        table
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

fn to_table<S: SettingsSection>(section: &S) -> toml::Table {
    toml::Table::try_from(section)
        .unwrap_or_else(|error| panic!("section `{}` is not a TOML table: {error}", S::KEY))
}

/// Removes every entry of `table` that equals its counterpart in `defaults`.
fn prune_defaults(table: &mut toml::Table, defaults: &toml::Table) {
    table.retain(|key, value| match (value, defaults.get(key)) {
        (toml::Value::Table(table), Some(toml::Value::Table(defaults))) => {
            prune_defaults(table, defaults);
            !table.is_empty()
        }
        (value, Some(default)) => value != default,
        (_, None) => true,
    });
}

/// The defaults of a section nobody registered, made once per type.
fn default_of<S: SettingsSection>() -> &'static S {
    type Defaults = HashMap<TypeId, &'static (dyn Any + Send + Sync)>;
    static DEFAULTS: OnceLock<Mutex<Defaults>> = OnceLock::new();
    let mut defaults = DEFAULTS
        .get_or_init(Mutex::default)
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    let default = *defaults.entry(TypeId::of::<S>()).or_insert_with(|| {
        let mut default = S::default();
        default.sanitize();
        Box::leak(Box::new(default))
    });
    default
        .downcast_ref()
        .expect("defaults are stored by their type")
}

#[cfg(test)]
#[path = "document/tests.rs"]
mod tests;
