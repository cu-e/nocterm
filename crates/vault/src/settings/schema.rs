//! Stable public schema identity for the established settings format.
use super::*;
impl JsonSchema for VaultSettings {
    fn schema_name() -> std::borrow::Cow<'static, str> {
        "VaultSettings".into()
    }
    fn schema_id() -> std::borrow::Cow<'static, str> {
        "nocterm_vault_ui::settings::VaultSettings".into()
    }
    fn json_schema(generator: &mut schemars::SchemaGenerator) -> schemars::Schema {
        let default = Self::default();
        let mut schema = nocterm_settings::schema::SectionSchema::new(generator);
        schema.field(
            "auto_lock_minutes",
            &default.auto_lock_minutes,
            "Lock after this many minutes without vault use (1–1440).",
        );
        schema.range("auto_lock_minutes", 1, 1440);
        schema.field(
            "prompt_on_startup",
            &default.prompt_on_startup,
            "Prompt for the master password when Nocterm starts.",
        );
        schema.finish("Automatic locking of the portable encrypted vault.")
    }
}
