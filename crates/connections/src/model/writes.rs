//! Profile mutations: validation, the ordered write queue and publishing.

use gpui_kit::AsyncApp;
use nocterm_core::persist::Rejected;

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
        if self.queue.is_closing() {
            return Task::ready(Err(rejection(Rejected::Closing)));
        }
        if let Err(error) = check_size(&mutation) {
            return Task::ready(Err(error));
        }
        if self.profiles_file.is_none() && self.queue.is_empty() {
            let result = self
                .candidate(&mutation)
                .map(|profiles| self.publish(profiles, &mutation, cx));
            return Task::ready(result);
        }
        let (done, result) = oneshot::channel();
        match self.queue.push(PendingWrite { mutation, done }) {
            Err(rejected) => return Task::ready(Err(rejection(rejected))),
            Ok(false) => {}
            Ok(true) => {
                self.writer = Some(cx.spawn(async move |this, cx| {
                    while let Some(write) = next_write(&this, cx) {
                        let (done, result) = save(write, &this, cx).await;
                        let _ = this.update(cx, |this, cx| {
                            this.profile_persistence_error = result.clone().err();
                            cx.notify();
                        });
                        let _ = done.send(result);
                    }
                }))
            }
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

fn rejection(rejected: Rejected) -> String {
    match rejected {
        Rejected::Closing => "Connections service is shutting down.",
        Rejected::Full => {
            "Connection save queue is full. Wait for pending saves to finish and retry."
        }
    }
    .into()
}

/// Rejects mutations whose text would not fit in a profile.
fn check_size(mutation: &Mutation) -> Result<(), String> {
    let metadata_bytes = match mutation {
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
        return Err("Connection mutation exceeds the 64 KiB size limit.".into());
    }
    if let Mutation::Save { profile, .. } = mutation {
        let text = toml::to_string(profile.as_ref()).map_err(|error| error.to_string())?;
        if text.len() > MAX_PROFILE_BYTES {
            return Err("Connection fields exceed the 64 KiB size limit.".into());
        }
    }
    Ok(())
}

/// Rebases the oldest queued mutation on the profiles saved so far.
fn next_write(this: &WeakEntity<Connections>, cx: &mut AsyncApp) -> Option<Write> {
    this.update(cx, |this, _| {
        let Some(pending) = this.queue.take() else {
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
            writing: this.queue.begin_write(),
            #[cfg(test)]
            writer: this.test_writer.clone(),
        })
    })
    .ok()
    .flatten()
}

/// Writes one mutation off the main thread and publishes it once saved.
async fn save(
    write: Write,
    this: &WeakEntity<Connections>,
    cx: &mut AsyncApp,
) -> (oneshot::Sender<Result<(), String>>, Result<(), String>) {
    let Write {
        pending,
        candidate,
        path,
        writing,
        #[cfg(test)]
        writer,
    } = write;
    let profiles = match candidate {
        Ok(profiles) => profiles,
        Err(error) => return (pending.done, Err(error)),
    };
    let (profiles, result) = cx
        .background_executor()
        .spawn(async move {
            let _writing = writing;
            #[cfg(test)]
            if let Some(writer) = writer {
                let result = writer(profiles.clone()).await;
                return (profiles, result);
            }
            let result = write_profiles(path, &profiles);
            (profiles, result)
        })
        .await;
    let result = result.and_then(|()| {
        this.update(cx, |this, cx| this.publish(profiles, &pending.mutation, cx))
            .map_err(|_| "Connections service closed during saving.".to_string())
    });
    (pending.done, result)
}
