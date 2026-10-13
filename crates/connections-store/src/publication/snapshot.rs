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
    Credential {
        target: Target,
        auth: Auth,
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
        let effect = match mutation {
            ProfileMutation::Credential { target, auth, id } => Some(Effect::Credential {
                target: target.clone(),
                auth: auth.clone(),
                id: *id,
            }),
            _ => {
                let removed = self
                    .profiles
                    .iter()
                    .map(|profile| profile.id)
                    .filter(|id| candidate.get(*id).is_none())
                    .collect::<Vec<_>>();
                (!removed.is_empty()).then_some(Effect::Unlink(removed))
            }
        };
        if candidate == self.profiles && effect.is_none() {
            return Ok(());
        }
        if let Some(error) = load_error {
            return Err(ConnectionsError::ReadOnly(error.clone()));
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
        if let Some(effect) = effect {
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
                Effect::Credential { target, auth, id } => {
                    recents.associate_credential(target, auth, *id);
                }
            }
        }
    }
}
