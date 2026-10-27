//! Every contribution of the application's plugin catalog reaches the window it composes. The
//! sessions of the hubs are registered only when the application has hub access, which this
//! window does not.
use super::fixture;
use gpui_kit::TestAppContext;

#[gpui_kit::test]
fn every_contribution_of_the_catalog_is_registered_in_a_composed_window(cx: &mut TestAppContext) {
    let (_, workspace, _, _) = fixture(cx);
    let catalog = crate::shell_setup::plugin_catalog().expect("the application catalog is valid");
    let shell: Vec<_> = nocterm_workspace::plugin::CORE
        .iter()
        .map(|contribution| &contribution.id)
        .collect();
    let without_hubs: Vec<_> = nocterm_hubs_ui::plugin::PLUGIN
        .contributions
        .iter()
        .map(|contribution| &contribution.id)
        .collect();

    assert!(
        catalog.entries().count() > shell.len(),
        "the catalog holds contributions beyond the shell's own"
    );
    cx.update(|cx| {
        let workspace = workspace.read(cx);
        let unregistered: Vec<_> = catalog
            .entries()
            .filter(|contribution| !shell.contains(&&contribution.id))
            .filter(|contribution| !without_hubs.contains(&&contribution.id))
            .filter(|contribution| !workspace.is_registered(&contribution.id))
            .map(|contribution| contribution.id.to_string())
            .collect();

        assert!(
            unregistered.is_empty(),
            "contributions the window never registered: {unregistered:?}"
        );

        let registered_locally: Vec<_> = without_hubs
            .iter()
            .filter(|id| workspace.is_registered(id))
            .collect();
        assert!(
            registered_locally.is_empty(),
            "contributions of the hubs a window without hub access registered: {registered_locally:?}"
        );
    });
}
