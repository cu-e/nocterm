use super::*;
#[derive(Clone, PartialEq)]
pub(crate) struct ProfileSnapshot {
    profiles: Profiles,
    generation: u64,
    effects: Vec<StampedEffect>,
    epochs: nocterm_connections_model::profile_authority::ObservationEpochs,
}
#[derive(Clone, PartialEq)]
struct StampedEffect {
    generation: u64,
    effect: Effect,
}
#[derive(Clone, PartialEq)]
enum Effect {
    Unlink(Vec<ProfileId>),
    /// Removed profiles that reached their host through jump hosts.
    Forget(Vec<ProfileId>),
    Credential {
        scope: CredentialScope,
        id: CredentialId,
    },
}

impl ProfileSnapshot {
    pub(crate) fn new(profiles: Profiles) -> Self {
        let epochs =
            nocterm_connections_model::profile_authority::ObservationEpochs::imported(&profiles);
        Self {
            profiles,
            generation: 0,
            effects: Vec::new(),
            epochs,
        }
    }
    pub(crate) fn apply(
        &mut self,
        mutation: &ProfileMutation,
        consumed: u64,
        load_error: Option<&Arc<ConnectionStorageError>>,
    ) -> Result<(), ConnectionsError> {
        let mut candidate = self.profiles.clone();
        mutation.apply(&mut candidate)?;
        let effects = match mutation {
            ProfileMutation::Credential { scope, id } => vec![Effect::Credential {
                scope: scope.clone(),
                id: *id,
            }],
            _ => {
                let (routed, direct): (Vec<_>, Vec<_>) = self
                    .profiles
                    .iter()
                    .filter(|profile| candidate.get(profile.id).is_none())
                    .partition(|profile| !profile.jump_hosts.is_empty());
                let ids = |profiles: Vec<&nocterm_connections_model::profiles::Profile>| -> Vec<ProfileId> {
                    profiles.into_iter().map(|profile| profile.id).collect()
                };
                [
                    (!direct.is_empty()).then(|| Effect::Unlink(ids(direct))),
                    (!routed.is_empty()).then(|| Effect::Forget(ids(routed))),
                ]
                .into_iter()
                .flatten()
                .collect()
            }
        };
        if candidate == self.profiles && effects.is_empty() {
            return Ok(());
        }
        if let Some(error) = load_error {
            return Err(ConnectionsError::read_only(error.clone()));
        }
        let generation = self
            .generation
            .checked_add(1)
            .ok_or(ConnectionsError::RevisionExhausted)?;
        self.epochs
            .transition(&self.profiles, &candidate, generation);
        self.profiles = candidate;
        self.generation = generation;
        self.effects.retain(|effect| effect.generation > consumed);
        for effect in effects {
            self.effects.push(StampedEffect {
                generation: self.generation,
                effect,
            });
        }
        Ok(())
    }
    pub(crate) fn profiles(&self) -> &Profiles {
        &self.profiles
    }
    pub(crate) fn into_profiles(self) -> Profiles {
        self.profiles
    }
    pub(crate) fn generation(&self) -> u64 {
        self.generation
    }
    pub(crate) fn has_effects_since(&self, consumed: u64) -> bool {
        self.effects
            .iter()
            .any(|effect| effect.generation > consumed)
    }
    pub(crate) fn publication(
        &self,
    ) -> nocterm_connections_model::profile_authority::ProfilePublication {
        self.epochs.publication(self.generation, &self.profiles)
    }
    pub(crate) fn apply_effects(&self, recents: &mut Recents, consumed: u64) {
        for effect in self
            .effects
            .iter()
            .filter(|effect| effect.generation > consumed)
        {
            match &effect.effect {
                Effect::Unlink(ids) => {
                    for id in ids {
                        recents.unlink(*id);
                    }
                }
                Effect::Forget(ids) => {
                    for id in ids {
                        recents.forget(*id);
                    }
                }
                Effect::Credential { scope, id } => {
                    recents.associate_credential(scope, *id, &self.profiles);
                }
            }
        }
    }
}
