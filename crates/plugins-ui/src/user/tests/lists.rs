//! The detail's lists: what the plugin is allowed to do and its audit log, the newest first,
//! each folded past three lines.

use super::*;

#[gpui_kit::test]
fn a_long_list_of_permissions_shows_three_and_folds_the_rest(cx: &mut TestAppContext) {
    let many = UserPluginRow {
        granted: (1..=5).map(|n| format!("Permission {n}")).collect(),
        ..row()
    };
    let fixture = open(vec![many], Ok(candidate()), cx);
    frame(&fixture, cx, |window, cx| {
        window.click("user-plugin-row:user-acme-clock", cx)
    });
    frame(&fixture, cx, |window, cx| {
        assert_eq!(window.find("user-granted-fold").label(), Some("Show all 5"));
        window.click("user-granted-fold", cx);
    });
    frame(&fixture, cx, |window, cx| {
        assert_eq!(window.find("user-granted-fold").label(), Some("Show fewer"));
        window.click("user-plugin-row:user-acme-clock", cx);
    });
    frame(&fixture, cx, |window, _| {
        assert!(
            window.try_find("user-audit-fold").is_none(),
            "two audit lines need no fold"
        );
    });
}

#[gpui_kit::test]
fn the_audit_log_lists_the_newest_first(cx: &mut TestAppContext) {
    let fixture = open(vec![row()], Ok(candidate()), cx);
    frame(&fixture, cx, |window, cx| {
        window.click("user-plugin-row:user-acme-clock", cx)
    });
    frame(&fixture, cx, |window, _| {
        assert_eq!(
            window.find("user-audit:0").label(),
            Some("user-acme-clock disabled")
        );
        assert_eq!(
            window.find("user-audit:1").label(),
            Some("user-acme-clock installed")
        );
    });
}
