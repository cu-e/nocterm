use super::*;
#[gpui_kit::test]
fn plain_settings_without_ports_offer_default_and_do_not_fetch(cx: &mut TestAppContext) {
    let (_, view, _) = open(cx);
    cx.run_until_parked();
    cx.update(|cx| assert_eq!(view.read(cx).selected_page_id(), "appearance"));
}
