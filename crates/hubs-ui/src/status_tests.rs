//! The chip of a server that speaks another protocol says which side needs the update.

use super::status_chip;
use crate::test_support::{fixture, server};
use gpui_kit::TestAppContext;
use nocterm_hubs::HubStatus;
use nocterm_hubs_model::testing::FakeHub;

fn label(status: HubStatus, cx: &mut TestAppContext) -> String {
    let _page = fixture(cx, FakeHub::new().with_server(server()), Vec::new(), None);
    cx.update(|cx| status_chip(&status, cx).0.to_string())
}

#[gpui_kit::test]
fn a_server_behind_this_nocterm_is_the_one_to_update(cx: &mut TestAppContext) {
    let status = HubStatus::UpgradeRequired { hub: 1, client: 2 };
    assert_eq!(label(status, cx), "Update the server");
}

#[gpui_kit::test]
fn a_server_ahead_of_this_nocterm_asks_to_update_nocterm(cx: &mut TestAppContext) {
    let status = HubStatus::UpgradeRequired { hub: 3, client: 2 };
    assert_eq!(label(status, cx), "Update Nocterm");
}
