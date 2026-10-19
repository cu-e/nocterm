use super::*;
use nocterm_ai::model::auth::{PermissionChoice, PermissionEffect, ProviderPermission};
use nocterm_ai::model::tools::{ToolChanges, ToolPatch};

fn request(options: Vec<PermissionChoice>) -> ProviderPermission {
    ProviderPermission::new(
        "session",
        ToolPatch::new("tool", ToolChanges::default()),
        options,
    )
}

#[test]
fn automatic_choice_uses_kind_regardless_of_order_or_label() {
    let mut options = vec![
        PermissionChoice::new("deny", "Allow once", PermissionEffect::DenyOnce),
        PermissionChoice::new("persistent", "Allow", PermissionEffect::AllowRemembered),
        PermissionChoice::new("once", "Reject", PermissionEffect::AllowOnce),
    ];
    for _ in 0..options.len() {
        assert_eq!(allow_once(&request(options.clone())), Some("once".into()));
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
            allow_once(&request(vec![PermissionChoice::new(
                "id",
                "Allow once",
                kind
            )])),
            None
        );
    }
    assert_eq!(allow_once(&request(Vec::new())), None);
}

#[test]
fn missing_or_ambiguous_option_ids_cannot_be_selected() {
    let empty = request(vec![PermissionChoice::new(
        " ",
        "Allow",
        PermissionEffect::AllowOnce,
    )]);
    assert_eq!(allow_once(&empty), None);
    assert!(!unique_option(&empty, &" ".into()));
    let duplicate = request(vec![
        PermissionChoice::new("same", "Allow once", PermissionEffect::AllowOnce),
        PermissionChoice::new("same", "Allow always", PermissionEffect::AllowRemembered),
    ]);
    assert_eq!(allow_once(&duplicate), None);
    assert!(!unique_option(&duplicate, &"same".into()));
    assert!(!unique_option(&duplicate, &"foreign".into()));
}
