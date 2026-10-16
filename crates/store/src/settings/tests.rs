//! Checked edits validate actual ordered settings candidates, before publication.
use super::*;
use crate::test_support::gated_writer;
use futures::FutureExt as _;
use gpui::TestAppContext;
use nocterm_session::{LocalShellSettings, SshSettings};
use std::error::Error as _;

fn memory(cx: &mut TestAppContext) -> SettingsHandle {
    cx.update(|cx| {
        SettingsConfig::in_memory(SettingsDocument::default())
            .with(LocalShellSettings::default())
            .with(SshSettings::default())
            .install(cx)
    })
}

#[gpui::test]
async fn memory_checked_edits_are_immediate_atomic_and_keep_semantic_revisions(
    cx: &mut TestAppContext,
) {
    let settings = memory(cx);
    let other = memory(cx);
    let error = StoreError::edit(std::io::Error::from(std::io::ErrorKind::InvalidInput));
    let original = error.source().unwrap().source().unwrap() as *const dyn std::error::Error;
    let rejected = cx.update(|cx| {
        settings.edit_checked(cx, move |document| {
            document
                .update::<LocalShellSettings>(|section| section.program = Some("discard".into()));
            Err(error)
        })
    });
    let rejected = rejected.await.unwrap_err();
    assert!(std::ptr::addr_eq(
        rejected.source().unwrap().source().unwrap(),
        original
    ));
    assert!(
        rejected
            .source()
            .unwrap()
            .source()
            .unwrap()
            .is::<std::io::Error>()
    );
    cx.update(|cx| {
        assert!(settings.setting::<LocalShellSettings>(cx).program.is_none());
        assert_eq!(settings.revision(cx), 0);
    });
    let accepted = cx.update(|cx| {
        settings.update_checked::<LocalShellSettings>(cx, |section| {
            section.program = Some("accepted".into());
            Ok(())
        })
    });
    cx.update(|cx| {
        assert_eq!(
            settings
                .setting::<LocalShellSettings>(cx)
                .program
                .as_deref(),
            Some("accepted")
        );
        assert!(other.setting::<LocalShellSettings>(cx).program.is_none());
    });
    assert_eq!(accepted.await.unwrap(), 1);
    assert_eq!(
        cx.update(|cx| settings.update_checked::<LocalShellSettings>(cx, |_| Ok(())))
            .await
            .unwrap(),
        1
    );
}

#[gpui::test]
async fn ordered_validation_uses_successful_predecessor_and_discards_mutate_then_error(
    cx: &mut TestAppContext,
) {
    let (writer, release, writes) = gated_writer().into_parts();
    let settings = cx.update(|cx| {
        SettingsConfig::with_writer(SettingsDocument::default(), writer)
            .with(LocalShellSettings::default())
            .install(cx)
    });
    let first = cx.update(|cx| {
        settings.update::<LocalShellSettings>(cx, |section| section.program = Some("prior".into()))
    });
    cx.run_until_parked();
    let second = cx.update(|cx| {
        settings.update_checked::<LocalShellSettings>(cx, |section| {
            if section.program.as_deref() != Some("prior") {
                return Err(StoreError::PendingWrites);
            }
            section.args = vec!["retained".into()];
            Ok(())
        })
    });
    let rejected = cx.update(|cx| {
        settings.update_checked::<LocalShellSettings>(cx, |section| {
            section.program = Some("discard".into());
            Err(StoreError::edit(std::io::Error::from(
                std::io::ErrorKind::InvalidInput,
            )))
        })
    });
    let third = cx.update(|cx| {
        settings.update_checked::<LocalShellSettings>(cx, |section| {
            assert_eq!(section.program.as_deref(), Some("prior"));
            assert_eq!(section.args, ["retained"]);
            section.cwd = Some("/final".into());
            Ok(())
        })
    });
    release.send(Ok(())).unwrap();
    cx.run_until_parked();
    assert_eq!(first.await.unwrap(), 1);
    assert_eq!(second.await.unwrap(), 2);
    assert!(matches!(rejected.await, Err(StoreError::Edit(_))));
    assert_eq!(third.await.unwrap(), 3);
    assert_eq!(writes.lock().unwrap().len(), 3);
    cx.update(|cx| {
        assert_eq!(
            settings.setting::<LocalShellSettings>(cx).cwd.as_deref(),
            Some("/final")
        )
    });
}

#[gpui::test]
async fn failed_predecessor_does_not_enter_the_next_checked_base(cx: &mut TestAppContext) {
    let (writer, release, writes) = gated_writer().into_parts();
    let settings = cx.update(|cx| {
        SettingsConfig::with_writer(SettingsDocument::default(), writer)
            .with(LocalShellSettings::default())
            .install(cx)
    });
    let first = cx.update(|cx| {
        settings.update_checked::<LocalShellSettings>(cx, |section| {
            section.program = Some("failed".into());
            Ok(())
        })
    });
    cx.run_until_parked();
    let second = cx.update(|cx| {
        settings.update_checked::<LocalShellSettings>(cx, |section| {
            assert!(
                section.program.is_none(),
                "physical rejection must not enter the ordered base"
            );
            section.args.push("survivor".into());
            Ok(())
        })
    });
    release
        .send(Err(StoreError::backend(std::io::Error::from(
            std::io::ErrorKind::PermissionDenied,
        ))))
        .unwrap();
    cx.run_until_parked();
    let error = first.await.unwrap_err();
    assert!(matches!(error, StoreError::Backend(_)));
    assert_eq!(
        error
            .source()
            .unwrap()
            .source()
            .unwrap()
            .downcast_ref::<std::io::Error>()
            .unwrap()
            .kind(),
        std::io::ErrorKind::PermissionDenied
    );
    assert_eq!(second.await.unwrap(), 1);
    let writes = writes.lock().unwrap();
    assert_eq!(writes.len(), 2);
    assert!(writes[1].get::<LocalShellSettings>().program.is_none());
    assert_eq!(writes[1].get::<LocalShellSettings>().args, ["survivor"]);
    cx.update(|cx| assert_eq!(settings.revision(cx), 1));
}

#[gpui::test]
async fn checked_queue_full_closing_and_capture_keep_receipts_after_owner_drop(
    cx: &mut TestAppContext,
) {
    let (writer, release, writes) = gated_writer().into_parts();
    let settings = cx.update(|cx| {
        SettingsConfig::with_writer(SettingsDocument::default(), writer)
            .with(LocalShellSettings::default())
            .install(cx)
    });
    let first = cx.update(|cx| {
        settings.update_checked::<LocalShellSettings>(cx, |section| {
            section.args = vec!["0".into()];
            Ok(())
        })
    });
    cx.run_until_parked();
    let queued = cx.update(|cx| {
        (1..=SETTINGS_CAPACITY)
            .map(|index| {
                settings.update_checked::<LocalShellSettings>(cx, move |section| {
                    section.args = vec![index.to_string()];
                    Ok(())
                })
            })
            .collect::<Vec<_>>()
    });
    let full = cx.update(|cx| settings.update_checked::<LocalShellSettings>(cx, |_| Ok(())));
    assert!(matches!(full.await, Err(StoreError::Full)));
    let work = cx.update(|cx| settings.capture_shutdown(cx));
    let closing = cx.update(|cx| settings.update_checked::<LocalShellSettings>(cx, |_| Ok(())));
    assert!(matches!(closing.await, Err(StoreError::Closing)));
    drop(settings);
    let final_task = cx.background_executor.spawn(work);
    release.send(Ok(())).unwrap();
    final_task.await.unwrap();
    assert_eq!(first.await.unwrap(), 1);
    for (index, receipt) in queued.into_iter().enumerate() {
        assert_eq!(receipt.await.unwrap(), index as u64 + 2);
    }
    assert_eq!(
        writes
            .lock()
            .unwrap()
            .last()
            .unwrap()
            .get::<LocalShellSettings>()
            .args,
        [SETTINGS_CAPACITY.to_string()]
    );
}

#[gpui::test]
async fn durable_noop_returns_semantic_revision_without_publication_notification(
    cx: &mut TestAppContext,
) {
    let (writer, release, writes) = gated_writer().into_parts();
    let settings = cx.update(|cx| {
        SettingsConfig::with_writer(SettingsDocument::default(), writer)
            .with(LocalShellSettings::default())
            .install(cx)
    });
    let notifications = Rc::new(Cell::new(0));
    let observed = notifications.clone();
    let _subscription =
        cx.update(|cx| settings.observe(cx, move |_| observed.set(observed.get() + 1)));
    let first = cx.update(|cx| {
        settings.update_checked::<LocalShellSettings>(cx, |section| {
            section.program = Some("changed".into());
            Ok(())
        })
    });
    cx.run_until_parked();
    let mut noop = cx.update(|cx| settings.update_checked::<LocalShellSettings>(cx, |_| Ok(())));
    cx.run_until_parked();
    assert!((&mut noop).now_or_never().is_none());
    assert_eq!(notifications.get(), 0);
    release.send(Ok(())).unwrap();
    cx.run_until_parked();
    assert_eq!(first.await.unwrap(), 1);
    assert_eq!(noop.await.unwrap(), 1);
    assert_eq!(notifications.get(), 1);
    assert_eq!(
        writes.lock().unwrap().len(),
        1,
        "durable noop follows its predecessor without another physical write"
    );
}

#[gpui::test]
async fn late_schema_hydration_keeps_checked_candidate_unknown_and_broken_tables(
    cx: &mut TestAppContext,
) {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("settings.toml");
    std::fs::write(
        &path,
        "[local]\nprogram='raw'\n[ssh]\nconnect_timeout_secs='broken'\n[unknown]\nkeep=true\n",
    )
    .unwrap();
    let mut document = SettingsFile::new(path).load().unwrap();
    document.register::<LocalShellSettings>();
    let (writer, release, writes) = gated_writer().into_parts();
    let settings = cx.update(|cx| SettingsConfig::with_writer(document, writer).install(cx));
    let edit = cx.update(|cx| {
        settings.update_checked::<LocalShellSettings>(cx, |section| {
            section.args = vec!["queued".into()];
            Ok(())
        })
    });
    cx.run_until_parked();
    cx.update(|cx| {
        settings.register::<SshSettings>(cx);
        assert_eq!(settings.revision(cx), 0);
        assert!(
            settings
                .section_errors(cx)
                .iter()
                .any(|error| error.key == "ssh")
        );
    });
    release.send(Ok(())).unwrap();
    cx.run_until_parked();
    assert_eq!(edit.await.unwrap(), 1);
    cx.update(|cx| {
        assert_eq!(settings.setting::<SshSettings>(cx).connect_timeout_secs, 15);
        assert!(
            settings
                .section_errors(cx)
                .iter()
                .any(|error| error.key == "ssh")
        );
        assert_eq!(
            settings.document(cx).to_table()["unknown"]["keep"].as_bool(),
            Some(true)
        );
        assert_eq!(
            settings.document(cx).to_table()["ssh"]["connect_timeout_secs"].as_str(),
            Some("broken")
        );
    });
    assert_eq!(writes.lock().unwrap().len(), 1);
}

#[gpui::test]
async fn captured_checked_rejection_keeps_later_base_and_cannot_publish_partial_mutations(
    cx: &mut TestAppContext,
) {
    let (writer, release, writes) = gated_writer().into_parts();
    let settings = cx.update(|cx| {
        SettingsConfig::with_writer(SettingsDocument::default(), writer)
            .with(LocalShellSettings::default())
            .install(cx)
    });
    let active = cx.update(|cx| {
        settings.update_checked::<LocalShellSettings>(cx, |section| {
            section.program = Some("original".into());
            Ok(())
        })
    });
    cx.run_until_parked();
    let rejected = cx.update(|cx| {
        settings.edit_checked(cx, |document| {
            document
                .update::<LocalShellSettings>(|section| section.program = Some("discard".into()));
            Err(StoreError::edit(std::io::Error::from(
                std::io::ErrorKind::InvalidInput,
            )))
        })
    });
    let later = cx.update(|cx| {
        settings.update_checked::<LocalShellSettings>(cx, |section| {
            assert_eq!(section.program.as_deref(), Some("original"));
            section.args = vec!["final".into()];
            Ok(())
        })
    });
    let work = cx.update(|cx| settings.capture_shutdown(cx));
    assert!(matches!(rejected.await, Err(StoreError::Edit(_))));
    drop(settings);
    let final_task = cx.background_executor.spawn(work);
    release.send(Ok(())).unwrap();
    final_task.await.unwrap();
    assert_eq!(active.await.unwrap(), 1);
    assert_eq!(later.await.unwrap(), 2);
    let writes = writes.lock().unwrap();
    let final_document = writes.last().unwrap();
    assert_eq!(
        final_document
            .get::<LocalShellSettings>()
            .program
            .as_deref(),
        Some("original")
    );
    assert_eq!(final_document.get::<LocalShellSettings>().args, ["final"]);
}

#[gpui::test]
async fn checked_section_publishes_sanitized_value_and_counts_sanitized_noop_once(
    cx: &mut TestAppContext,
) {
    let settings = memory(cx);
    let first = cx.update(|cx| {
        settings.update_checked::<SshSettings>(cx, |section| {
            section.connect_timeout_secs = 0;
            Ok(())
        })
    });
    assert_eq!(first.await.unwrap(), 1);
    cx.update(|cx| assert_eq!(settings.setting::<SshSettings>(cx).connect_timeout_secs, 1));
    let same = cx.update(|cx| {
        settings.update_checked::<SshSettings>(cx, |section| {
            section.connect_timeout_secs = 0;
            Ok(())
        })
    });
    assert_eq!(same.await.unwrap(), 1);
    cx.update(|cx| assert_eq!(settings.revision(cx), 1));
}
