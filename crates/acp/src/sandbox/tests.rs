#[cfg(target_os = "linux")]
use super::*;
mod native;

#[cfg(target_os = "linux")]
#[test]
fn availability_finds_only_a_bubblewrap_file_on_path() {
    let temp = tempfile::tempdir().unwrap();
    let root = std::fs::canonicalize(temp.path()).unwrap();
    let env = BTreeMap::from([("PATH".into(), root.to_string_lossy().into_owned())]);
    assert_eq!(availability(&env, Some(&root)), Availability::Missing);
    std::fs::create_dir(root.join("bwrap")).unwrap();
    assert_eq!(availability(&env, Some(&root)), Availability::Missing);
    std::fs::remove_dir(root.join("bwrap")).unwrap();
    std::fs::write(root.join("bwrap"), "#!/bin/sh\nexit 0\n").unwrap();
    assert_eq!(
        availability(&env, Some(&root)),
        Availability::Available(root.join("bwrap"))
    );
}
