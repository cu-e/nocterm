#![deny(rust_2018_idioms)]

use std::path::Path;

#[test]
fn test_override_temp_dir() {
    #[cfg(not(target_os = "wasi"))]
    assert_eq!(tempfile::env::temp_dir(), std::env::temp_dir());

    assert!(tempfile::env::with_temp_dir_override(|path| path.is_none()));

    let new_tmp = Path::new("/tmp/override");
    tempfile::env::override_temp_dir(new_tmp).unwrap();
    assert_eq!(tempfile::env::temp_dir(), new_tmp);
    tempfile::env::with_temp_dir_override(|path| assert_eq!(path, Some(new_tmp)));

    let new_tmp2 = Path::new("/tmp/override2");
    tempfile::env::override_temp_dir(new_tmp2).expect_err("override should only be possible once");
}
