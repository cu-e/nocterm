//! The Credentials tab: a password added under a name, renamed, detached from a server.
use super::*;

fn unlocked_on_credentials(
    cx: &mut TestAppContext,
) -> (gpui_kit::AnyWindowHandle, Arc<ScriptedVaultAccess>) {
    let (handle, _, service) = setup(cx);
    block_on(service.create(Secret::new("master password"))).unwrap();
    drain(cx);
    tab(handle, 1, cx);
    drain(cx);
    (handle, service)
}

fn listed(service: &ScriptedVaultAccess) -> Vec<nocterm_vault::CredentialInfo> {
    block_on(service.list()).unwrap()
}

#[gpui_kit::test]
fn a_password_added_under_a_name_signs_in_to_no_server_until_one_chooses_it(
    cx: &mut TestAppContext,
) {
    let (handle, service) = unlocked_on_credentials(cx);
    click(handle, "vault-add-password", cx);
    click(handle, "vault-add-confirm", cx);
    drain(cx);
    assert!(
        listed(&service).is_empty(),
        "a name and a password are needed"
    );
    type_field(handle, "vault-new-label", "Office", cx);
    type_field(handle, "vault-new-password", "shared secret", cx);
    click(handle, "vault-add-confirm", cx);
    drain(cx);
    let records = listed(&service);
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].label, "Office");
    assert_eq!(records[0].binding, CredentialBinding::password([]));
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(
            !gpui_kit::component::WindowExt::has_active_dialog(window, cx),
            "the dialog closes once it is kept"
        );
        assert!(window.try_find(("vault-rename", 0usize)).is_some());
    })
    .unwrap();
}

#[gpui_kit::test]
fn the_search_narrows_the_list_by_name_or_server(cx: &mut TestAppContext) {
    let (handle, service) = unlocked_on_credentials(cx);
    for (label, host) in [("Office", "office.example"), ("Lab", "lab.example")] {
        block_on(service.put(
            None,
            label.into(),
            CredentialBinding::password([Target::new("ops", host, 22)]),
            Secret::new("pw"),
        ))
        .unwrap();
    }
    click(handle, "vault-refresh", cx);
    drain(cx);
    let shown = |cx: &mut TestAppContext| {
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            (0..3usize)
                .filter(|index| window.try_find(("vault-rename", *index)).is_some())
                .count()
        })
        .unwrap()
    };
    assert_eq!(shown(cx), 2);
    type_field(handle, "vault-search", "lab.ex", cx);
    assert_eq!(shown(cx), 1, "a server's address finds its password");
    type_field(handle, "vault-search", "nothing", cx);
    assert_eq!(shown(cx), 0);
}

#[gpui_kit::test]
fn a_credential_is_renamed_and_a_shared_password_detached_from_one_server(cx: &mut TestAppContext) {
    let (handle, service) = unlocked_on_credentials(cx);
    let (a, b) = (
        Target::new("ops", "a.test", 22),
        Target::new("ops", "b.test", 22),
    );
    let id = block_on(service.put(
        None,
        "Office".into(),
        CredentialBinding::password([a.clone(), b.clone()]),
        Secret::new("shared secret"),
    ))
    .unwrap();
    click(handle, "vault-refresh", cx);
    drain(cx);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("vault-detach-0-1").is_some());
        window.click(("vault-rename", 0usize), cx);
        window.render_frame(cx);
        window.press("ctrl-a", cx);
        window.input("Office VPN", cx);
        window.press("enter", cx);
    })
    .unwrap();
    drain(cx);
    assert_eq!(listed(&service)[0].label, "Office VPN");
    click(handle, "vault-detach-0-0", cx);
    drain(cx);
    let record = &listed(&service)[0];
    assert_eq!(record.binding, CredentialBinding::password([b.clone()]));
    assert_eq!(record.label, "Office VPN");
    // The secret stays: the password still signs in to the other server.
    let asked = CredentialBinding::password([b]);
    assert_eq!(
        block_on(service.get(id, asked)).unwrap().expose(),
        "shared secret"
    );
    assert!(block_on(service.get(id, CredentialBinding::password([a]))).is_err());
}
