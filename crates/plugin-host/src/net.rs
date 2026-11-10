//! How a plugin's request is built from its settings: the origin the user typed and the
//! credential the plugin never reads.

use nocterm_plugin_api::{
    Manifest,
    capability::net::{NetAnswer, NetCall, Origin, Request},
};
use nocterm_plugin_model::{SettingValue, SettingValues};

/// The request a plugin asked for, at the origin and with the credential its settings hold; or
/// the words for why none can be sent, which the plugin receives as a status-0 response.
pub(crate) fn build(
    manifest: &Manifest,
    settings: &SettingValues,
    request: &Request,
) -> Result<NetCall, NetAnswer> {
    let Some(network) = manifest.network() else {
        return Err(NetAnswer::failed("the package has no [network] table"));
    };
    let text = |key: &str| match settings.get(key) {
        Some(SettingValue::Text(text)) => text.trim(),
        _ => "",
    };
    let origin = Origin::parse(text(network.origin_setting())).map_err(|error| {
        NetAnswer::failed(format!("Check the address in the settings: {error}"))
    })?;
    let url = origin
        .url(&request.path)
        .map_err(|error| NetAnswer::failed(format!("the path is refused: {error}")))?;
    let credential = network.credential().and_then(|credential| {
        let secret = text(credential.setting());
        (!secret.is_empty()).then(|| {
            (
                credential.header().to_owned(),
                format!("{}{secret}", credential.prefix()),
            )
        })
    });
    Ok(NetCall {
        method: request.method,
        url,
        origin,
        body: request.body.as_deref().map(|body| {
            if request.bytes {
                nocterm_plugin_api::base64::decode(body).unwrap_or_default()
            } else {
                body.as_bytes().to_vec()
            }
        }),
        bytes: request.bytes,
        credential,
    })
}

/// The settings after a change, with the credential forgotten when the address it was entered
/// for changed and the change did not set a new one: a key never follows a changed address.
pub(crate) fn bind_credential(
    manifest: &Manifest,
    before: &SettingValues,
    mut after: SettingValues,
) -> SettingValues {
    let Some(network) = manifest.network() else {
        return after;
    };
    let Some(credential) = network.credential() else {
        return after;
    };
    let origin = network.origin_setting();
    let moved = before.get(origin) != after.get(origin);
    let kept = before.get(credential.setting()) == after.get(credential.setting());
    if moved && kept {
        after.insert(credential.setting(), SettingValue::Text(String::new()));
    }
    after
}
