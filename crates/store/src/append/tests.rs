use super::*;
use gpui::TestAppContext;
use nocterm_session::recording::PhysicalPrefix;
use nocterm_session::testing::recording::{ScriptedOutputLog, ScriptedOutputLogFactory};
use std::path::PathBuf;

#[gpui::test]
async fn append_unicode_chunks_drains_and_seals(cx: &mut TestAppContext) {
    let factory = Arc::new(ScriptedOutputLogFactory::default());
    let log = Arc::new(ScriptedOutputLog::new(PathBuf::from("/test.log")));
    factory.enqueue(
        nocterm_session::testing::Gate::open(),
        Ok(log.clone() as Arc<dyn OutputLog>),
    );

    let (entity, handle) = cx.update(|cx| {
        let mut handle_slot = None;
        let entity = cx.new(|cx| {
            let (persisted, handle) = AppendPersisted::new(
                factory.clone(),
                RecordingOptions {
                    directory: None,
                    max_file_mib: 10,
                },
                None,
                cx,
            );
            handle_slot = Some(handle);
            persisted
        });
        (entity, handle_slot.unwrap())
    });

    log.enqueue_append(
        nocterm_session::testing::Gate::open(),
        AppendOutcome::new(PhysicalPrefix::new(1, 4, 0, false, false), None),
    );
    log.enqueue_seal(
        nocterm_session::testing::Gate::open(),
        AppendOutcome::new(PhysicalPrefix::new(1, 4, 0, true, true), None),
    );

    cx.update(|cx| {
        handle.output("test".as_bytes(), cx);
        handle.stop(cx);
    });

    cx.run_until_parked();

    let status = cx.update(|cx| entity.read(cx).status.clone());
    assert_eq!(status.bytes, 4);
    assert!(status.finished);
    assert!(status.error.is_none());
}
