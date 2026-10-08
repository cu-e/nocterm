//! Profile mutations: validation, the ordered write queue and publishing.

use super::*;

impl Connections {
    pub(super) fn candidate(&self, mutation: &Mutation) -> Result<Profiles, String> {
        let mut profiles = self.profiles.clone();
        match mutation {
            Mutation::Save { profile, expected } => {
                let current = profiles.get(profile.id);
                let matches = match expected {
                    ProfileExpectation::Any => true,
                    ProfileExpectation::Absent => current.is_none(),
                    ProfileExpectation::Snapshot(expected) => current == Some(expected.as_ref()),
                };
                if !matches {
                    return Err("This connection changed or was deleted in another view. Reopen the editor to load its current values; your draft was not saved.".into());
                }
                profiles.upsert(*profile.clone());
            }
            Mutation::Move { id, group, before } => {
                profiles.place(*id, group.clone(), *before)?;
            }
            Mutation::Delete(id) => {
                profiles.remove(*id);
            }
            Mutation::RenameGroup { from, to } => {
                if !profiles.has_group(from) {
                    return Err("Group no longer exists".into());
                }
                let to = to.trim();
                if to.is_empty() {
                    return Err("Group name cannot be empty".into());
                }
                profiles.rename_group(from, to);
            }
            Mutation::Ungroup(group) => {
                if !profiles.has_group(group) {
                    return Err("Group no longer exists".into());
                }
                profiles.ungroup(group);
            }
            Mutation::DeleteGroup { group, expected } => {
                if !profiles.has_group(group) {
                    return Err("Group no longer exists".into());
                }
                let mut current: Vec<_> = profiles
                    .iter()
                    .filter(|profile| profile.group.as_deref() == Some(group.as_str()))
                    .map(|profile| profile.id.to_string())
                    .collect();
                let mut expected: Vec<_> = expected.iter().map(ToString::to_string).collect();
                current.sort();
                expected.sort();
                if current != expected {
                    return Err("Group changed; review it again.".into());
                }
                profiles.remove_group(group);
            }
            Mutation::Credential { target, auth, id } => {
                for profile in profiles
                    .iter_mut()
                    .filter(|p| &p.target == target && &p.auth == auth)
                {
                    profile.credential = Some(*id);
                }
            }
        }
        if profiles != self.profiles
            && let Some(error) = &self.load_error
        {
            return Err(format!(
                "Saved connections are read-only until the file is fixed: {error}"
            ));
        }
        Ok(profiles)
    }
    pub(super) fn enqueue(
        &mut self,
        mutation: Mutation,
        cx: &mut Context<Self>,
    ) -> Task<Result<(), String>> {
        if self.closing {
            return Task::ready(Err("Connections service is shutting down.".into()));
        }
        let metadata_bytes = match &mutation {
            Mutation::Move { group, .. } => group.as_ref().map_or(0, String::len),
            Mutation::RenameGroup { from, to } => from.len().saturating_add(to.len()),
            Mutation::Ungroup(group) | Mutation::DeleteGroup { group, .. } => group.len(),
            Mutation::Credential { target, auth, .. } => target
                .host
                .len()
                .saturating_add(target.user.len())
                .saturating_add(
                    serde_json::to_string(auth).map_or(MAX_PROFILE_BYTES + 1, |text| text.len()),
                ),
            _ => 0,
        };
        if metadata_bytes > MAX_PROFILE_BYTES {
            return Task::ready(Err(
                "Connection mutation exceeds the 64 KiB size limit.".into()
            ));
        }
        if let Mutation::Save { profile, .. } = &mutation {
            match toml::to_string(profile.as_ref()) {
                Ok(text) if text.len() <= MAX_PROFILE_BYTES => {}
                Ok(_) => {
                    return Task::ready(Err(
                        "Connection fields exceed the 64 KiB size limit.".into()
                    ));
                }
                Err(error) => return Task::ready(Err(error.to_string())),
            }
        }
        if self.profiles_file.is_none() && self.pending.is_empty() {
            let result = self
                .candidate(&mutation)
                .map(|profiles| self.publish(profiles, &mutation, cx));
            return Task::ready(result);
        }
        if self.pending.len() >= MAX_PENDING_WRITES {
            return Task::ready(Err(
                "Connection save queue is full. Wait for pending saves to finish and retry.".into(),
            ));
        }
        let (done, result) = oneshot::channel();
        self.pending.push_back(PendingWrite { mutation, done });
        if self.writer.is_none() {
            self.writer = Some(cx.spawn(async move |this, cx| {
                loop {
                    let next = this.update(cx, |this, _| {
                        let Some(pending) = this.pending.pop_front() else {
                            this.writer = None;
                            return None;
                        };
                        let candidate = this.candidate(&pending.mutation);
                        let path = if candidate
                            .as_ref()
                            .is_ok_and(|profiles| profiles == &this.profiles)
                        {
                            None
                        } else {
                            this.profiles_file.clone()
                        };
                        Some(Write {
                            pending,
                            candidate,
                            path,
                            saving: this.saving.clone(),
                            #[cfg(test)]
                            writer: this.test_writer.clone(),
                        })
                    });
                    let Ok(Some(write)) = next else {
                        break;
                    };
                    let Write {
                        pending,
                        candidate,
                        path,
                        saving,
                        #[cfg(test)]
                        writer,
                    } = write;
                    let result = match candidate {
                        Ok(profiles) => {
                            let (profiles, result) = cx
                                .background_executor()
                                .spawn(async move {
                                    let _saving = saving;
                                    #[cfg(test)]
                                    let result = if let Some(writer) = writer {
                                        writer(profiles.clone()).await
                                    } else {
                                        write_profiles(path, &profiles)
                                    };
                                    #[cfg(not(test))]
                                    let result = write_profiles(path, &profiles);
                                    (profiles, result)
                                })
                                .await;
                            match result {
                                Ok(()) => this
                                    .update(cx, |this, cx| {
                                        this.publish(profiles, &pending.mutation, cx);
                                        Ok(())
                                    })
                                    .unwrap_or_else(|_| {
                                        Err("Connections service closed during saving.".into())
                                    }),
                                Err(error) => Err(error),
                            }
                        }
                        Err(error) => Err(error),
                    };
                    let _ = this.update(cx, |this, cx| {
                        this.profile_persistence_error = result.clone().err();
                        cx.notify();
                    });
                    let _ = pending.done.send(result);
                }
            }));
        }
        cx.spawn(async move |_, _| {
            result
                .await
                .unwrap_or_else(|_| Err("Connections service closed before saving.".into()))
        })
    }
    pub(super) fn publish(
        &mut self,
        profiles: Profiles,
        mutation: &Mutation,
        cx: &mut Context<Self>,
    ) {
        let kept: std::collections::HashSet<ProfileId> =
            profiles.iter().map(|profile| profile.id).collect();
        let removed: Vec<ProfileId> = self
            .profiles
            .iter()
            .map(|profile| profile.id)
            .filter(|id| !kept.contains(id))
            .collect();
        self.profiles = profiles;
        if !removed.is_empty() {
            for id in removed {
                self.recents.unlink(id);
            }
            self.schedule_recents(cx);
        }
        if let Mutation::Credential { target, auth, id } = mutation {
            for recent in self
                .recents
                .iter_mut()
                .filter(|r| &r.target == target && &r.auth == auth)
            {
                recent.credential = Some(*id);
            }
            self.schedule_recents(cx);
        }
        cx.notify();
    }
}
