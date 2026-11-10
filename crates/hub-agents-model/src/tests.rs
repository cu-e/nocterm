use nocterm_session::Secret;

use crate::{
    AgentDraft, AgentKey, DraftError, ProviderDraft, ProviderId, ProviderKind, ServerStatus,
};

fn provider() -> Option<ProviderId> {
    Some(ProviderId::new("p1").unwrap())
}

#[test]
fn an_agent_needs_a_name_a_provider_and_a_model() {
    let draft = |name: &str, provider, models: &[&str]| AgentDraft {
        name: name.into(),
        provider,
        models: models.iter().map(|m| (*m).to_owned()).collect(),
        ..AgentDraft::default()
    };
    assert_eq!(
        draft(" ", provider(), &["m"]).validate(),
        Err(DraftError::NameMissing)
    );
    assert_eq!(
        draft("ops", None, &["m"]).validate(),
        Err(DraftError::ProviderMissing)
    );
    assert_eq!(
        draft("ops", provider(), &[" "]).validate(),
        Err(DraftError::ModelMissing)
    );
    let valid = draft(" ops ", provider(), &["a", "b", "a"])
        .validate()
        .unwrap();
    assert_eq!(valid.name, "ops");
    assert_eq!(valid.model, "a");
    assert_eq!(valid.models, ["a", "b"]);
}

#[test]
fn the_default_model_is_always_one_of_the_models() {
    let valid = AgentDraft {
        name: "ops".into(),
        provider: provider(),
        model: "big".into(),
        models: vec!["small".into()],
        ..AgentDraft::default()
    }
    .validate()
    .unwrap();
    assert_eq!(valid.model, "big");
    assert_eq!(valid.models, ["big", "small"]);
}

#[test]
fn a_provider_needs_a_web_address_and_keeps_its_key_out_of_debug() {
    let draft = ProviderDraft {
        kind: ProviderKind::Anthropic,
        name: "Mine".into(),
        base_url: "ftp://x".into(),
        models: vec![],
        api_key: Some(Secret::new("sk-secret")),
    };
    assert_eq!(draft.clone().validate(), Err(DraftError::BaseUrl));
    let valid = ProviderDraft {
        base_url: "https://api.anthropic.com/".into(),
        ..draft
    }
    .validate()
    .unwrap();
    assert_eq!(valid.base_url, "https://api.anthropic.com");
    assert!(!format!("{valid:?}").contains("sk-secret"));
}

#[test]
fn an_empty_key_on_a_change_keeps_the_stored_one() {
    let valid = ProviderDraft {
        name: "Mine".into(),
        base_url: "https://x".into(),
        api_key: Some(Secret::new("  ")),
        ..ProviderDraft::default()
    }
    .validate()
    .unwrap();
    assert!(valid.api_key.is_none());
}

#[test]
fn the_agent_key_line_is_recognized_with_any_options_or_comment() {
    let key = AgentKey {
        public_key: "ssh-ed25519 AAAAblob nocterm".into(),
        authorized_line: "no-pty ssh-ed25519 AAAAblob nocterm".into(),
        fingerprint: "SHA256:x".into(),
    };
    assert!(key.is_line_of("restrict ssh-ed25519 AAAAblob other"));
    assert!(!key.is_line_of("ssh-ed25519 AAAAother nocterm"));
    assert!(!key.is_line_of(""));
}

#[test]
fn only_a_ready_server_is_usable() {
    assert!(ServerStatus::Ready.is_ready());
    assert!(!ServerStatus::KeyRotated.is_ready());
    assert_eq!(ServerStatus::Ready.label(), "Agents ✓");
}
