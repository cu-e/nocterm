use super::*;

#[test]
fn repeated_family_metadata_cannot_multiply_past_the_retained_content_budget() {
    let variants: Vec<_> = (0..2_000)
        .map(|index| format!(r#"{{"name":"Theme {index}","appearance":"dark","style":{{}}}}"#))
        .collect();
    let bytes = format!(
        r#"{{"author":"{}","themes":[{}]}}"#,
        "a".repeat(32 * 1024),
        variants.join(",")
    );
    let path = PathBuf::from("many.json");
    let mut builder = CatalogBuilder::new();
    builder.document(&path, ThemeSource::User(path.clone()), bytes.as_bytes());
    let catalog = builder.finish();
    assert!(catalog.entries().len() < 1_024);
    assert!(
        catalog
            .problems()
            .iter()
            .any(|problem| problem.error.to_string().contains("retained content bytes"))
    );
}
#[test]
fn supplied_documents_reserve_names_and_keep_first_precedence() {
    let mut builder = CatalogBuilder::new();
    let document = br##"{"themes":[{"name":"First","appearance":"dark","style":{}},{"name":"Nocterm Default","appearance":"light","style":{}}]}"##;
    builder.document(
        Path::new("a.json"),
        ThemeSource::User("a.json".into()),
        document,
    );
    builder.document(
        Path::new("b.json"),
        ThemeSource::User("b.json".into()),
        document,
    );
    let catalog = builder.finish();
    assert_eq!(catalog.entries().len(), 1);
    assert_eq!(catalog.problems().len(), 3);
    assert!(
        matches!(&catalog.entries()[0].source,ThemeSource::User(path) if path == Path::new("a.json"))
    );
}
#[test]
fn empty_pack_metadata_is_retained_and_manifest_identity_checked() {
    let mut builder = CatalogBuilder::new();
    assert!(
        builder
            .manifest(
                Path::new("extension.toml"),
                "x",
                b"id='x'\nname='X'\nversion='1'"
            )
            .is_some()
    );
    assert!(
        builder
            .manifest(
                Path::new("wrong.toml"),
                "y",
                b"id='x'\nname='X'\nversion='1'"
            )
            .is_none()
    );
    let catalog = builder.finish();
    assert_eq!(catalog.packs().len(), 1);
    assert_eq!(catalog.problems().len(), 1);
}
