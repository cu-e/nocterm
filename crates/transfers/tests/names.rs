use nocterm_transfers::*;
#[test]
fn hostile_leaf_names_are_rejected_on_every_platform() {
    for name in [
        "",
        ".",
        "..",
        "../escape",
        "a/b",
        "a\\b",
        "a\0b",
        "a:b",
        "a.",
        "a ",
        "CON",
        "nul.txt",
        "COM1",
        "LPT9.ext",
        "CONIN$",
    ] {
        assert!(!valid_download_name(name), "{name:?}");
    }
    assert!(valid_download_name("name (1).txt"));
    assert!(valid_download_name("файл.txt"));
    assert!(!valid_download_name(&"a".repeat(1025)));
}
#[test]
fn remote_source_traversal_is_rejected_without_normalizing_it() {
    for source in ["relative", "/a/../escape", "/a/./file", "/a\0file", "/"] {
        assert!(!valid_remote_source(source));
    }
    assert!(valid_remote_source("/a/file.txt"));
}
