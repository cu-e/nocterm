use super::*;

use nocterm_ai::model::{
    auth::PermissionChoice,
    ids::ToolId,
    tools::{ToolChanges, ToolPatch},
};
fn choice(id: &str, label: &str, effect: PermissionEffect) -> PermissionChoice {
    PermissionChoice {
        id: PermissionOptionId::new(id),
        label: label.into(),
        effect,
        extensions: None,
    }
}
fn request(choices: Vec<PermissionChoice>) -> ProviderPermission {
    ProviderPermission {
        session: SessionId::new("session"),
        tool: ToolPatch {
            id: ToolId::new("tool"),
            changes: ToolChanges::default(),
            extensions: None,
            claims: Default::default(),
            literal: Default::default(),
        },
        choices,
        extensions: None,
    }
}

#[test]
fn automatic_choice_uses_kind_regardless_of_order_or_label() {
    let mut options = vec![
        choice("deny", "Allow once", PermissionEffect::DenyOnce),
        choice("persistent", "Allow", PermissionEffect::AllowRemembered),
        choice("once", "Reject", PermissionEffect::AllowOnce),
    ];
    for _ in 0..options.len() {
        assert_eq!(
            allow_once(&request(options.clone())),
            Some(PermissionOptionId::new("once"))
        );
        options.rotate_left(1);
    }
}

#[test]
fn persistent_or_rejection_options_never_qualify_for_automatic_approval() {
    for kind in [
        PermissionEffect::AllowRemembered,
        PermissionEffect::DenyOnce,
        PermissionEffect::DenyRemembered,
    ] {
        assert_eq!(
            allow_once(&request(vec![choice("id", "Allow once", kind)])),
            None
        );
    }
    assert_eq!(allow_once(&request(Vec::new())), None);
}

#[test]
fn missing_or_ambiguous_option_ids_cannot_be_selected() {
    let empty = request(vec![choice(" ", "Allow", PermissionEffect::AllowOnce)]);
    assert_eq!(allow_once(&empty), None);
    assert!(!unique_option(&empty, &PermissionOptionId::new(" ")));
    let duplicate = request(vec![
        choice("same", "Allow once", PermissionEffect::AllowOnce),
        choice("same", "Allow always", PermissionEffect::AllowRemembered),
    ]);
    assert_eq!(allow_once(&duplicate), None);
    assert!(!unique_option(&duplicate, &PermissionOptionId::new("same")));
    assert!(!unique_option(
        &duplicate,
        &PermissionOptionId::new("foreign")
    ));
}
