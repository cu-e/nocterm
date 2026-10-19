use super::*;

#[test]
fn copied_native_paths_preserve_exact_spelling_and_requested_capacity() {
    for path in ["", ".", "a/../b", "/tmp//folder/", "名字"] {
        let path = Path::new(path);
        let owned = AdmittedPath::try_copy(path, ()).unwrap();
        assert_eq!(owned.as_path().as_os_str(), path.as_os_str());
        assert_eq!(owned.capacity(), AdmittedPath::<()>::copy_capacity(path));
    }
}

#[cfg(unix)]
#[test]
fn admitted_leaf_join_matches_original_native_path_push_for_relative_and_raw_spelling() {
    use std::os::unix::ffi::OsStrExt;
    let leaf = std::ffi::OsStr::new("{F2F8F6AC-EAE5-4DD7-B4BC-3713DA3ED73C}.json");
    for base in [
        b"".as_slice(),
        b"/",
        b".",
        b"relative",
        b"/tmp//",
        b"a/../b",
        b"//server/share",
        b"raw\xff",
    ] {
        let base = Path::new(std::ffi::OsStr::from_bytes(base));
        let owned = AdmittedPath::try_join_leaf(base, leaf, ()).unwrap();
        assert_eq!(owned.as_path().as_os_str(), base.join(leaf).as_os_str());
        assert_eq!(
            owned.capacity(),
            AdmittedPath::<()>::leaf_capacity(base, leaf).unwrap()
        );
    }
    for leaf in ["", ".", "..", "/absolute", "a/b"] {
        assert!(matches!(
            AdmittedPath::<()>::leaf_capacity(Path::new("base"), std::ffi::OsStr::new(leaf)),
            Err(PathAdmissionError::InvalidLeaf)
        ));
    }
}

#[cfg(windows)]
#[test]
fn supported_windows_conversion_matches_profile_and_round_trips_original_native_units() {
    use std::os::windows::ffi::OsStrExt;
    let cases = [
        vec![],
        vec![b'a' as u16; 100_000],
        vec![0x7ff; 19],
        vec![0x800; 19],
        vec![0xd800],
        vec![0xdc00],
        vec![0xd800, 0xdc00],
        vec![0xd800, 0xd800, 0xdc00, 0xdc00],
        vec![b'a' as u16, 0xd800, 0x800, 0xdc00, 0xd83d, 0xde00],
    ];
    for wide in cases {
        let plan = WidePathPlan::new(&wide).unwrap();
        let actual = plan.allocate(&wide, (), &()).unwrap();
        assert_eq!(actual.capacity(), plan.final_capacity());
        assert_eq!(
            actual
                .as_path()
                .as_os_str()
                .encode_wide()
                .collect::<Vec<_>>(),
            wide
        );
        assert!(plan.peak_bytes() >= actual.capacity());
    }
}

#[cfg(windows)]
#[test]
fn admitted_windows_leaf_join_preserves_drive_unc_and_verbatim_std_semantics() {
    let leaf = std::ffi::OsStr::new("{F2F8F6AC-EAE5-4DD7-B4BC-3713DA3ED73C}.json");
    for base in [
        "",
        ".",
        "C:",
        "C:\\",
        "C:\\folder\\",
        "\\server\\share",
        r"\\server\share\folder",
        r"\\?\C:\folder\.\..\unchanged",
        r"\\?\UNC\server\share\folder",
        "relative/with/slashes",
    ] {
        let base = Path::new(base);
        let owned = AdmittedPath::try_join_leaf(base, leaf, ()).unwrap();
        assert_eq!(owned.as_path().as_os_str(), base.join(leaf).as_os_str());
        assert_eq!(
            owned.capacity(),
            LeafPathPlan::new(base, leaf).unwrap().capacity()
        );
    }
}

#[test]
fn prepared_general_join_matches_native_push_without_changing_spelling() {
    let bases = [
        "",
        ".",
        "/",
        "base/../raw//",
        "C:",
        r"C:\folder",
        r"\\?\C:\folder\old",
        r"\\?\UNC\server\share\folder",
    ];
    let suffixes = [
        "",
        ".",
        "..",
        "../next",
        "/rooted",
        r"\rooted",
        "C:relative",
        r"C:\absolute",
        "a//b/../c",
    ];
    for base in bases {
        for suffix in suffixes {
            let base = Path::new(base);
            let suffix = Path::new(suffix);
            let prepared = PreparedPathJoin::new(base, suffix, ()).unwrap();
            let capacity = prepared.capacity().unwrap();
            let actual = prepared.allocate(()).unwrap();
            assert_eq!(
                actual.as_path().as_os_str(),
                base.join(suffix).as_os_str(),
                "{base:?} + {suffix:?}"
            );
            assert_eq!(actual.capacity(), capacity);
        }
    }
}

#[test]
fn prepared_join_fills_only_empty_exact_capacity_targets_without_implicit_growth() {
    let base = Path::new("relative/é");
    let suffix = Path::new("nested/../leaf");
    let prepared = PreparedPathJoin::new(base, suffix, ()).unwrap();
    let capacity = prepared.capacity().unwrap();
    let mut target = std::ffi::OsString::with_capacity(capacity);
    let original_capacity = target.capacity();
    prepared.fill(&mut target).unwrap();
    assert_eq!(target, base.join(suffix).as_os_str());
    assert_eq!(target.capacity(), original_capacity);
    let retained = target.clone();
    assert!(matches!(
        prepared.fill(&mut target),
        Err(PathAdmissionError::SourceMismatch)
    ));
    assert_eq!(target, retained);
    let mut undersized = std::ffi::OsString::new();
    assert!(matches!(
        prepared.fill(&mut undersized),
        Err(PathAdmissionError::SourceMismatch)
    ));
    assert!(undersized.is_empty());
    assert_eq!(undersized.capacity(), 0);
}
