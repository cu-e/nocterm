//! Linux releases a session-only KEK through a root-owned broker, never keyring.
use crate::platform;
use nocterm_vault::{
    DeviceAvailability as A, DeviceCancellation, DeviceCapability, DeviceRegistrationState,
    DeviceUnlockError as E, DeviceUnlockProvider, VaultBinding, VaultKey,
};
use std::{sync::Mutex, time::Duration};
use zbus::{
    Connection, Proxy,
    fdo::DBusProxy,
    names::{BusName, WellKnownName},
};
const BROKER: &str = "dev.nocterm.VaultBroker1";
const PATH: &str = "/dev/nocterm/VaultBroker1";
const FPRINT: &str = "net.reactivated.Fprint";
struct Client {
    runtime: tokio::runtime::Runtime,
    connection: Connection,
}
#[derive(Default)]
pub(super) struct Linux {
    client: Mutex<Option<Client>>,
}
impl Client {
    fn bounded<T>(
        &self,
        timeout: Duration,
        future: impl std::future::Future<Output = Result<T, E>>,
    ) -> Result<T, E> {
        self.runtime.block_on(async {
            tokio::time::timeout(timeout, future)
                .await
                .map_err(|_| E::Platform("System authentication service timed out".into()))?
        })
    }
}
impl Linux {
    fn run<T>(&self, op: impl FnOnce(&Client) -> Result<T, E>) -> Result<T, E> {
        let mut guard = self.client.lock().map_err(platform)?;
        if guard.is_none() {
            let runtime = tokio::runtime::Builder::new_multi_thread()
                .worker_threads(1)
                .enable_all()
                .build()
                .map_err(platform)?;
            let connection = runtime
                .block_on(async {
                    tokio::time::timeout(
                        Duration::from_secs(5),
                        zbus::connection::Builder::address(
                            "unix:path=/run/dbus/system_bus_socket",
                        )?
                        .method_timeout(Duration::from_secs(40))
                        .build(),
                    )
                    .await
                    .map_err(|_| zbus::Error::Failure("Bus connection timed out".into()))?
                })
                .map_err(platform)?;
            *guard = Some(Client {
                runtime,
                connection,
            });
        }
        op(guard.as_ref().expect("initialized client"))
    }
}
async fn trusted(connection: &Connection, name: &str) -> Result<(), E> {
    let dbus = DBusProxy::new(connection).await.map_err(platform)?;
    // fprintd exits while idle and packaged brokers start on demand. D-Bus
    // activation needs no authentication; the owner is checked below.
    let _ = dbus
        .start_service_by_name(WellKnownName::try_from(name).map_err(platform)?, 0)
        .await;
    let owner = dbus
        .get_name_owner(BusName::try_from(name).map_err(platform)?)
        .await
        .map_err(platform)?;
    let uid = dbus
        .get_connection_unix_user(BusName::from(owner))
        .await
        .map_err(platform)?;
    if uid != 0 {
        return Err(E::Unavailable);
    }
    Ok(())
}
fn broker_missing() -> String {
    let install = match std::env::var("APPIMAGE") {
        Ok(appimage) => format!("Run \"{appimage}\" --install-vault-broker once"),
        Err(_) => "Install Nocterm from the .deb or .rpm package, or run \
                   scripts/install-vault-broker.sh as administrator"
            .into(),
    };
    format!(
        "Fingerprint reader detected, but the Nocterm vault broker is not installed. \
         {install}. The master password always works."
    )
}
// Only recognized broker envelopes are presentation noise. Keep unexpected
// transport or protocol failures intact so their cause is still visible.
fn broker_release_error(error: zbus::Error) -> E {
    match error {
        zbus::Error::MethodError(name, _, _)
            if name.as_str() == "dev.nocterm.VaultBroker1.Error.Locked" =>
        {
            E::Locked
        }
        zbus::Error::MethodError(name, Some(description), _)
            if matches!(
                name.as_str(),
                "org.freedesktop.DBus.Error.AccessDenied" | "org.freedesktop.DBus.Error.Failed"
            ) =>
        {
            E::Platform(description)
        }
        error => platform(error),
    }
}
// Only the old protocol's missing method disables progress; other failures matter.
fn attempts_result(result: Result<u8, zbus::Error>) -> Result<Option<u8>, E> {
    match result {
        Ok(remaining @ 0..=3) => Ok(Some(remaining)),
        Ok(_) => Err(E::Invalidated),
        Err(zbus::Error::MethodError(name, _, _))
            if name.as_str() == "org.freedesktop.DBus.Error.UnknownMethod" =>
        {
            Ok(None)
        }
        Err(error) => Err(broker_release_error(error)),
    }
}
async fn attempts_remaining(proxy: &Proxy<'_>, arguments: &(&str, &str)) -> Result<Option<u8>, E> {
    let result = tokio::time::timeout(
        Duration::from_secs(1),
        proxy.call("AttemptsRemaining", arguments),
    )
    .await
    .map_err(|_| E::Platform("Fingerprint progress request timed out".into()))?;
    attempts_result(result)
}
fn token_string(token: &[u8]) -> Result<&str, E> {
    let token = std::str::from_utf8(token).map_err(|_| E::Invalidated)?;
    if token.len() != 64
        || !token
            .bytes()
            .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
    {
        return Err(E::Invalidated);
    }
    Ok(token)
}

async fn cancel_release(proxy: &Proxy<'_>, arguments: &(&str, &str)) {
    let _ = tokio::time::timeout(
        Duration::from_secs(1),
        proxy.call::<_, _, ()>("Cancel", arguments),
    )
    .await;
}

async fn release_key(
    proxy: &Proxy<'_>,
    arguments: &(&str, &str),
    cancel: &DeviceCancellation,
) -> Result<VaultKey, E> {
    let mut remaining = attempts_remaining(proxy, arguments).await?;
    if let Some(value) = remaining {
        cancel.report_attempts_remaining(value);
    }
    if remaining == Some(0) {
        return Err(E::Locked);
    }
    cancel.check()?;
    // Fingerprint authentication needs a longer budget than passive probes.
    let future = proxy.call::<_, _, Vec<u8>>("Release", arguments);
    tokio::pin!(future);
    let bytes = loop {
        tokio::select! {
            result = &mut future => break zeroize::Zeroizing::new(result.map_err(broker_release_error)?),
            _ = tokio::time::sleep(Duration::from_millis(50)) => {
                if cancel.is_cancelled() {
                    cancel_release(proxy, arguments).await;
                    return Err(E::Cancelled);
                }
                if remaining.is_some() {
                    let value = match attempts_remaining(proxy, arguments).await {
                        Ok(value) => value,
                        Err(error) => {
                            cancel_release(proxy, arguments).await;
                            return Err(error);
                        }
                    };
                    if value != remaining {
                        if let Some(value) = value { cancel.report_attempts_remaining(value); }
                        remaining = value;
                    }
                }
            }
        }
    };
    cancel.check()?;
    if bytes.len() != 32 {
        return Err(E::Invalidated);
    }
    let mut key = VaultKey::new([0; 32]);
    key.copy_from_slice(&bytes);
    Ok(key)
}

impl DeviceUnlockProvider for Linux {
    fn probe(&self) -> Result<DeviceCapability, E> {
        let result = self.run(|client| {
            client.bounded(Duration::from_secs(8), async {
                trusted(&client.connection, FPRINT).await?;
                let manager = Proxy::new(
                    &client.connection,
                    FPRINT,
                    "/net/reactivated/Fprint/Manager",
                    "net.reactivated.Fprint.Manager",
                ).await.map_err(platform)?;
                let devices: Vec<zbus::zvariant::OwnedObjectPath> = manager
                    .call("GetDevices", &()).await.map_err(platform)?;
                if devices.is_empty() {
                    return Ok((A::NoHardware, "No supported fingerprint reader was found.".into()));
                }
                if devices.len() > 16 {
                    return Err(E::Unavailable);
                }
                let user = nix::unistd::User::from_uid(nix::unistd::getuid())
                    .map_err(platform)?.ok_or(E::Unavailable)?;
                let mut enrolled = false;
                for device in devices {
                    let proxy = Proxy::new(&client.connection, FPRINT, device,
                        "net.reactivated.Fprint.Device").await.map_err(platform)?;
                    let fingers: Result<Vec<String>, _> = proxy
                        .call("ListEnrolledFingers", &(user.name.as_str(),)).await;
                    enrolled |= fingers.is_ok_and(|fingers| !fingers.is_empty() && fingers.len() <= 10);
                }
                if !enrolled {
                    return Ok((A::NotEnrolled, "Register a fingerprint in your operating system settings.".into()));
                }
                if trusted(&client.connection, BROKER).await.is_err() {
                    return Ok((A::BrokerMissing, broker_missing()));
                }
                Ok((A::Available, "Unlock once with the master password after the computer starts. Fingerprint unlock then works, also after restarting Nocterm.".into()))
            })
        });
        let (availability, detail) = result.unwrap_or((
            A::Unavailable,
            "Fingerprint services are unavailable. Use the master password.".into(),
        ));
        Ok(DeviceCapability {
            availability,
            label: "Fingerprint".into(),
            detail,
            enabled: false,
            armed: false,
            session_only: true,
        })
    }
    fn enroll(
        &self,
        binding: VaultBinding,
        key: VaultKey,
        cancel: &DeviceCancellation,
    ) -> Result<Vec<u8>, E> {
        cancel.check()?;
        let token = self.run(|client| {
            client.bounded(Duration::from_secs(8), async {
                trusted(&client.connection, BROKER).await?;
                let proxy = Proxy::new(&client.connection, BROKER, PATH, BROKER)
                    .await
                    .map_err(platform)?;
                proxy
                    .call::<_, _, String>("Enroll", &(binding.identifier(), key.as_slice()))
                    .await
                    .map_err(platform)
            })
        })?;
        if let Err(error) = cancel.check() {
            let _ = self.remove(binding, token.as_bytes());
            return Err(error);
        }
        token_string(token.as_bytes())?;
        Ok(token.into_bytes())
    }
    fn release(
        &self,
        binding: VaultBinding,
        token: &[u8],
        cancel: &DeviceCancellation,
    ) -> Result<VaultKey, E> {
        let token = token_string(token)?;
        cancel.check()?;
        self.run(|client| {
            client.bounded(Duration::from_secs(40), async {
                trusted(&client.connection, BROKER).await?;
                let proxy = Proxy::new(&client.connection, BROKER, PATH, BROKER)
                    .await
                    .map_err(platform)?;
                let identity = binding.identifier();
                let arguments = (identity.as_str(), token);
                release_key(&proxy, &arguments, cancel).await
            })
        })
    }
    fn remove(&self, binding: VaultBinding, token: &[u8]) -> Result<(), E> {
        let token = token_string(token)?;
        self.run(|client| {
            client.bounded(Duration::from_secs(8), async {
                // A terminated/restarted broker has already erased its session keys.
                if trusted(&client.connection, BROKER).await.is_err() {
                    return Ok(());
                }
                let proxy = Proxy::new(&client.connection, BROKER, PATH, BROKER)
                    .await
                    .map_err(platform)?;
                proxy
                    .call("Remove", &(binding.identifier(), token))
                    .await
                    .map_err(platform)
            })
        })
    }
    fn registration_state(
        &self,
        binding: VaultBinding,
        token: &[u8],
    ) -> Result<DeviceRegistrationState, E> {
        let token = token_string(token)?;
        self.run(|client| {
            client.bounded(Duration::from_secs(8), async {
                trusted(&client.connection, BROKER).await?;
                let proxy = Proxy::new(&client.connection, BROKER, PATH, BROKER)
                    .await
                    .map_err(platform)?;
                let identity = binding.identifier();
                let arguments = (identity.as_str(), token);
                let registered: bool = proxy
                    .call("Registered", &arguments)
                    .await
                    .map_err(platform)?;
                if !registered {
                    return Ok(DeviceRegistrationState::Missing);
                }
                Ok(
                    if attempts_remaining(&proxy, &arguments).await? == Some(0) {
                        DeviceRegistrationState::Locked
                    } else {
                        DeviceRegistrationState::Ready
                    },
                )
            })
        })
    }
    fn registered(&self, binding: VaultBinding, token: &[u8]) -> bool {
        let Ok(token) = token_string(token) else {
            return false;
        };
        self.run(|client| {
            client.bounded(Duration::from_secs(8), async {
                trusted(&client.connection, BROKER).await?;
                let proxy = Proxy::new(&client.connection, BROKER, PATH, BROKER)
                    .await
                    .map_err(platform)?;
                proxy
                    .call("Registered", &(binding.identifier(), token))
                    .await
                    .map_err(platform)
            })
        })
        .unwrap_or(false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn attempts_protocol_only_falls_back_for_old_brokers() {
        assert_eq!(attempts_result(Ok(2)).unwrap(), Some(2));
        assert_eq!(
            attempts_result(Err(method_error(
                "org.freedesktop.DBus.Error.UnknownMethod",
                Some("missing")
            )))
            .unwrap(),
            None
        );
        assert!(
            attempts_result(Err(method_error(
                "org.freedesktop.DBus.Error.AccessDenied",
                Some("denied")
            )))
            .is_err()
        );
        assert!(attempts_result(Ok(4)).is_err());
        assert!(matches!(
            broker_release_error(method_error(
                "dev.nocterm.VaultBroker1.Error.Locked",
                Some("locked")
            )),
            E::Locked
        ));
    }
    mod protocol;
    #[test]
    fn broker_tokens_are_bounded_canonical_identifiers() {
        for invalid in [
            vec![255; 64],
            b"../../vault".to_vec(),
            b"a".repeat(65),
            b"A".repeat(64),
        ] {
            assert!(token_string(&invalid).is_err());
        }
        assert_eq!(token_string(&b"a".repeat(64)).unwrap(), "a".repeat(64));
    }
    fn method_error(name: &str, description: Option<&str>) -> zbus::Error {
        let message = zbus::Message::method_call(PATH, "Release")
            .unwrap()
            .build(&())
            .unwrap();
        zbus::Error::MethodError(
            name.try_into().unwrap(),
            description.map(str::to_string),
            message,
        )
    }
    #[test]
    fn broker_release_errors_show_known_descriptions_and_keep_unexpected_errors() {
        for name in [
            "org.freedesktop.DBus.Error.AccessDenied",
            "org.freedesktop.DBus.Error.Failed",
        ] {
            let error =
                broker_release_error(method_error(name, Some("Fingerprint reader disconnected")));
            assert_eq!(
                error.to_string(),
                "Device unlock failed: Fingerprint reader disconnected"
            );
        }
        let error = method_error(
            "net.reactivated.Fprint.Error.PermissionDenied",
            Some("Permission denied"),
        );
        let expected = platform(error.clone()).to_string();
        assert_eq!(broker_release_error(error).to_string(), expected);
        let error = method_error("org.freedesktop.DBus.Error.Failed", None);
        assert_eq!(
            broker_release_error(error.clone()).to_string(),
            platform(error).to_string()
        );
        let error = zbus::Error::Failure("Connection closed".into());
        assert_eq!(
            broker_release_error(error.clone()).to_string(),
            platform(error).to_string()
        );
    }
}
