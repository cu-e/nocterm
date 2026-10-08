//! Coalesced, revisioned writes of the recent connections.

use super::*;

impl Connections {
    pub(super) fn schedule_recents(&mut self, cx: &mut Context<Self>) {
        self.recents_revision = self.recents_revision.wrapping_add(1);
        if self.closing || self.recents_file.is_none() || self.recents_writer.is_some() {
            return;
        }
        self.recents_writer = Some(cx.spawn(async move |this, cx| {
            loop {
                let Ok((path, recents, revision, saving)) = this.update(cx, |this, _| {
                    (
                        this.recents_file.clone(),
                        this.recents.clone(),
                        this.recents_revision,
                        this.saving.clone(),
                    )
                }) else {
                    return;
                };
                let result = cx
                    .background_executor()
                    .spawn(async move {
                        let _saving = saving;
                        path.map_or(Ok(()), |path| {
                            persist::save(&path, &recents).map_err(|error| error.to_string())
                        })
                    })
                    .await;
                let again = this.update(cx, |this, cx| {
                    if result.is_ok() {
                        this.recents_written_revision = revision;
                    }
                    let error = result
                        .err()
                        .map(|error| format!("Could not save recent connections: {error}"));
                    if this.recents_persistence_error != error {
                        this.recents_persistence_error = error;
                        cx.notify();
                    }
                    if this.recents_revision != revision {
                        true
                    } else {
                        this.recents_writer = None;
                        false
                    }
                });
                if !matches!(again, Ok(true)) {
                    return;
                }
            }
        }));
    }
}
