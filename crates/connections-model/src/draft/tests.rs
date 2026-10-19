use super::*;
use std::error::Error as _;

fn draft() -> ProfileDraft {
    ProfileDraft {
        endpoint: EndpointDraft {
            host: " example.test ".into(),
            ..Default::default()
        },
        ..Default::default()
    }
}
fn defaults() -> EditorDefaults {
    EditorDefaults {
        user: Some("tester".into()),
        home: Some(PathBuf::from("/supplied/home")),
    }
}
#[test]
fn supplied_defaults_expand_private_key_without_native_discovery() {
    let mut draft = draft();
    draft.authentication.kind = AuthKind::Key;
    draft.authentication.key_path = " ~/.ssh/key ".into();
    let profile = draft.validate(&defaults()).unwrap();
    assert_eq!(
        profile.target,
        Target::new("tester", "example.test", DEFAULT_PORT)
    );
    assert_eq!(
        profile.auth,
        Auth::Key {
            path: PathBuf::from("/supplied/home/.ssh/key")
        }
    );
    assert_eq!(
        draft
            .validate(&EditorDefaults::default())
            .unwrap_err()
            .field(),
        ValidationField::User
    );
}
#[test]
fn endpoint_authentication_and_launch_validation_keep_their_order() {
    let mut draft = draft();
    draft.endpoint.host.clear();
    draft.endpoint.port = "invalid".into();
    draft.authentication.kind = AuthKind::Key;
    draft.authentication.credential = "invalid".into();
    draft.launch.override_launch = true;
    draft.launch.args = "invalid".into();
    assert!(matches!(
        draft.validate(&defaults()),
        Err(ProfileValidationError::MissingHost)
    ));
    draft.endpoint.host = "host".into();
    assert_eq!(
        draft.validate(&defaults()).unwrap_err().field(),
        ValidationField::Port
    );
    draft.endpoint.port.clear();
    assert_eq!(
        draft.validate(&defaults()).unwrap_err().field(),
        ValidationField::Key
    );
    draft.authentication.key_path = "/supplied/key".into();
    assert!(matches!(
        draft.validate(&defaults()),
        Err(ProfileValidationError::Credential(
            CredentialIdParseError::InvalidShape
        ))
    ));
    draft.authentication.credential.clear();
    assert_eq!(
        draft.validate(&defaults()).unwrap_err().field(),
        ValidationField::Arguments
    );
}
#[test]
fn json_failures_preserve_native_parse_causes_and_exact_messages() {
    let mut draft = draft();
    draft.launch.override_launch = true;
    draft.launch.args = "not json".into();
    let error = draft.validate(&defaults()).unwrap_err();
    assert_eq!(
        error.to_string(),
        "Arguments must be a JSON array of strings."
    );
    assert!(
        error
            .source()
            .unwrap()
            .downcast_ref::<serde_json::Error>()
            .is_some()
    );
    draft.launch.args = "[]".into();
    draft.launch.env = r#"{"LANG":false}"#.into();
    let error = draft.validate(&defaults()).unwrap_err();
    assert_eq!(
        error.to_string(),
        "Environment must be a JSON object of string values."
    );
    assert!(
        error
            .source()
            .unwrap()
            .downcast_ref::<serde_json::Error>()
            .is_some()
    );
}
#[test]
fn launch_and_session_errors_retain_typed_variants() {
    let mut draft = draft();
    draft.launch.override_launch = true;
    draft.launch.env = r#"{"INVALID;KEY":"value"}"#.into();
    assert!(matches!(
        draft.validate(&defaults()),
        Err(ProfileValidationError::Launch(
            LaunchValidationError::InvalidEnvironmentName
        ))
    ));
    draft.launch.env.clear();
    draft.options.term = Some("invalid TERM".into());
    assert!(matches!(
        draft.validate(&defaults()),
        Err(ProfileValidationError::Session(_))
    ));
}
#[test]
fn appearance_precedes_description_and_description_limit_counts_bytes() {
    let mut draft = draft();
    draft.metadata.description = "é".repeat(8193);
    draft.appearance.icon_color = "orange".into();
    draft.appearance.country = "invalid".into();
    assert_eq!(
        draft.validate(&defaults()).unwrap_err().field(),
        ValidationField::IconColor
    );
    draft.appearance.icon_color = " abc ".into();
    assert_eq!(
        draft.validate(&defaults()).unwrap_err().field(),
        ValidationField::Country
    );
    draft.appearance.country = " DE ".into();
    assert!(matches!(
        draft.validate(&defaults()),
        Err(ProfileValidationError::Description)
    ));
    draft.metadata.description = "é".repeat(8192);
    let profile = draft.validate(&defaults()).unwrap();
    assert_eq!(profile.icon_color.as_deref(), Some("#ABC"));
    assert_eq!(profile.country.as_deref(), Some("de"));
    assert_eq!(profile.description.len(), 16384);
}
#[test]
fn disabled_launch_ignores_hidden_json_but_preserves_starting_directory() {
    let mut draft = draft();
    draft.launch.args = "invalid json".into();
    draft.launch.env = "invalid json".into();
    assert!(draft.validate(&defaults()).unwrap().launch.is_none());
    draft.launch.cwd = " /work tree ".into();
    let launch = draft.validate(&defaults()).unwrap().launch.unwrap();
    assert_eq!(launch.cwd.as_deref(), Some("/work tree"));
    assert!(launch.program.is_none() && launch.args.is_empty() && launch.env.is_empty());
    assert!(launch.integration);
}
