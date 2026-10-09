use super::*;
use crate::{SettingsExt as _, TerminalSettings};
use futures::FutureExt as _;
use gpui_kit::TestAppContext;
use std::sync::{Arc, Mutex};

#[gpui_kit::test]
fn quit_flushes_accepted_settings_before_foreground_writer_runs(cx: &mut TestAppContext) {
    let directory = tempfile::tempdir().unwrap();
    let file = directory.path().join("settings.toml");
    let pending = cx.update(|cx| {
        cx.set_global(SettingsStore::new(
            SettingsDocument::default(),
            SettingsFile::new(&file),
        ));
        super::super::init(cx);
        super::super::register_setting::<TerminalSettings>(cx);
        let first = cx.update_setting::<TerminalSettings>(|s| s.font_size = Some(23.));
        let second = cx.update_setting::<TerminalSettings>(|s| s.copy_on_select = true);
        (first, second)
    });
    cx.quit();
    let mut saved = SettingsFile::new(&file).load().unwrap();
    saved.register::<TerminalSettings>();
    assert_eq!(saved.get::<TerminalSettings>().font_size, Some(23.));
    assert!(saved.get::<TerminalSettings>().copy_on_select);
    drop(pending);
}

#[gpui_kit::test]
async fn final_capture_keeps_in_flight_candidate_and_all_accepted_edits(cx: &mut TestAppContext) {
    cx.update(|cx| {
        cx.set_global(SettingsStore::new(
            SettingsDocument::default(),
            SettingsFile::new("/test-settings"),
        ));
        super::super::init(cx);
        super::super::register_setting::<TerminalSettings>(cx);
    });
    let writer = cx.update(|cx| cx.global::<SettingsWriter>().0.clone());
    let (release, receive) = oneshot::channel::<()>();
    let first_gate = Arc::new(Mutex::new(Some(receive)));
    let values = Arc::new(Mutex::new(Vec::new()));
    let observed = values.clone();
    writer.update(cx, |writer, _| {
        writer.test_writer = Some(Arc::new(move |value| {
            observed.lock().unwrap().push(value);
            let gate = first_gate.lock().unwrap().take();
            async move {
                if let Some(gate) = gate {
                    let _ = gate.await;
                    return Err("first write failed".into());
                }
                Ok(())
            }
            .boxed()
        }));
    });
    let first = cx.update(|cx| cx.update_setting::<TerminalSettings>(|s| s.font_size = Some(18.)));
    cx.run_until_parked();
    let second = cx.update(|cx| cx.update_setting::<TerminalSettings>(|s| s.copy_on_select = true));
    let (job, acknowledgements) = writer
        .update(cx, |writer, cx| writer.capture_shutdown(cx))
        .unwrap();
    let final_work = cx.background_executor.spawn(async move {
        let gate = job.gate.clone();
        let _guard = gate.final_write().await;
        let result = save(job).await;
        for (done, revision) in acknowledgements {
            let _ = done.send(result.clone().map(|()| revision));
        }
    });
    cx.run_until_parked();
    assert_eq!(
        values.lock().unwrap().len(),
        1,
        "final write must wait for active disk work"
    );
    release.send(()).unwrap();
    final_work.await;
    assert_eq!(first.await.unwrap(), 1);
    assert_eq!(second.await.unwrap(), 2);
    let values = values.lock().unwrap();
    assert_eq!(values.len(), 2);
    let final_settings = values[1].get::<TerminalSettings>();
    assert_eq!(final_settings.font_size, Some(18.));
    assert!(final_settings.copy_on_select);
    cx.update(|cx| {
        assert_eq!(
            cx.global::<SettingsStore>().revision(),
            0,
            "quit work must not publish through App"
        )
    });
}
