//! [`FingerprintBackend`] over fprintd on the system bus.
use super::{
    denied,
    fingerprint::{FingerprintBackend, FingerprintDevice, VerifyStatuses},
};
use crate::store::Owner;
use futures::StreamExt as _;
use zbus::{
    Connection, Proxy,
    fdo::{self, DBusProxy},
    names::{BusName, OwnedUniqueName, WellKnownName},
    proxy::SignalStream,
    zvariant::OwnedObjectPath,
};

pub(super) const FPRINT: &str = "net.reactivated.Fprint";
const MAX_DEVICES: usize = 16;
const MAX_FINGERS: usize = 10;

pub(super) struct FprintdBackend {
    connection: Connection,
    dbus: DBusProxy<'static>,
    /// The only user fprintd may run as.
    trusted_uid: u32,
}

impl FprintdBackend {
    pub(super) async fn new(connection: &Connection, trusted_uid: u32) -> fdo::Result<Self> {
        Ok(Self {
            connection: connection.clone(),
            dbus: DBusProxy::new(connection).await.map_err(denied)?,
            trusted_uid,
        })
    }

    /// The unique name of a running fprintd owned by the trusted user.
    async fn service(&self) -> fdo::Result<OwnedUniqueName> {
        let well_known = WellKnownName::try_from(FPRINT).map_err(denied)?;
        let _ = self.dbus.start_service_by_name(well_known, 0).await;
        let name = self
            .dbus
            .get_name_owner(BusName::try_from(FPRINT).map_err(denied)?)
            .await
            .map_err(denied)?;
        let uid = self
            .dbus
            .get_connection_unix_user(BusName::from(name.clone()))
            .await
            .map_err(denied)?;
        if uid != self.trusted_uid {
            return Err(denied("Untrusted fingerprint service"));
        }
        Ok(name)
    }
}

impl FingerprintBackend for FprintdBackend {
    type Device = FprintdDevice;

    async fn device(&self, owner: &Owner) -> fdo::Result<FprintdDevice> {
        let service = self.service().await?;
        let user = nix::unistd::User::from_uid(nix::unistd::Uid::from_raw(owner.uid))
            .map_err(denied)?
            .ok_or_else(|| denied("Unknown desktop user"))?
            .name;
        let manager = Proxy::new(
            &self.connection,
            service.clone(),
            "/net/reactivated/Fprint/Manager",
            "net.reactivated.Fprint.Manager",
        )
        .await
        .map_err(denied)?;
        let devices: Vec<OwnedObjectPath> =
            manager.call("GetDevices", &()).await.map_err(denied)?;
        if devices.len() > MAX_DEVICES {
            return Err(denied("Too many fingerprint devices"));
        }
        for path in devices {
            let proxy = Proxy::new(
                &self.connection,
                service.clone(),
                path,
                "net.reactivated.Fprint.Device",
            )
            .await
            .map_err(denied)?;
            let fingers: Result<Vec<String>, _> =
                proxy.call("ListEnrolledFingers", &(user.as_str(),)).await;
            if fingers.is_ok_and(|f| !f.is_empty() && f.len() <= MAX_FINGERS) {
                return Ok(FprintdDevice {
                    proxy,
                    service,
                    user,
                });
            }
        }
        Err(denied("No enrolled fingerprint reader"))
    }

    async fn client_present(&self, owner: &Owner) -> bool {
        let Ok(client) = BusName::try_from(owner.connection.as_str()) else {
            return false;
        };
        self.dbus.get_connection_unix_user(client).await.is_ok()
    }
}

pub(super) struct FprintdDevice {
    proxy: Proxy<'static>,
    service: OwnedUniqueName,
    user: String,
}

impl FingerprintDevice for FprintdDevice {
    type Statuses = FprintdStatuses;

    async fn claim(&self) -> fdo::Result<()> {
        self.proxy
            .call("Claim", &(self.user.as_str(),))
            .await
            .map_err(denied)
    }

    async fn verify_start(&self) -> fdo::Result<FprintdStatuses> {
        let stream = self
            .proxy
            .receive_signal("VerifyStatus")
            .await
            .map_err(denied)?;
        let _: () = self
            .proxy
            .call("VerifyStart", &("any",))
            .await
            .map_err(denied)?;
        Ok(FprintdStatuses {
            stream,
            service: self.service.clone(),
        })
    }

    async fn verify_stop(&self) -> fdo::Result<()> {
        self.proxy.call("VerifyStop", &()).await.map_err(denied)
    }

    async fn release(&self) -> fdo::Result<()> {
        self.proxy.call("Release", &()).await.map_err(denied)
    }
}

pub(super) struct FprintdStatuses {
    stream: SignalStream<'static>,
    service: OwnedUniqueName,
}

impl VerifyStatuses for FprintdStatuses {
    async fn next(&mut self) -> fdo::Result<(String, bool)> {
        let message = self
            .stream
            .next()
            .await
            .ok_or_else(|| denied("Fingerprint service disconnected"))?;
        if message.header().sender() != Some(&self.service) {
            return Err(denied("Untrusted fingerprint signal"));
        }
        message.body().deserialize().map_err(denied)
    }
}
