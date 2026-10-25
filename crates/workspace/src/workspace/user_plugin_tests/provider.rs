//! What a user plugin may ask of its own panel, through the window's provider: the window
//! enforces the rules, so a request that breaks one is refused whoever sends it.
use nocterm_plugin_api::{
    HostError, LocalId,
    capability::panels::{SetOpen, SetPanelOpen},
};
use nocterm_plugin_ports::{Apply, Surface as _};

use super::*;
use crate::ShellCapabilities;

fn plugin() -> PluginId {
    PluginId::user("acme", "notes").expect("a valid user id")
}

fn local() -> LocalId {
    LocalId::parse("panel").expect("a valid local id")
}

/// The provider of the window, whose sidebar group starts hidden: with another member showing in
/// it, opening the plugin's panel would replace that one.
fn provider(cx: &mut TestAppContext, rig: &Rig) -> ShellCapabilities {
    if cx.update(|cx| rig.workspace.read(cx).panel_is_open(&PANEL_A.id)) {
        cx.update_window(rig.handle, |_, window, cx| {
            rig.workspace.update(cx, |workspace, cx| {
                workspace.toggle_panel(&PANEL_A.id, window, cx)
            })
        })
        .unwrap();
        settle(cx, rig);
    }
    let provider = cx.update(|cx| ShellCapabilities::new(&rig.workspace, rig.handle, cx));
    cx.run_until_parked();
    provider
}

/// What the window answers a plugin that asks to show or close its panel.
fn ask(
    cx: &mut TestAppContext,
    rig: &Rig,
    provider: &ShellCapabilities,
    open: bool,
) -> Result<(), HostError> {
    let request = SetOpen {
        local: local(),
        open,
    };
    let answer = cx.update(|cx| Apply::<SetPanelOpen>::apply(provider, &plugin(), request, cx));
    settle(cx, rig);
    answer
}

fn open_panels(cx: &mut TestAppContext, rig: &Rig) -> Vec<bool> {
    cx.update(|cx| {
        let workspace = rig.workspace.read(cx);
        [panel_id(), PANEL_A.id.clone(), EXPANDABLE.id.clone()]
            .iter()
            .map(|id| workspace.panel_is_open(id))
            .collect()
    })
}

fn focus(cx: &mut TestAppContext, rig: &Rig) -> Option<gpui_kit::FocusHandle> {
    cx.update_window(rig.handle, |_, window, cx| window.focused(cx))
        .unwrap()
}

#[gpui_kit::test]
fn a_plugin_opens_and_closes_its_panel_and_focus_stays_where_it_was(cx: &mut TestAppContext) {
    let rig = rig(cx, 1400., set(1, vec![entry(true, true)]));
    let provider = provider(cx, &rig);
    let before = focus(cx, &rig);

    assert_eq!(ask(cx, &rig, &provider, true), Ok(()));
    assert!(state(cx, &rig).2, "the panel shows");
    assert_eq!(focus(cx, &rig), before);

    assert_eq!(ask(cx, &rig, &provider, false), Ok(()));
    assert!(!state(cx, &rig).2, "the panel closed");
    assert_eq!(focus(cx, &rig), before);
}

#[gpui_kit::test]
fn a_plugin_cannot_replace_the_panel_the_user_has_open(cx: &mut TestAppContext) {
    let rig = rig(cx, 1400., set(1, vec![entry(true, true)]));
    let provider = provider(cx, &rig);
    cx.update_window(rig.handle, |_, window, cx| {
        rig.workspace.update(cx, |workspace, cx| {
            workspace.open_panel(&PANEL_A.id, window, cx)
        })
    })
    .unwrap();
    settle(cx, &rig);
    let before = open_panels(cx, &rig);

    assert_eq!(
        ask(cx, &rig, &provider, true),
        Err(HostError::Unavailable),
        "panel A shows in the same group"
    );

    assert_eq!(open_panels(cx, &rig), before, "nothing was replaced");
    assert!(before[1], "the panel the user opened still shows");
}

#[gpui_kit::test]
fn attention_raised_between_the_snapshot_and_the_apply_refuses_the_open(cx: &mut TestAppContext) {
    let rig = rig(cx, 1400., set(1, vec![entry(true, true)]));
    let provider = provider(cx, &rig);
    // The host checked the panel when it queued the request.
    assert_eq!(
        cx.update(|cx| provider.permit(&plugin(), &local(), cx)),
        Ok(())
    );
    let before = open_panels(cx, &rig);
    // Then a panel asked for the user, before the request was applied.
    cx.update_window(rig.handle, |_, _, cx| {
        rig.side_panel.update(cx, |panel, cx| {
            panel.set_attention(Some("approval pending".into()), cx)
        });
    })
    .unwrap();
    settle(cx, &rig);

    assert_eq!(ask(cx, &rig, &provider, true), Err(HostError::Denied));

    assert_eq!(open_panels(cx, &rig), before, "the layout did not change");
}

#[gpui_kit::test]
fn a_request_for_a_panel_the_user_did_not_approve_is_unavailable(cx: &mut TestAppContext) {
    let rig = rig(cx, 1400., set(1, vec![entry(true, false)]));
    let provider = provider(cx, &rig);

    assert_eq!(
        cx.update(|cx| provider.permit(&plugin(), &local(), cx)),
        Err(HostError::Unavailable)
    );
    assert_eq!(ask(cx, &rig, &provider, true), Err(HostError::Unavailable));
    assert!(!state(cx, &rig).2);

    publish(cx, &rig, set(2, vec![entry(true, true)]));
    assert_eq!(ask(cx, &rig, &provider, true), Ok(()));
    assert!(state(cx, &rig).2, "approval opens it");
}

#[gpui_kit::test]
fn a_request_for_a_disabled_or_unknown_panel_is_unavailable(cx: &mut TestAppContext) {
    let rig = rig(cx, 1400., set(1, vec![entry(false, true)]));
    let provider = provider(cx, &rig);

    assert_eq!(ask(cx, &rig, &provider, true), Err(HostError::Unavailable));
    assert_eq!(rig.made.get(), 0, "a disabled plugin creates nothing");

    let stranger = SetOpen {
        local: LocalId::parse("elsewhere").unwrap(),
        open: true,
    };
    let answer = cx.update(|cx| Apply::<SetPanelOpen>::apply(&provider, &plugin(), stranger, cx));
    assert_eq!(answer, Err(HostError::Unavailable));
}

#[gpui_kit::test]
fn closing_a_panel_that_does_not_show_changes_nothing(cx: &mut TestAppContext) {
    let rig = rig(cx, 1400., set(1, vec![entry(true, true)]));
    let provider = provider(cx, &rig);
    cx.update_window(rig.handle, |_, window, cx| {
        rig.workspace.update(cx, |workspace, cx| {
            workspace.open_panel(&PANEL_A.id, window, cx)
        })
    })
    .unwrap();
    settle(cx, &rig);
    let before = open_panels(cx, &rig);

    assert_eq!(ask(cx, &rig, &provider, false), Ok(()));

    assert_eq!(open_panels(cx, &rig), before);
}

#[gpui_kit::test]
fn withdrawing_closes_the_panel_and_lets_the_plugin_register_again(cx: &mut TestAppContext) {
    let rig = rig(cx, 1400., set(1, vec![entry(true, true)]));
    let provider = provider(cx, &rig);
    assert_eq!(ask(cx, &rig, &provider, true), Ok(()));

    cx.update_window(rig.handle, |_, window, cx| {
        rig.workspace.update(cx, |workspace, cx| {
            ShellCapabilities::withdraw(workspace, &panel_id(), window, cx)
        })
    })
    .unwrap()
    .expect("a user contribution can be withdrawn");
    settle(cx, &rig);
    assert_eq!(state(cx, &rig), (false, false, false));
    assert_eq!(ask(cx, &rig, &provider, true), Err(HostError::Unavailable));

    cx.update_window(rig.handle, |_, window, cx| {
        rig.workspace.update(cx, |workspace, cx| {
            register_user(&rig.made, workspace, window, cx)
        })
    })
    .unwrap();
    settle(cx, &rig);

    assert_eq!(state(cx, &rig), (true, true, false), "it starts closed");
}

#[gpui_kit::test]
fn withdrawing_a_built_in_contribution_is_refused_and_leaves_it_registered(
    cx: &mut TestAppContext,
) {
    let rig = rig(cx, 1400., set(1, vec![entry(true, true)]));
    let _provider = provider(cx, &rig);

    let answer = cx
        .update_window(rig.handle, |_, window, cx| {
            rig.workspace.update(cx, |workspace, cx| {
                ShellCapabilities::withdraw(workspace, &PANEL_A.id, window, cx)
            })
        })
        .unwrap();
    settle(cx, &rig);

    assert_eq!(answer, Err(crate::WithdrawRefused));
    assert!(cx.update(|cx| rig.workspace.read(cx).panel_is_available(&PANEL_A.id)));
}
