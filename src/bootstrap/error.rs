//! Why composing the application's services failed.
use nocterm_plugin_model::CatalogError;
use nocterm_settings_ui::SettingsCatalogError;

#[derive(Debug, thiserror::Error)]
pub(crate) enum BootstrapError {
    #[error("could not compose settings pages")]
    Settings(#[from] SettingsCatalogError),
    #[error("could not compose the plugin catalog")]
    Plugins(#[from] CatalogError),
}
