//! Memory fixtures over supplied actual service and identity capabilities.
use gpui_kit::App;
use nocterm_snippets::test_support::SequenceIds;
use std::sync::Arc;
pub(crate) fn install(
    settings: nocterm_store::SettingsHandle,
    shutdown: &nocterm_store::ShutdownCoordinator,
    cx: &mut App,
) -> Result<crate::Installed, nocterm_store::StoreError> {
    install_with_ids(settings, Arc::new(SequenceIds::new(10_000)), shutdown, cx)
}
pub(crate) fn install_with_ids(
    settings: nocterm_store::SettingsHandle,
    ids: Arc<dyn nocterm_snippets::SnippetIds>,
    shutdown: &nocterm_store::ShutdownCoordinator,
    cx: &mut App,
) -> Result<crate::Installed, nocterm_store::StoreError> {
    let model = nocterm_snippets_store::SnippetsHandle::install(
        nocterm_snippets_store::PreparedLibrary::memory(),
        Arc::new(|_| Box::pin(async { Ok(()) })),
        shutdown,
        cx,
    )?;
    Ok(crate::init(model, settings, ids, cx))
}
