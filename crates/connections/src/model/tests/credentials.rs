use super::*;

#[gpui_kit::test]
async fn associating_unmatched_credentials_preserves_malformed_profile_bytes(
    cx: &mut gpui_kit::TestAppContext,
) {
    let directory = tempfile::tempdir().unwrap();
    let paths = Paths::rooted_at(directory.path());
    fs::create_dir_all(paths.config_dir()).unwrap();
    let damaged = b"# preserve this user document\ninvalid [[";
    fs::write(paths.connections_file(), damaged).unwrap();
    let connections = cx.new(|_| Connections::load(&paths));
    assert!(connections.read_with(cx, |c, _| c.load_error().is_some()));
    assert!(connections.read_with(cx, |c, cx| c.profiles(cx).is_empty()));
    let result = connections
        .update(cx, |c, cx| {
            c.associate_credential(
                &Target::new("unmatched", "host.test", 22),
                &Auth::Password,
                CredentialId::from_bytes([8; 16]),
                cx,
            )
        })
        .await;
    assert_eq!(fs::read(paths.connections_file()).unwrap(), damaged);
    assert!(matches!(result, Err(ConnectionsError::ReadOnly(_))));
}

#[gpui_kit::test]
async fn stale_readonly_load_cannot_overwrite_profiles_repaired_by_another_writer(
    cx: &mut gpui_kit::TestAppContext,
) {
    let directory = tempfile::tempdir().unwrap();
    let paths = Paths::rooted_at(directory.path());
    fs::create_dir_all(paths.config_dir()).unwrap();
    fs::write(paths.connections_file(), "invalid [[").unwrap();
    let connections = cx.new(|_| Connections::load(&paths));
    assert!(connections.read_with(cx, |c, _| c.load_error().is_some()));
    let repaired = profile("Repaired externally");
    persist::save(
        &paths.connections_file(),
        &documents(std::slice::from_ref(&repaired)),
    )
    .unwrap();
    let preserved = fs::read(paths.connections_file()).unwrap();
    let result = connections
        .update(cx, |c, cx| {
            c.associate_credential(
                &repaired.target,
                &repaired.auth,
                CredentialId::from_bytes([9; 16]),
                cx,
            )
        })
        .await;
    assert_eq!(fs::read(paths.connections_file()).unwrap(), preserved);
    assert!(matches!(result, Err(ConnectionsError::ReadOnly(_))));
    let reloaded = cx.new(|_| Connections::load(&paths));
    assert_eq!(
        reloaded.read_with(cx, |c, cx| c.profiles(cx).get(repaired.id).cloned()),
        Some(repaired)
    );
}

#[gpui_kit::test]
async fn capturing_readonly_credential_association_preserves_profiles_and_recent_credentials(
    cx: &mut gpui_kit::TestAppContext,
) {
    for externally_repaired in [false, true] {
        let directory = tempfile::tempdir().unwrap();
        let paths = Paths::rooted_at(directory.path());
        fs::create_dir_all(paths.config_dir()).unwrap();
        fs::write(paths.connections_file(), "# readonly user data\ninvalid [[").unwrap();
        let mut original = profile("Externally repaired");
        original.credential = Some(CredentialId::from_bytes([4; 16]));
        let recents = shutdown::recent_profiles(std::slice::from_ref(&original));
        persist::save(&paths.recents_file(), &recents).unwrap();
        let coordinator = ShutdownCoordinator::default();
        let connections = cx.update(|cx| {
            Connections::load(&paths)
                .into_entity(&coordinator, cx)
                .unwrap()
        });
        assert!(connections.read_with(cx, |c, _| c.load_error().is_some()));
        if externally_repaired {
            persist::save(
                &paths.connections_file(),
                &documents(std::slice::from_ref(&original)),
            )
            .unwrap();
        }
        let profiles_before = fs::read(paths.connections_file()).unwrap();
        let recents_before = fs::read(paths.recents_file()).unwrap();
        let receipt = connections.update(cx, |c, cx| {
            c.associate_credential(
                &original.target,
                &original.auth,
                CredentialId::from_bytes([5; 16]),
                cx,
            )
        });
        let captured = cx.update(|cx| coordinator.capture_all(cx));
        drop(connections);
        let report = cx.background_executor.spawn(captured).await;
        assert!(report.failures().is_empty());
        assert_eq!(fs::read(paths.recents_file()).unwrap(), recents_before);
        assert_eq!(fs::read(paths.connections_file()).unwrap(), profiles_before);
        assert!(matches!(receipt.await, Err(ConnectionsError::ReadOnly(_))));
        let saved: Recents = persist::load(&paths.recents_file()).unwrap().unwrap();
        assert_eq!(saved.iter().next().unwrap().credential, original.credential);
    }
}
