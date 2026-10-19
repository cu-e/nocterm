//! Configuration transitions preserve per-chat choices and reject obsolete replies.
use super::*;
use crate::thread::controls::Controls;
use nocterm_ai::session::SessionState;

fn option(id: &str, current: &str, values: &[&str]) -> ConfigOption {
    use nocterm_ai::model::{
        controls::{ConfigChoice, ConfigKind, SelectChoices},
        ids::ConfigChoiceId,
    };
    ConfigOption {
        id: ConfigId::new(id),
        name: id.into(),
        description: None,
        category: None,
        kind: ConfigKind::Select {
            current: ConfigChoiceId::new(current),
            choices: SelectChoices::Flat(
                values
                    .iter()
                    .map(|value| ConfigChoice {
                        id: ConfigChoiceId::new(*value),
                        name: (*value).into(),
                        description: None,
                        extensions: None,
                    })
                    .collect(),
            ),
        },
        extensions: None,
    }
}
fn category(mut option: ConfigOption, category: ConfigCategory) -> ConfigOption {
    option.category = Some(category);
    option
}
fn controls(options: Vec<ConfigOption>, modes: Option<Modes>) -> Controls {
    Controls {
        current_mode: modes.as_ref().map(|modes| modes.current.clone()),
        options,
        modes,
    }
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
        session_config::current(config.preview_options())["model"],
        ConfigValue::Value("saved".into())
    );
    config.opening_started(ticket);
    let advertised = controls(reported, None);
    assert!(
        matches!(config.next_request(ticket, &advertised), ConfigStep::SetOption(_, ConfigValue::Value(value)) if value == "saved")
    );
    // The agent rejected it: the same controls come back and the choice is not retried.
    assert!(matches!(
        config.next_request(ticket, &advertised),
        ConfigStep::Ready
    ));
    assert!(config.sync_ready(SessionPhase::Ready, &advertised));
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
    let first = controls(
        vec![
            option("model", "a", &["a", "b"]),
            option("effort", "low", &["low"]),
        ],
        None,
    );
    assert!(
        matches!(config.next_request(ticket, &first), ConfigStep::SetOption(id, _) if id.to_string() == "model")
    );
    let second = controls(
        vec![
            option("model", "b", &["a", "b"]),
            option("effort", "low", &["low", "high"]),
        ],
        None,
    );
    assert!(
        matches!(config.next_request(ticket, &second), ConfigStep::SetOption(id, _) if id.to_string() == "effort")
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
    let advertised = controls(
        vec![option("model", "default", &["default", "saved"])],
        None,
    );
    assert!(!config.current(old));
    assert!(matches!(
        config.next_request(old, &advertised),
        ConfigStep::Stale
    ));
    assert!(
        matches!(config.next_request(current, &advertised), ConfigStep::SetOption(_, ConfigValue::Value(value)) if value == "saved")
    );
}

#[test]
fn mode_category_suppresses_separate_mode_request() {
    let (_, ticket) = opening();
    let mut config = ChatConfig::default();
    config.restore(Choices::new(), Some(ModeId::new("code")), None);
    config.opening_started(ticket);
    let modes = Modes {
        current: ModeId::new("ask"),
        choices: ["ask", "code"]
            .into_iter()
            .map(|id| nocterm_ai::model::controls::ModeChoice {
                id: ModeId::new(id),
                name: id.into(),
                description: None,
                extensions: None,
            })
            .collect(),
        extensions: None,
    };
    let advertised = controls(
        vec![category(
            option("mode", "ask", &["ask", "code"]),
            ConfigCategory::Mode,
        )],
        Some(modes.clone()),
    );
    assert!(config.overrides_modes(&advertised));
    assert!(matches!(
        config.next_request(ticket, &advertised),
        ConfigStep::Ready
    ));
    let plain = controls(Vec::new(), Some(modes));
    assert!(
        matches!(config.next_request(ticket, &plain), ConfigStep::SetMode(mode) if mode == ModeId::new("code"))
    );
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
    let reported = vec![option("model", "default", &["default", "saved"])];
    config.preview(Choices::new(), &session_config::store(&reported));
    config.opening_started(ticket);
    let advertised = controls(reported, None);
    assert!(!config.sync_ready(SessionPhase::Opening, &advertised));
    assert_eq!(
        config.choices()["model"],
        ConfigValue::Value("saved".into())
    );
    let mut restored = ChatConfig::default();
    restored.hydrate_restart(config.restart_snapshot());
    assert_eq!(restored.model().as_deref(), Some("last model"));
    assert_eq!(restored.preview_options().len(), 1);
    assert!(matches!(
        restored.next_request(ticket, &advertised),
        ConfigStep::Stale
    ));
}

#[test]
fn accepted_prompt_records_model_category_label_before_later_reports() {
    let mut config = ChatConfig::default();
    config.restore(Choices::new(), None, Some("saved model".into()));
    assert_eq!(config.model().as_deref(), Some("saved model"));
    let reported = controls(
        vec![
            category(
                option("effort", "high", &["high"]),
                ConfigCategory::ThoughtLevel,
            ),
            category(option("model", "a", &["a"]), ConfigCategory::Model),
        ],
        None,
    );
    config.prompt_started(&reported);
    assert_eq!(config.model().as_deref(), Some("a"));
    // Later reports never reach the recorded label; only an accepted prompt records it.
    assert_eq!(config.model().as_deref(), Some("a"));
}

/// Certified command notices from one real source lease of a fresh chat.
struct Notices {
    _state: nocterm_ai::thread::ThreadState,
    _lease: nocterm_ai::history::source::SourceLease,
    context: nocterm_ai::history::source::SourceContext,
}
impl Notices {
    fn new() -> Self {
        let authority = nocterm_ai::model::retention::RetentionAuthority::new(
            nocterm_ai::history::MAX_RESIDENT_BYTES,
        )
        .unwrap();
        let state = nocterm_ai::thread::ThreadState::new(
            &authority,
            "agent",
            "00000000-0000-4000-8000-000000000001",
            1,
        )
        .unwrap();
        let lease = state.source().capture().unwrap();
        let context = lease.context().unwrap();
        Self {
            _state: state,
            _lease: lease,
            context,
        }
    }
    fn commands(&self, session: &str, name: &str) -> nocterm_ai::history::source::ManagedNotice {
        use nocterm_ai::model::notice::{AdvertisedCommand, SessionNotice, SessionUpdate};
        let event = nocterm_ai::AgentEvent::Session(SessionNotice {
            session: SessionId::new(session),
            update: SessionUpdate::Commands(vec![AdvertisedCommand::new(name, name)]),
        });
        match nocterm_ai::testing::managed_event(&self.context, event) {
            Ok(nocterm_ai::connection::ManagedAgentEvent::Session(notice)) => notice,
            _ => panic!("fixture notice is certified"),
        }
    }
}
fn command_names(notices: &[nocterm_ai::history::source::ManagedNotice]) -> Vec<String> {
    use nocterm_ai::model::notice::SessionUpdate;
    notices
        .iter()
        .flat_map(|notice| match &notice.notice().update {
            SessionUpdate::Commands(commands) => commands
                .iter()
                .map(|command| command.name.clone())
                .collect(),
            _ => Vec::new(),
        })
        .collect()
}

#[test]
fn pending_controls_keep_latest_kind_for_exact_opened_session() {
    let notices = Notices::new();
    let (_, ticket) = opening();
    let mut config = ChatConfig::default();
    config.opening_started(ticket);
    for (session, name) in [("other", "foreign"), ("mine", "old"), ("mine", "latest")] {
        config.keep_control(ticket, notices.commands(session, name));
    }
    let held = config.take_pending(&SessionId::new("mine"));
    assert_eq!(command_names(&held), ["latest"]);
    assert!(config.take_pending(&SessionId::new("other")).is_empty());
}

#[test]
fn pre_open_controls_evict_oldest_sessions_at_the_bound() {
    let notices = Notices::new();
    let (_, ticket) = opening();
    let mut config = ChatConfig::default();
    config.opening_started(ticket);
    for index in 0..65 {
        config.keep_control(
            ticket,
            notices.commands(&format!("session-{index}"), "latest"),
        );
    }
    assert!(config.take_pending(&SessionId::new("session-0")).is_empty());
    config.opening_started(ticket);
    for index in 0..65 {
        config.keep_control(
            ticket,
            notices.commands(&format!("session-{index}"), "latest"),
        );
    }
    assert_eq!(
        command_names(&config.take_pending(&SessionId::new("session-64"))),
        ["latest"]
    );
}
