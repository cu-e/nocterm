//! One owned native update prepares and publishes through the original Live role.
use super::*;
use crate::{
    history::physical::{PresentationUpdate, RowBody},
    model::retention::document::PolicyLiability,
};

pub(super) struct NoticeApplication<'a> {
    live: &'a mut LiveOwner,
    update: Option<NativeOwned<SessionUpdate>>,
    policy: Option<&'a mut PolicyLiability>,
    now: u64,
    stamp: bool,
}
impl<'a> NoticeApplication<'a> {
    pub(super) fn new(
        live: &'a mut LiveOwner,
        update: NativeOwned<SessionUpdate>,
        policy: Option<&'a mut PolicyLiability>,
        now: u64,
    ) -> Self {
        Self {
            live,
            update: Some(update),
            policy,
            now,
            stamp: false,
        }
    }
    fn transition<R>(
        &mut self,
        edit: impl FnOnce(&mut SnapshotBuilder<'_>) -> Result<(R, bool), PhysicalError>,
    ) -> Result<(R, bool), LiveError> {
        let stamp = self.stamp;
        let now = self.now;
        let pinned = self
            .live
            .live
            .snapshot()
            .ok_or(LiveError::MissingPayload)?
            .metadata()
            .pinned();
        self.live
            .transition_certified(self.policy.take(), |builder| {
                let (value, changed) = edit(builder)?;
                if changed && stamp {
                    builder.stamp_metadata(now, pinned)?;
                }
                Ok((value, changed))
            })
    }
    pub(super) fn run(mut self) -> Result<LiveRouted, LiveError> {
        let now = self.now;
        let (update, allocation) = self
            .update
            .take()
            .expect("owned notice application")
            .into_parts();
        let in_saved_window = self.in_saved_window(&update);
        self.stamp = in_saved_window
            && matches!(
                &update,
                SessionUpdate::UserChunk(_)
                    | SessionUpdate::AgentChunk(_)
                    | SessionUpdate::ThoughtChunk(_)
                    | SessionUpdate::ToolStarted(_)
                    | SessionUpdate::ToolChanged(_)
                    | SessionUpdate::Info(_)
            );
        let changed = match update {
            SessionUpdate::UserChunk(content) => {
                let content = NativeOwned::from_parts(content, allocation);
                self.transition(|b| b.append_user_content(content, now).map(|()| ((), true)))?
                    .1
            }
            SessionUpdate::AgentChunk(content) => self.chunk(content, allocation, false, now)?,
            SessionUpdate::ThoughtChunk(content) => self.chunk(content, allocation, true, now)?,
            SessionUpdate::ToolStarted(record) => {
                let row = NativeOwned::from_parts(Entry::Tool(record), allocation);
                self.transition(|b| b.append_row(row, now).map(|()| ((), true)))?
                    .1
            }
            SessionUpdate::ToolChanged(patch) => {
                let patch = NativeOwned::from_parts(patch, allocation);
                self.transition(|b| b.patch_tool(patch).map(|changed| ((), changed)))?
                    .1
            }
            SessionUpdate::Info(change) => return self.info(change, allocation),
            SessionUpdate::Commands(value) => {
                return self.presentation(PresentationUpdate::Commands(NativeOwned::from_parts(
                    value, allocation,
                )));
            }
            SessionUpdate::Configuration(value) => {
                return self.presentation(PresentationUpdate::Configuration(
                    NativeOwned::from_parts(value, allocation),
                ));
            }
            SessionUpdate::Mode(value) => {
                return self.presentation(PresentationUpdate::Mode(NativeOwned::from_parts(
                    value, allocation,
                )));
            }
            SessionUpdate::Usage(usage) => {
                return self.presentation(PresentationUpdate::Usage(NativeOwned::from_parts(
                    usage, allocation,
                )));
            }
            SessionUpdate::Plan(plan) => {
                let value = NativeOwned::from_parts(plan, allocation);
                self.transition(|builder| {
                    builder.replace_plan(value).map(|changed| ((), changed))
                })?;
                return Ok(LiveRouted::Applied(self.live.applied(false)));
            }
        };
        Ok(LiveRouted::Applied(
            self.live.applied(changed && in_saved_window),
        ))
    }
    /// Tool patches outside the saved window, or with unsaved-only changes,
    /// do not dirty the saved document.
    fn in_saved_window(&self, update: &SessionUpdate) -> bool {
        let SessionUpdate::ToolChanged(patch) = update else {
            return true;
        };
        self.live
            .rows()
            .iter()
            .enumerate()
            .find_map(|(index, row)| {
                matches!(row.view(), RowBody::Tool(tool) if *tool.id() == patch.id)
                    .then_some((index, row))
            })
            .is_some_and(|(index, row)| {
                index >= self.live.saved_start()
                    && matches!(row.view(), RowBody::Tool(tool) if tool.changes_saved(&patch.changes))
            })
    }
    fn info(
        &mut self,
        change: crate::model::controls::Change<String>,
        allocation: crate::model::json::deferred::ledger::Reservation,
    ) -> Result<LiveRouted, LiveError> {
        let title = match change {
            crate::model::controls::Change::Keep => {
                return Ok(LiveRouted::Applied(AppliedUpdate {
                    change: ThreadChange::Metadata,
                    saved_changed: false,
                    revision: self.live.revision,
                }));
            }
            crate::model::controls::Change::Set(value) => Some(value),
            crate::model::controls::Change::Clear => None,
        };
        let current = self
            .live
            .live
            .snapshot()
            .ok_or(LiveError::MissingPayload)?
            .metadata()
            .title();
        let changed = current.as_deref() != title.as_deref();
        drop(current);
        if changed {
            self.transition(|builder| {
                builder
                    .replace_title(NativeOwned::from_parts(title, allocation))
                    .map(|()| ((), true))
            })?;
        }
        Ok(LiveRouted::Applied(AppliedUpdate {
            change: ThreadChange::Metadata,
            saved_changed: changed,
            revision: self.live.revision,
        }))
    }
    fn presentation(&mut self, update: PresentationUpdate) -> Result<LiveRouted, LiveError> {
        self.transition(|builder| {
            builder
                .update_presentation(update)
                .map(|changed| ((), changed))
        })?;
        Ok(LiveRouted::Applied(AppliedUpdate {
            change: ThreadChange::Controls,
            saved_changed: false,
            revision: self.live.revision,
        }))
    }
    fn chunk(
        &mut self,
        content: Content,
        allocation: crate::model::json::deferred::ledger::Reservation,
        thought: bool,
        now: u64,
    ) -> Result<bool, LiveError> {
        Ok(match content {
            Content::Text(text) => {
                // Annotations/extensions of a streamed chunk are not retained,
                // matching the transcript reducer.
                let crate::model::content::Text {
                    text,
                    annotations,
                    extensions,
                } = text;
                // Discarded semantic children retire before atomic certificate
                // consumption can make their policy available to another input.
                drop(annotations);
                drop(extensions);
                let chunk = NativeOwned::from_parts(text, allocation);
                self.transition(|b| {
                    b.append_agent_text(thought, chunk, now)
                        .map(|changed| ((), changed))
                })?
                .1
            }
            other => {
                let row = NativeOwned::from_parts(Entry::Content(other), allocation);
                self.transition(|b| b.append_row(row, now).map(|()| ((), true)))?
                    .1
            }
        })
    }
}
