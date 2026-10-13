//! Configuration transitions preserve per-chat choices and reject obsolete replies.
use super::*;
use nocterm_ai::session::SessionState;

fn option(id: &str, current: &str, values: &[&str]) -> acp::SessionConfigOption {
    acp::SessionConfigOption::select(
        id.to_owned(),
        id.to_owned(),
        current.to_owned(),
        values
            .iter()
            .map(|value| acp::SessionConfigSelectOption::new(value.to_string(), value.to_string()))
            .collect::<Vec<_>>(),
    )
}
fn opening() -> (SessionState, Ticket) {
    let mut session = SessionState::default();
    assert!(session.queue());
    session.leased();
    let ticket = session.ticket();
    (session, ticket)
}

#[test]
fn per_chat_choices_override_defaults_and_rejected_choice_is_attempted_once() {
    let (_, ticket) = opening();
    let mut config = ChatConfig::default();
    config.restore(
        Choices::from([("model".into(), ConfigValue::Value("saved".into()))]),
        None,
        None,
    );
    let reported = vec![option("model", "default", &["default", "saved"])];
    config.preview(
        Choices::from([("model".into(), ConfigValue::Value("default".into()))]),
        &session_config::store(&reported),
    );
    assert_eq!(
        session_config::current(config.options())["model"],
        ConfigValue::Value("saved".into())
    );
    config.opening_started(ticket);
    config.start_open(ticket, None, reported);
    assert!(
        matches!(config.next_request(ticket),ConfigStep::SetOption(_,ConfigValue::Value(value)) if value=="saved")
    );
    assert!(
        config
            .accept_option_result(ticket, Err(AgentError::Io("rejected".into())))
            .unwrap()
            .is_err()
    );
    assert!(matches!(config.next_request(ticket), ConfigStep::Ready));
    assert!(config.sync_ready(SessionPhase::Ready));
    assert_eq!(
        config.choices()["model"],
        ConfigValue::Value("default".into())
    );
}

#[test]
fn option_responses_reevaluate_dependent_choices_one_request_at_a_time() {
    let (_, ticket) = opening();
    let mut config = ChatConfig::default();
    config.restore(
        Choices::from([
            ("model".into(), ConfigValue::Value("b".into())),
            ("effort".into(), ConfigValue::Value("high".into())),
        ]),
        None,
        None,
    );
    config.opening_started(ticket);
    config.start_open(
        ticket,
        None,
        vec![
            option("model", "a", &["a", "b"]),
            option("effort", "low", &["low"]),
        ],
    );
    assert!(
        matches!(config.next_request(ticket),ConfigStep::SetOption(id,_) if id.to_string()=="model")
    );
    config
        .accept_option_result(
            ticket,
            Ok(vec![
                option("model", "b", &["a", "b"]),
                option("effort", "low", &["low", "high"]),
            ]),
        )
        .unwrap()
        .unwrap();
    assert!(
        matches!(config.next_request(ticket),ConfigStep::SetOption(id,_) if id.to_string()=="effort")
    );
}

#[test]
fn old_ticket_completion_cannot_overwrite_or_consume_new_reconciliation() {
    let (mut session, old) = opening();
    let mut config = ChatConfig::default();
    config.restore(
        Choices::from([("model".into(), ConfigValue::Value("saved".into()))]),
        None,
        None,
    );
    config.opening_started(old);
    session.detach();
    assert!(session.queue());
    session.leased();
    let current = session.ticket();
    config.opening_started(current);
    config.start_open(
        current,
        None,
        vec![option("model", "default", &["default", "saved"])],
    );
    assert!(
        config
            .accept_option_result(old, Ok(vec![option("model", "late", &["late"])]))
            .is_none()
    );
    assert!(
        matches!(config.next_request(current),ConfigStep::SetOption(_,ConfigValue::Value(value)) if value=="saved")
    );
}

#[test]
fn pending_controls_keep_latest_kind_for_exact_opened_session() {
    let (_, ticket) = opening();
    let mut config = ChatConfig::default();
    config.opening_started(ticket);
    for (id, name) in [("other", "foreign"), ("mine", "old"), ("mine", "latest")] {
        config.keep_control(
            ticket,
            &acp::SessionNotification::new(
                id,
                acp::SessionUpdate::AvailableCommandsUpdate(acp::AvailableCommandsUpdate::new(
                    vec![acp::AvailableCommand::new(name, name)],
                )),
            ),
        );
    }
    config.start_open(ticket, None, Vec::new());
    config.begin_reconciliation(&acp::SessionId::new("mine"), ticket);
    assert_eq!(config.commands()[0].name, "latest");
    assert_eq!(config.commands_revision(), 1);
}

#[test]
fn mode_category_suppresses_separate_mode_request_and_commands_invalidate_revision() {
    let (_, ticket) = opening();
    let mut config = ChatConfig::default();
    config.restore(Choices::new(), Some(acp::SessionModeId::new("code")), None);
    config.opening_started(ticket);
    let modes=serde_json::from_value(serde_json::json!({"currentModeId":"ask","availableModes":[{"id":"ask","name":"Ask"},{"id":"code","name":"Code"}]})).unwrap();
    config.start_open(
        ticket,
        Some(modes),
        vec![
            option("mode", "ask", &["ask", "code"])
                .category(acp::SessionConfigOptionCategory::Mode),
        ],
    );
    assert!(config.config_overrides_modes());
    assert!(matches!(config.next_request(ticket), ConfigStep::Ready));
    let update =
        acp::SessionUpdate::AvailableCommandsUpdate(acp::AvailableCommandsUpdate::new(Vec::new()));
    assert!(config.report_control(&update));
    assert!(config.report_control(&update));
    assert_eq!(config.commands_revision(), 2);
}

#[test]
fn opening_reports_preserve_desired_values_until_ready_and_restart_keeps_preview() {
    let (_, ticket) = opening();
    let mut config = ChatConfig::default();
    config.restore(
        Choices::from([("model".into(), ConfigValue::Value("saved".into()))]),
        None,
        Some("last model".into()),
    );
    config.opening_started(ticket);
    config.start_open(
        ticket,
        None,
        vec![option("model", "default", &["default", "saved"])],
    );
    assert!(!config.sync_ready(SessionPhase::Opening));
    assert_eq!(
        config.choices()["model"],
        ConfigValue::Value("saved".into())
    );
    let mut restored = ChatConfig::default();
    restored.hydrate_restart(config.restart_snapshot());
    assert_eq!(restored.model().as_deref(), Some("last model"));
    assert_eq!(restored.options().len(), 1);
    assert!(matches!(restored.next_request(ticket), ConfigStep::Stale));
}

#[test]
fn pre_open_controls_evict_oldest_sessions_at_the_bound() {
    let (_, ticket) = opening();
    let mut config = ChatConfig::default();
    config.opening_started(ticket);
    for index in 0..65 {
        config.keep_control(
            ticket,
            &acp::SessionNotification::new(
                format!("session-{index}"),
                acp::SessionUpdate::AvailableCommandsUpdate(acp::AvailableCommandsUpdate::new(
                    vec![acp::AvailableCommand::new("latest", "Latest")],
                )),
            ),
        );
    }
    config.start_open(ticket, None, Vec::new());
    config.begin_reconciliation(&acp::SessionId::new("session-0"), ticket);
    assert!(config.commands().is_empty());
    assert_eq!(config.commands_revision(), 0);
}

#[test]
fn accepted_prompt_records_model_category_label_before_later_reports() {
    let mut config = ChatConfig::default();
    config.restore(Choices::new(), None, Some("saved model".into()));
    assert_eq!(config.model().as_deref(), Some("saved model"));
    config.report_control(&acp::SessionUpdate::ConfigOptionUpdate(
        acp::ConfigOptionUpdate::new(vec![
            option("effort", "high", &["high"])
                .category(acp::SessionConfigOptionCategory::ThoughtLevel),
            option("model", "a", &["a"]).category(acp::SessionConfigOptionCategory::Model),
        ]),
    ));
    config.prompt_started();
    assert_eq!(config.model().as_deref(), Some("a"));
    config.report_control(&acp::SessionUpdate::ConfigOptionUpdate(
        acp::ConfigOptionUpdate::new(Vec::new()),
    ));
    assert_eq!(config.model().as_deref(), Some("a"));
}
