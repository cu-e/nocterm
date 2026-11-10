//! The agent registers as the required plugin its descriptor declares.
use super::*;
use crate::plugin::{CHATS, PLUGIN};
use nocterm_ui::contribution_icon;
use nocterm_workspace::test_support::{Toggle, toggle_presentation};

#[gpui_kit::test]
fn the_side_panel_toggle_shows_the_descriptors_title_and_icon(cx: &mut TestAppContext) {
    let f = fixture(cx);

    cx.update(|cx| {
        let workspace = f.workspace.read(cx);
        assert!(workspace.is_registered(&CHATS.id));
        assert_eq!(
            toggle_presentation(workspace, Toggle::SidePanel, cx),
            Some((CHATS.title.into(), contribution_icon(CHATS.icon)))
        );
    });
}

#[gpui_kit::test]
fn the_agent_is_a_required_plugin_and_the_master_switch_withdraws_its_panel(
    cx: &mut TestAppContext,
) {
    let f = fixture(cx);
    assert!(PLUGIN.required);
    assert!(cx.update(|cx| f.workspace.read(cx).panel_is_available(&CHATS.id)));

    let disabled = cx.update(|cx| {
        f.installation
            .ready
            .settings()
            .update::<nocterm_ai::AiSettings>(cx, |settings| settings.enabled = false)
    });
    cx.foreground_executor().block_test(disabled).unwrap();
    cx.run_until_parked();

    assert!(cx.update(|cx| f.workspace.read(cx).is_registered(&CHATS.id)));
    assert!(!cx.update(|cx| f.workspace.read(cx).panel_is_available(&CHATS.id)));
}
