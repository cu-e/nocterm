//! Stable public schema identity for the established settings format.
use super::*;
impl JsonSchema for MonitorSettings {
    fn schema_name() -> std::borrow::Cow<'static, str> {
        "MonitorSettings".into()
    }
    fn schema_id() -> std::borrow::Cow<'static, str> {
        "nocterm_monitor_ui::settings::MonitorSettings".into()
    }
    fn json_schema(generator: &mut schemars::SchemaGenerator) -> schemars::Schema {
        let default = Self::default();
        let mut schema = nocterm_settings::schema::SectionSchema::new(generator);
        schema.field(
            "enabled",
            &default.enabled,
            "Show the active host's resources in the status bar.",
        );
        schema.field(
            "local",
            &default.local,
            "Watch this computer while no remote session is active.",
        );
        schema.field("interval_secs", &default.interval_secs, "Seconds between updates while the details are closed. Only what the\nstatus bar shows is collected then.");
        schema.range(
            "interval_secs",
            MONITOR_INTERVAL_RANGE.start(),
            MONITOR_INTERVAL_RANGE.end(),
        );
        schema.field(
            "detail_interval_secs",
            &default.detail_interval_secs,
            "Seconds between updates while the details are open.",
        );
        schema.range(
            "detail_interval_secs",
            MONITOR_DETAIL_INTERVAL_RANGE.start(),
            MONITOR_DETAIL_INTERVAL_RANGE.end(),
        );
        schema.field(
            "history_points",
            &default.history_points,
            "Samples the graphs in the details keep.",
        );
        schema.range(
            "history_points",
            MONITOR_HISTORY_RANGE.start(),
            MONITOR_HISTORY_RANGE.end(),
        );
        schema.field(
            "status_bar",
            &default.status_bar,
            "What the status bar shows, in this order.",
        );
        schema.field("details", &default.details, "What the details show.");
        schema.finish("The resources of the active host, shown in the status bar.")
    }
}
