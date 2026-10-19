//! Native snapshot, filtered pairs and captured secrets retain their real storage.
use super::budget::SourceBudget;
use agent_client_protocol::managed::{Buffer, ManagedError, Source};
use nocterm_ai::{connection::ManagedLaunch, env, model::json::deferred::JsonDecodeError};
use std::ops::Range;
type Error = ManagedError<JsonDecodeError>;
struct Entry {
    key: Range<usize>,
    value: Range<usize>,
    ordinal: usize,
}
struct Pair {
    key: Source<SourceBudget>,
    value: Source<SourceBudget>,
}
pub(super) struct Environment {
    pairs: Buffer<Pair, SourceBudget>,
}
fn explicit(key: &str, value: &str) -> bool {
    env::valid_name(key)
        && !env::blocked(key)
        && !env::credential_override(key)
        && !value.contains('\0')
}
fn inherited(key: &str, value: &str, launch: &ManagedLaunch) -> bool {
    env::valid_name(key)
        && !env::blocked(key)
        && !value.contains('\0')
        && (env::allowed(key) || launch.inherit_env().iter().any(|name| name == key))
}
fn pair<'a>(raw: &'a [u8], entry: &Entry) -> Option<(&'a str, &'a str)> {
    Some((
        std::str::from_utf8(&raw[entry.key.clone()]).ok()?,
        std::str::from_utf8(&raw[entry.value.clone()]).ok()?,
    ))
}
impl Environment {
    pub(super) fn from_snapshot(
        raw: &[u8],
        launch: &ManagedLaunch,
        budget: &SourceBudget,
    ) -> Result<Self, Error> {
        let mut entries = Buffer::new(budget.clone());
        let mut offset = 0;
        for (ordinal, bytes) in raw.split_inclusive(|byte| *byte == 0).enumerate() {
            let bytes = bytes
                .strip_suffix(&[0])
                .ok_or(ManagedError::InvalidEnvelope)?;
            if let Some(equal) = bytes
                .get(1..)
                .and_then(|tail| tail.iter().position(|byte| *byte == b'='))
                .map(|index| index + 1)
            {
                let entry = Entry {
                    key: offset..offset + equal,
                    value: offset + equal + 1..offset + bytes.len(),
                    ordinal,
                };
                if pair(raw, &entry).is_some_and(|(key, value)| inherited(key, value, launch)) {
                    entries.push(entry).map_err(|(error, _)| error)?;
                }
            }
            offset += bytes.len() + 1;
        }
        entries.as_mut_slice().sort_unstable_by(|left, right| {
            raw[left.key.clone()]
                .cmp(&raw[right.key.clone()])
                .then(left.ordinal.cmp(&right.ordinal))
        });
        let mut pairs = Buffer::new(budget.clone());
        let accepted = entries
            .as_slice()
            .iter()
            .enumerate()
            .filter(|(index, entry)| last(*index, entry, entries.as_slice(), raw))
            .filter(|(_, entry)| {
                pair(raw, entry).is_some_and(|(key, _)| {
                    !launch
                        .env()
                        .get(key)
                        .is_some_and(|value| explicit(key, value))
                })
            })
            .count();
        let overrides = launch
            .env()
            .iter()
            .filter(|(key, value)| explicit(key, value))
            .count();
        pairs.reserve(
            accepted
                .checked_add(overrides)
                .ok_or(ManagedError::CapacityOverflow)?,
        )?;
        for (index, entry) in entries.as_slice().iter().enumerate() {
            let Some((key, value)) = pair(raw, entry) else {
                continue;
            };
            if last(index, entry, entries.as_slice(), raw)
                && !launch
                    .env()
                    .get(key)
                    .is_some_and(|value| explicit(key, value))
            {
                insert(&mut pairs, key, value, budget)?;
            }
        }
        for (key, value) in launch
            .env()
            .iter()
            .filter(|(key, value)| explicit(key, value))
        {
            insert(&mut pairs, key, value, budget)?;
        }
        pairs
            .as_mut_slice()
            .sort_unstable_by(|left, right| left.key.text().cmp(right.key.text()));
        Ok(Self { pairs })
    }
    pub(super) fn iter(&self) -> impl ExactSizeIterator<Item = (&str, &str)> + Clone {
        self.pairs
            .as_slice()
            .iter()
            .map(|pair| (pair.key.text(), pair.value.text()))
    }
    pub(super) fn get(&self, key: &str) -> Option<&str> {
        self.pairs
            .as_slice()
            .binary_search_by(|pair| pair.key.text().cmp(key))
            .ok()
            .map(|index| self.pairs.as_slice()[index].value.text())
    }
    pub(super) fn secrets(
        &self,
        launch: &ManagedLaunch,
        budget: &SourceBudget,
    ) -> Result<Buffer<Source<SourceBudget>, SourceBudget>, Error> {
        let mut captured = Buffer::new(budget.clone());
        let inherited = self
            .iter()
            .filter(|(key, _)| secret_name(key))
            .map(|(_, value)| value);
        for (ordinal, value) in inherited
            .chain(launch.env().iter().map(|(_, value)| value.as_str()))
            .filter(|value| value.len() >= 4)
            .enumerate()
        {
            captured
                .push(Secret {
                    value: Source::copy(value, budget.clone())?,
                    ordinal,
                })
                .map_err(|(error, _)| error)?;
        }
        captured.as_mut_slice().sort_unstable_by(|left, right| {
            right
                .value
                .text()
                .len()
                .cmp(&left.value.text().len())
                .then(left.ordinal.cmp(&right.ordinal))
        });
        let count = captured
            .as_slice()
            .iter()
            .enumerate()
            .filter(|(index, item)| {
                *index == 0 || captured.as_slice()[index - 1].value.text() != item.value.text()
            })
            .count();
        let mut secrets = Buffer::new(budget.clone());
        secrets.reserve(count)?;
        for item in captured.into_values() {
            if secrets
                .as_slice()
                .last()
                .is_none_or(|last: &Source<SourceBudget>| last.text() != item.value.text())
            {
                secrets.push(item.value).map_err(|(error, _)| error)?;
            }
        }
        Ok(secrets)
    }
}
fn last(index: usize, entry: &Entry, entries: &[Entry], raw: &[u8]) -> bool {
    entries
        .get(index + 1)
        .is_none_or(|next| raw[next.key.clone()] != raw[entry.key.clone()])
}
fn insert(
    pairs: &mut Buffer<Pair, SourceBudget>,
    key: &str,
    value: &str,
    budget: &SourceBudget,
) -> Result<(), Error> {
    pairs
        .push(Pair {
            key: Source::copy(key, budget.clone())?,
            value: Source::copy(value, budget.clone())?,
        })
        .map_err(|(error, _)| error)
}
struct Secret {
    value: Source<SourceBudget>,
    ordinal: usize,
}
fn secret_name(name: &str) -> bool {
    ["KEY", "TOKEN", "PASSWORD", "SECRET", "PROXY"]
        .into_iter()
        .any(|needle| {
            name.char_indices().any(|(start, _)| {
                name[start..]
                    .chars()
                    .flat_map(char::to_uppercase)
                    .take(needle.len())
                    .eq(needle.chars())
            })
        })
}

#[cfg(test)]
mod tests;
