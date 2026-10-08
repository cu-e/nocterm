use super::*;

fn request(options: Vec<acp::PermissionOption>) -> acp::RequestPermissionRequest {
    serde_json::from_value(serde_json::json!({
        "sessionId":"session", "toolCall":{"toolCallId":"tool"}, "options":options
    }))
    .unwrap()
}

#[test]
fn automatic_choice_uses_kind_regardless_of_order_or_label() {
    let mut options = vec![
        acp::PermissionOption::new("deny", "Allow once", acp::PermissionOptionKind::RejectOnce),
        acp::PermissionOption::new(
            "persistent",
            "Allow",
            acp::PermissionOptionKind::AllowAlways,
        ),
        acp::PermissionOption::new("once", "Reject", acp::PermissionOptionKind::AllowOnce),
    ];
    for _ in 0..options.len() {
        assert_eq!(allow_once(&request(options.clone())), Some("once".into()));
        options.rotate_left(1);
    }
}

#[test]
fn persistent_or_rejection_options_never_qualify_for_automatic_approval() {
    for kind in [
        acp::PermissionOptionKind::AllowAlways,
        acp::PermissionOptionKind::RejectOnce,
        acp::PermissionOptionKind::RejectAlways,
    ] {
        assert_eq!(
            allow_once(&request(vec![acp::PermissionOption::new(
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
    let empty = request(vec![acp::PermissionOption::new(
        " ",
        "Allow",
        acp::PermissionOptionKind::AllowOnce,
    )]);
    assert_eq!(allow_once(&empty), None);
    assert!(!unique_option(&empty, &" ".into()));
    let duplicate = request(vec![
        acp::PermissionOption::new("same", "Allow once", acp::PermissionOptionKind::AllowOnce),
        acp::PermissionOption::new(
            "same",
            "Allow always",
            acp::PermissionOptionKind::AllowAlways,
        ),
    ]);
    assert_eq!(allow_once(&duplicate), None);
    assert!(!unique_option(&duplicate, &"same".into()));
    assert!(!unique_option(&duplicate, &"foreign".into()));
}
