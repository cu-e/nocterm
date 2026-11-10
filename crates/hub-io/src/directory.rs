//! Account display values are checked against the original authenticated native subject.
use crate::{account::key_admission::KeyAdmission, convert};
use nocterm_hub_client::{ClientError, HubClient};
use nocterm_hub_proto::v2;
use nocterm_hubs_model::{Account, HubError};

pub(crate) async fn who_am_i(
    client: &HubClient,
    owner: &KeyAdmission,
) -> Result<Account, HubError> {
    let response = owner
        .during(client.account().who_am_i(v2::Empty {}))
        .await?
        .map_err(|status| owner.failure(ClientError::from(status)))?
        .into_inner();
    checked_account(owner, convert::account(response))
}
pub(crate) fn checked_account(owner: &KeyAdmission, account: Account) -> Result<Account, HubError> {
    owner.check()?;
    if account.id.as_str() != owner.member().subject()?.account() {
        return Err(HubError::Stale);
    }
    Ok(account)
}
