//! Raw `CloudService` calls for the device core's effects, signed with this device's key.
//!
//! The core seals probes and commands itself and hands out exact bytes; the effect executor
//! only has to send them. `nocterm-hub-client` at the pinned contract revision offers no
//! device-signed raw `CloudService` client (its probe and publication paths build their own
//! requests), so this adapter signs the request proof itself with the public layout
//! `nocterm_hub_model::pop::request_message`, the device key from its own credential file and
//! the client's current access token. `Capabilities` is a public call and goes unsigned.
//!
//! TODO(contract, P7): replace this module with a hub-client "device transport" that sends
//! core-built `ProbeHead`, `PublishOne`, `Replay`, `Watch`, `RunnerStatus` and
//! `Capabilities{nonce}` requests under the client's own signer (see the P6 report).
use nocterm_hub_client::{ClientError, HubClient};
use nocterm_hub_grpc::{cloud_service_client::CloudServiceClient, metadata, method_path};
use nocterm_hub_model::cloud::relay::MAX_RELAY_WIRE_BYTES;
use nocterm_keys::DeviceKey;
use std::sync::Arc;
use tonic::service::Interceptor;
use tonic::service::interceptor::InterceptedService;
use tonic::transport::Channel;
use tonic::{GrpcMethod, Request, Status};

/// The device's proof on one request: the request message of `pop`, signed.
#[derive(Clone)]
pub(super) struct DeviceProof {
    hub_id: Arc<str>,
    key: Arc<DeviceKey>,
    token: Arc<str>,
    device: Arc<str>,
}
impl Interceptor for DeviceProof {
    fn call(&mut self, mut request: Request<()>) -> Result<Request<()>, Status> {
        let method = request
            .extensions()
            .get::<GrpcMethod>()
            .ok_or_else(|| Status::internal("unknown method"))?;
        let path = method_path(method.service(), method.method());
        let nonce = nocterm_keys::random_bytes::<16>()
            .map(|bytes| {
                bytes
                    .iter()
                    .map(|byte| format!("{byte:02x}"))
                    .collect::<String>()
            })
            .map_err(|_| Status::internal("no randomness"))?;
        let signed_at = nocterm_hub_client::now_ms();
        let signature = self.key.sign(&nocterm_hub_model::pop::request_message(
            &self.hub_id,
            &path,
            &self.token,
            &nonce,
            signed_at,
        ));
        let ascii = |text: String| {
            tonic::metadata::AsciiMetadataValue::try_from(text)
                .map_err(|_| Status::internal("invalid metadata"))
        };
        let headers = request.metadata_mut();
        let mut authorization = ascii(format!("Bearer {}", self.token))?;
        authorization.set_sensitive(true);
        headers.insert(metadata::AUTHORIZATION, authorization);
        headers.insert(metadata::DEVICE, ascii(self.device.to_string())?);
        headers.insert(metadata::NONCE, ascii(nonce)?);
        headers.insert(metadata::SIGNED_AT, ascii(signed_at.to_string())?);
        headers.insert_bin(
            metadata::PROOF,
            tonic::metadata::BinaryMetadataValue::from_bytes(&signature),
        );
        Ok(request)
    }
}

pub(super) type SignedCloud = CloudServiceClient<InterceptedService<Channel, DeviceProof>>;

/// Why the device's own Cloud client could not be made.
#[derive(Debug, thiserror::Error)]
pub(crate) enum DeviceRpcError {
    #[error("this device is not signed in to the hub")]
    NotSignedIn,
    #[error("the stored device key is not the key of the signed-in device")]
    OtherKey,
    #[error("the access token could not be refreshed")]
    Refresh(#[source] ClientError),
}

/// The client of one signed-in device and its key.
pub(super) struct DeviceRpc {
    client: HubClient,
    key: Arc<DeviceKey>,
}
impl DeviceRpc {
    /// `key` must be the key the client signs with.
    pub(super) fn new(client: HubClient, key: DeviceKey) -> Result<Self, DeviceRpcError> {
        let device = client
            .cloud_device()
            .map_err(|_| DeviceRpcError::NotSignedIn)?;
        if device.signing_key() != key.public_key() {
            return Err(DeviceRpcError::OtherKey);
        }
        Ok(Self {
            client,
            key: Arc::new(key),
        })
    }
    pub(super) fn client(&self) -> &HubClient {
        &self.client
    }
    /// A signed Cloud client with the current access token, refreshed first when it expires.
    pub(super) async fn signed(&self) -> Result<SignedCloud, DeviceRpcError> {
        self.client
            .ensure_fresh()
            .await
            .map_err(DeviceRpcError::Refresh)?;
        let session = self.client.session().ok_or(DeviceRpcError::NotSignedIn)?;
        let proof = DeviceProof {
            hub_id: Arc::from(self.client.info().hub_id.as_str()),
            key: self.key.clone(),
            token: Arc::from(session.access_token.as_str()),
            device: Arc::from(session.device_id.as_str()),
        };
        Ok(
            CloudServiceClient::new(InterceptedService::new(self.client.channel(), proof))
                .max_decoding_message_size(MAX_RELAY_WIRE_BYTES)
                .max_encoding_message_size(MAX_RELAY_WIRE_BYTES),
        )
    }
    /// The unsigned client of the public `Capabilities` call.
    pub(super) fn public(&self) -> CloudServiceClient<Channel> {
        CloudServiceClient::new(self.client.channel())
            .max_decoding_message_size(MAX_RELAY_WIRE_BYTES)
            .max_encoding_message_size(MAX_RELAY_WIRE_BYTES)
    }
}
