//! Who a signed-in device is on a hub.

use nocterm_hub_client::{ClientError, HubClient};
use nocterm_hub_proto::v1;
use nocterm_hubs_model::{Account, HubError, HubId};

use crate::convert;
use crate::pool::Pool;

/// Who the device is signed in as on a hub.
pub(crate) async fn who_am_i(
    client: &HubClient,
    hub: &HubId,
    pool: &Pool,
) -> Result<Account, HubError> {
    let me = client
        .directory()
        .who_am_i(v1::Empty {})
        .await
        .map_err(|status| pool.failed(hub, ClientError::from(status)))?
        .into_inner();
    Ok(convert::account(me))
}
