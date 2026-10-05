//! Transfers as each Explorer half shows them: a progress strip while they
//! run, and a fresh listing of the folder they changed when they finish.

use gpui_kit::{
    AnyElement, App, SharedString,
    component::{ActiveTheme as _, Icon, Sizable as _, h_flex, progress::Progress as Bar, v_flex},
    div,
    prelude::*,
};
use nocterm_session::{Target, fs::path};
use nocterm_transfers::{Progress, TransferDirection};
use nocterm_ui::IconName;
use std::path::Path;

use crate::{ShowTransfers, statistics::bytes};

/// The transfer queue as last seen.
#[derive(Default)]
pub(crate) struct Activity {
    jobs: Vec<Progress>,
}

impl Activity {
    /// Takes a new snapshot; whether anything changed, and the jobs that
    /// finished since the last one.
    pub(crate) fn update(&mut self, snapshot: Vec<Progress>) -> (bool, Vec<Progress>) {
        if snapshot == self.jobs {
            return (false, Vec::new());
        }
        let finished = snapshot
            .iter()
            .filter(|job| job.state.finished() && job.completed_files > 0)
            .filter(|job| {
                !self
                    .jobs
                    .iter()
                    .any(|old| old.id == job.id && old.state.finished())
            })
            .cloned()
            .collect();
        self.jobs = snapshot;
        (true, finished)
    }

    /// Unfinished uploads to `target`.
    pub(crate) fn uploads<'a>(&'a self, target: &'a Target) -> impl Iterator<Item = &'a Progress> {
        self.jobs.iter().filter(move |job| {
            !job.state.finished()
                && job.direction == TransferDirection::Upload
                && job.target == *target
        })
    }

    /// Unfinished downloads.
    pub(crate) fn downloads(&self) -> impl Iterator<Item = &Progress> {
        self.jobs
            .iter()
            .filter(|job| !job.state.finished() && job.direction == TransferDirection::Download)
    }
}

/// Whether a finished upload changed the remote folder `shown` on `target`:
/// it was copied into that folder or into one of its subfolders.
pub(crate) fn changes_remote(job: &Progress, target: &Target, shown: &str) -> bool {
    let trim = |value: &str| value.trim_end_matches('/').to_owned();
    job.direction == TransferDirection::Upload
        && job.target == *target
        && (trim(&job.destination) == trim(shown)
            || trim(path::parent(&job.destination)) == trim(shown))
}

/// Whether a finished download changed the local folder `shown`.
pub(crate) fn changes_local(job: &Progress, shown: &Path) -> bool {
    let destination = Path::new(&job.destination);
    job.direction == TransferDirection::Download
        && (destination == shown || destination.parent() == Some(shown))
}

/// One line per running transfer, with its progress; clicking opens the
/// transfer queue.
pub(crate) fn strip<'a>(
    id: &'static str,
    jobs: impl Iterator<Item = &'a Progress>,
    cx: &App,
) -> Option<AnyElement> {
    let rows: Vec<AnyElement> = jobs
        .take(3)
        .map(|job| {
            let verb = match job.direction {
                TransferDirection::Upload => "Uploading",
                TransferDirection::Download => "Downloading",
            };
            let fraction = if job.total_bytes > 0 {
                job.sent_bytes as f32 / job.total_bytes as f32
            } else if job.discovered_files > 0 {
                (job.completed_files + job.skipped_files) as f32 / job.discovered_files as f32
            } else {
                0.
            };
            v_flex()
                .gap_0p5()
                .child(
                    h_flex()
                        .gap_1()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child(
                            Icon::new(match job.direction {
                                TransferDirection::Upload => IconName::ArrowUp,
                                TransferDirection::Download => IconName::ArrowDown,
                            })
                            .xsmall(),
                        )
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .overflow_hidden()
                                .text_ellipsis()
                                .child(format!(
                                    "{verb} {}/{} · {} / {}{}",
                                    job.completed_files,
                                    job.discovered_files,
                                    bytes(job.sent_bytes),
                                    bytes(job.total_bytes),
                                    if job.discovery_complete {
                                        ""
                                    } else {
                                        " · discovering…"
                                    }
                                )),
                        ),
                )
                .child(
                    Bar::new(SharedString::from(format!("{id}-{}", job.id)))
                        .small()
                        .w_full()
                        .accessibility_label("Transfer progress")
                        .loading(!job.discovery_complete)
                        .value(fraction.clamp(0., 1.) * 100.),
                )
                .into_any_element()
        })
        .collect();
    if rows.is_empty() {
        return None;
    }
    Some(
        v_flex()
            .id(id)
            .gap_1()
            .py_1()
            .cursor_pointer()
            .children(rows)
            .on_click(|_, window, cx| window.dispatch_action(Box::new(ShowTransfers), cx))
            .into_any_element(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use nocterm_transfers::TransferState;

    fn job(direction: TransferDirection, destination: &str, state: TransferState) -> Progress {
        Progress {
            id: uuid::Uuid::new_v4(),
            target: Target::parse("u@h", None).unwrap(),
            destination: destination.into(),
            direction,
            state,
            discovered_files: 1,
            completed_files: 1,
            skipped_files: 0,
            failed_files: 0,
            total_bytes: 1,
            sent_bytes: 1,
            discovery_complete: true,
            errors: Vec::new(),
        }
    }

    #[test]
    fn a_job_is_reported_finished_once() {
        let mut activity = Activity::default();
        let mut running = job(TransferDirection::Upload, "/srv", TransferState::Running);
        assert_eq!(activity.update(vec![running.clone()]).1.len(), 0);
        running.state = TransferState::Completed;
        assert_eq!(activity.update(vec![running.clone()]).1.len(), 1);
        assert_eq!(activity.update(vec![running]), (false, Vec::new()));
    }

    #[test]
    fn finished_transfers_change_their_folder_and_its_parent() {
        let target = Target::parse("u@h", None).unwrap();
        let upload = job(
            TransferDirection::Upload,
            "/srv/app/",
            TransferState::Completed,
        );
        assert!(changes_remote(&upload, &target, "/srv/app"));
        assert!(changes_remote(&upload, &target, "/srv"));
        assert!(!changes_remote(&upload, &target, "/etc"));
        let other = Target::parse("u@other", None).unwrap();
        assert!(!changes_remote(&upload, &other, "/srv/app"));
        let download = job(
            TransferDirection::Download,
            "/tmp/in",
            TransferState::Completed,
        );
        assert!(changes_local(&download, Path::new("/tmp/in")));
        assert!(changes_local(&download, Path::new("/tmp")));
        assert!(!changes_local(&upload, Path::new("/srv/app")));
    }
}
