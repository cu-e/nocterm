//! A partial root restore cannot hide a newer same-control background tail.
use super::*;
use nocterm_cloud_runner::DurableCurrentCloudHeads;

#[test]
fn partial_root_restore_cannot_hide_a_background_event_after_cold_key_retirement() {
    for restore_header_index in [false, true] {
        let root = private_root();
        let cipher = Arc::new(TestCipher::default());
        let store = journal(root.path(), cipher.clone())
            .with_background_events(Arc::new(TestEvents::default()), publication_budgets())
            .unwrap();
        let lease = claim(&store, None);
        let mut state = state(1, &lease);
        let mut position = JournalPosition::default();
        let job = dispatch(&store, &lease, &mut state, &mut position, 1);
        let before = current(&store);
        let saved = RootStateSnapshot::capture(root.path());
        block_on(store.retain(job)).unwrap();
        let after = current(&store);
        assert_eq!(before.control(), after.control());
        assert_eq!(before.events().advance(1).unwrap(), after.events());
        assert_eq!(
            block_on(store.head(stream())).unwrap().unwrap().lease(),
            &lease
        );
        drop(store); // Clear all native refusal marks: exercise actual persisted linkage.
        saved.restore_root_rows(root.path());
        if restore_header_index {
            saved.restore_header_index_without_its_ciphertext(root.path());
        }
        assert!(matches!(
            open_journal(
                root.path(),
                cipher.clone(),
                nocterm_cloud_runner::JournalLimits::default()
            ),
            Err(JournalError::Corrupt)
        ));
        cipher.retire_content_key();
        let cold = DurableCurrentCloudHeads::open_existing(
            root.path(),
            catalog_binding(),
            Arc::new(TestCipher::default()),
            Arc::new(TestCipher::default()),
            nocterm_cloud_runner::JournalLimits::default(),
        );
        match cold {
            Err(JournalError::Corrupt) => {}
            Ok(cold) => assert!(matches!(
                block_on(cold.chat(stream())),
                Err(JournalError::Corrupt)
            )),
            Err(error) => panic!(
                "partial root restore must not become an empty/current observation: {error:?}"
            ),
        }
    }
}
