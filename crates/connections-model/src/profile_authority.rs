//! Pure live-profile authority: every successful candidate transition advances stamps.
use crate::profiles::{ProfileId, Profiles};
use nocterm_session::Target;
use std::collections::HashMap;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ProfileStamp {
    pub incarnation: u64,
    pub endpoint: u64,
    pub icon_override: u64,
    pub country_override: u64,
}
impl ProfileStamp {
    fn at(generation: u64) -> Self {
        Self {
            incarnation: generation,
            endpoint: generation,
            icon_override: generation,
            country_override: generation,
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProfileAuthorityRow {
    pub target: Target,
    pub icon: Option<String>,
    pub country: Option<String>,
    pub stamp: ProfileStamp,
}
/// An immutable projection of currently live committed profiles, without full profile copies.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProfilePublication {
    generation: u64,
    rows: HashMap<ProfileId, ProfileAuthorityRow>,
}
impl ProfilePublication {
    pub fn generation(&self) -> u64 {
        self.generation
    }
    pub fn get(&self, id: ProfileId) -> Option<&ProfileAuthorityRow> {
        self.rows.get(&id)
    }
    pub fn iter(&self) -> impl Iterator<Item = (ProfileId, &ProfileAuthorityRow)> {
        self.rows.iter().map(|(id, row)| (*id, row))
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ObservationEpochs {
    live: HashMap<ProfileId, ProfileStamp>,
}
impl ObservationEpochs {
    pub fn imported(profiles: &Profiles) -> Self {
        Self {
            live: profiles
                .iter()
                .map(|p| (p.id, ProfileStamp::at(0)))
                .collect(),
        }
    }
    pub fn transition(&mut self, before: &Profiles, after: &Profiles, generation: u64) {
        self.live.retain(|id, _| after.get(*id).is_some());
        for profile in after.iter() {
            let Some(previous) = before.get(profile.id) else {
                self.live.insert(profile.id, ProfileStamp::at(generation));
                continue;
            };
            let stamp = self
                .live
                .entry(profile.id)
                .or_insert(ProfileStamp::at(generation));
            // The same address behind other jump hosts is another machine, so what
            // was detected on the old one no longer applies.
            if previous.target != profile.target || previous.jump_hosts != profile.jump_hosts {
                stamp.endpoint = generation;
            }
            if previous.icon != profile.icon {
                stamp.icon_override = generation;
            }
            if previous.country != profile.country {
                stamp.country_override = generation;
            }
        }
    }
    pub fn publication(&self, generation: u64, profiles: &Profiles) -> ProfilePublication {
        let rows = profiles
            .iter()
            .filter_map(|profile| {
                self.live.get(&profile.id).map(|stamp| {
                    (
                        profile.id,
                        ProfileAuthorityRow {
                            target: profile.target.clone(),
                            icon: profile.icon.clone(),
                            country: profile.country.clone(),
                            stamp: *stamp,
                        },
                    )
                })
            })
            .collect();
        ProfilePublication { generation, rows }
    }
}
#[cfg(test)]
mod tests;
