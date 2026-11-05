//! The effects of the device core, performed over the hub: Store, Cloud and agent calls and
//! signatures. Each effect becomes a stream of answers and frames; nothing here decides what a
//! flow does with them.
use super::rpc::DeviceRpc;
use futures::stream::{self, BoxStream, StreamExt as _};
use nocterm_cloud_device::command::Publication;
use nocterm_cloud_device::flow::{
    CollectionEntry, Effect, EffectFailure, Frame, Input, InstanceEntry, InstanceState,
    ObjectEntry, ProbeRequest, RecordHeader, StreamKind, StreamSelector,
};
use nocterm_hub_client::agents::AgentError;
use nocterm_hub_client::{ClientError, StatusCode};
use nocterm_hub_proto::v1;
use std::sync::Arc;
use std::time::Duration;

/// One answer or frame of an effect.
#[derive(Debug)]
pub(crate) enum Event {
    Input(Input),
    Frame(Frame),
}

/// Performs effects. Implemented over the hub, and by scripted fakes in tests.
pub(crate) trait EffectRunner: Send + Sync {
    fn run(&self, effect: Effect) -> BoxStream<'static, Event>;
}

/// The signed Store client of the device.
type Store = nocterm_hub_grpc::store_service_client::StoreServiceClient<nocterm_hub_client::Signed>;

/// The effects of one signed-in device on its hub.
pub(crate) struct HubEffects {
    rpc: Arc<DeviceRpc>,
}
impl HubEffects {
    pub(super) fn new(rpc: Arc<DeviceRpc>) -> Self {
        Self { rpc }
    }
}

fn failed(failure: EffectFailure) -> Event {
    Event::Input(Input::Failed(failure))
}
fn one(event: Event) -> BoxStream<'static, Event> {
    stream::once(async move { event }).boxed()
}

/// How a status answer fails an effect. `sent` says the request may have reached the hub.
pub(crate) fn classify(code: tonic::Code, sent: bool) -> EffectFailure {
    use tonic::Code;
    match code {
        Code::NotFound => EffectFailure::NotFound,
        Code::AlreadyExists => EffectFailure::AlreadyExists,
        Code::FailedPrecondition | Code::Aborted => EffectFailure::Conflict,
        Code::Unavailable | Code::DeadlineExceeded if !sent => EffectFailure::Unavailable,
        Code::Unavailable | Code::DeadlineExceeded | Code::Cancelled | Code::Unknown => {
            if sent {
                EffectFailure::Unknown
            } else {
                EffectFailure::Unavailable
            }
        }
        _ => EffectFailure::Refused,
    }
}
fn client_failure(error: &ClientError) -> EffectFailure {
    match error {
        ClientError::Status { code, .. } => match code {
            StatusCode::NotFound => EffectFailure::NotFound,
            StatusCode::AlreadyExists => EffectFailure::AlreadyExists,
            StatusCode::FailedPrecondition | StatusCode::Aborted => EffectFailure::Conflict,
            StatusCode::Unavailable | StatusCode::DeadlineExceeded => EffectFailure::Unavailable,
            _ => EffectFailure::Refused,
        },
        ClientError::Connect(_) => EffectFailure::Unavailable,
        _ => EffectFailure::Refused,
    }
}
fn agent_failure(error: &AgentError) -> EffectFailure {
    match error {
        AgentError::Hub(error) => client_failure(error),
        _ => EffectFailure::Refused,
    }
}

fn wire_selector(selector: &StreamSelector) -> v1::CloudSelector {
    let stream = match selector.stream {
        StreamKind::Inbox => v1::cloud_selector::Stream::Inbox(v1::CloudRunnerInbox {}),
        StreamKind::Chat(chat) => v1::cloud_selector::Stream::ChatId(chat.to_vec()),
        StreamKind::Directory => v1::cloud_selector::Stream::Directory(v1::CloudRunnerDirectory {}),
    };
    v1::CloudSelector {
        runner_id: selector.runner_id.to_vec(),
        runner_key_epoch: selector.runner_key_epoch,
        stream: Some(stream),
    }
}

/// A replay frame of the hub as the core's frame; `None` for a frame without content.
pub(crate) fn replay_frame(frame: v1::CloudReplayFrame) -> Option<Frame> {
    use v1::cloud_replay_frame::Frame as Wire;
    Some(match frame.frame? {
        Wire::Header(record) => {
            let publication = record.publication.unwrap_or_default();
            let origin = record.origin.unwrap_or_default();
            let runner = match origin.issuer {
                Some(v1::cloud_publication_origin::Issuer::RunnerId(runner)) => Some(runner),
                _ => None,
            };
            Frame::Header(RecordHeader {
                publication_id: publication.publication_id,
                binding: publication.binding,
                total_bytes: publication.total_bytes,
                ciphertext_sha256: publication.ciphertext_sha256,
                cursor: record.cursor,
                origin_account_id: origin.account_id,
                origin_signing_key: origin.signing_key,
                origin_runner_id: runner,
            })
        }
        Wire::Data(bytes) => Frame::Data(bytes),
        Wire::EndRecord(end) => Frame::End {
            publication_id: end.publication_id,
        },
        Wire::PageEnd(end) => Frame::PageEnd {
            through_cursor: end.through_cursor,
            has_more: end.has_more,
        },
        Wire::Gap(gap) => Frame::Gap {
            retained_from: gap.retained_from_cursor,
            snapshot_publication_id: gap.snapshot_publication_id,
        },
    })
}

fn instance(wire: &v1::Instance) -> InstanceEntry {
    let state = match v1::InstanceState::try_from(wire.state) {
        Ok(v1::InstanceState::Provisioning) => InstanceState::Provisioning,
        Ok(v1::InstanceState::AwaitingEnrolment) => InstanceState::AwaitingEnrolment,
        Ok(v1::InstanceState::Ready) => InstanceState::Ready,
        Ok(v1::InstanceState::Suspended) => InstanceState::Suspended,
        Ok(v1::InstanceState::Deleting) => InstanceState::Deleting,
        Ok(v1::InstanceState::Failed) => InstanceState::Failed,
        _ => InstanceState::Unknown,
    };
    InstanceEntry {
        instance_id: wire.instance_id.clone(),
        agent_id: wire.agent_id.clone(),
        pool_id: wire.pool_id.clone(),
        runner_id: wire.runner_id.as_slice().try_into().ok(),
        state,
    }
}

/// The seat of `agent` that hosts chats: enrolled and not on its way out.
fn hosting(instances: &[v1::Instance]) -> Option<&v1::Instance> {
    instances
        .iter()
        .find(|seat| seat.runner_id.len() == 16 && seat.state != v1::InstanceState::Deleting as i32)
}

impl HubEffects {
    async fn unary(rpc: Arc<DeviceRpc>, effect: Effect) -> Event {
        match effect {
            Effect::Sign(request) => match rpc.client().sign_cloud(&request) {
                Ok(signature) => Event::Input(Input::Signature(signature.to_vec())),
                Err(_) => failed(EffectFailure::Refused),
            },
            Effect::Capabilities { nonce } => {
                let request = v1::CloudCapabilitiesRequest {
                    nonce: nonce.to_vec(),
                };
                match rpc.public().capabilities(request).await {
                    Ok(answer) => {
                        let answer = answer.into_inner();
                        Event::Input(Input::Capabilities {
                            claim: answer.claim,
                            signature: answer.signature,
                        })
                    }
                    Err(status) => failed(classify(status.code(), false)),
                }
            }
            Effect::Probe(request) => Self::probe(rpc, request).await,
            Effect::Publish(publication) => Self::publish(rpc, publication).await,
            Effect::RunnerStatus { runner_id } => {
                let mut signed = match rpc.signed().await {
                    Ok(signed) => signed,
                    Err(_) => return failed(EffectFailure::Unavailable),
                };
                let request = v1::CloudRunnerRef {
                    runner_id: runner_id.to_vec(),
                };
                match signed.runner_status(request).await {
                    Ok(status) => Event::Input(Input::RunnerStatus {
                        signed_status: status.into_inner().signed_status,
                    }),
                    Err(status) => failed(classify(status.code(), false)),
                }
            }
            Effect::ListInstances => match rpc.client().agents().list_instances(None).await {
                Ok(listed) => Event::Input(Input::Instances(listed.iter().map(instance).collect())),
                Err(AgentError::Unsupported) => Event::Input(Input::Instances(Vec::new())),
                Err(error) => failed(agent_failure(&error)),
            },
            Effect::StartInstance {
                agent_id, chat_id, ..
            } => Self::start(rpc, agent_id, chat_id).await,
            store => Self::store(rpc, store).await,
        }
    }
    async fn probe(rpc: Arc<DeviceRpc>, request: ProbeRequest) -> Event {
        let wait = request
            .expires_at_ms
            .saturating_sub(nocterm_hub_client::now_ms())
            .clamp(0, 10_000);
        let mut signed = match rpc.signed().await {
            Ok(signed) => signed,
            Err(_) => return failed(EffectFailure::Unavailable),
        };
        let wire = v1::CloudHeadProbe {
            scope: request.scope,
            probe_id: request.probe_id.to_vec(),
            ciphertext: request.ciphertext,
            expires_at_ms: request.expires_at_ms,
        };
        let call = signed.probe_head(wire);
        let wait = Duration::from_millis(u64::try_from(wait).unwrap_or(0));
        match tokio::time::timeout(wait, call).await {
            Ok(Ok(reply)) => {
                let reply = reply.into_inner();
                Event::Input(Input::ProbeReply {
                    scope: reply.scope,
                    probe_id: reply.probe_id,
                    ciphertext: reply.ciphertext,
                })
            }
            // Nobody answered in time: a sleeping runner, as the core expects it.
            Ok(Err(_)) | Err(_) => failed(EffectFailure::Unavailable),
        }
    }
    async fn publish(rpc: Arc<DeviceRpc>, publication: Publication) -> Event {
        let Ok(binding) = publication.binding() else {
            return failed(EffectFailure::Refused);
        };
        let mut signed = match rpc.signed().await {
            Ok(signed) => signed,
            Err(_) => return failed(EffectFailure::Unavailable),
        };
        let wire = v1::CloudPublishOne {
            header: Some(v1::CloudPublication {
                publication_id: publication.publication_id().to_vec(),
                binding,
                total_bytes: publication.total_bytes(),
                ciphertext_sha256: publication.ciphertext_sha256().to_vec(),
                retry_until_ms: 0,
            }),
            data: publication.ciphertext().to_vec(),
        };
        match signed.publish_one(wire).await {
            Ok(receipt) => {
                let receipt = receipt.into_inner();
                Event::Input(Input::Receipt {
                    publication_id: receipt.publication_id,
                    cursor: receipt.cursor,
                    ciphertext_sha256: receipt.ciphertext_sha256,
                })
            }
            Err(status) => failed(classify(status.code(), true)),
        }
    }
    async fn start(rpc: Arc<DeviceRpc>, agent_id: String, chat_id: Option<String>) -> Event {
        let (Ok(agent), chat) = (agent_id.parse::<nocterm_hub_model::AgentId>(), chat_id) else {
            return failed(EffectFailure::Refused);
        };
        let agents = rpc.client().agents();
        // A chat never provisions a seat: without an enrolled one the agent must be started
        // first, which also enrols its runner (ADR 0122).
        let listed = match agents.list_instances(Some(agent)).await {
            Ok(listed) => listed,
            Err(error) => return failed(agent_failure(&error)),
        };
        let Some(seat) = hosting(&listed) else {
            return failed(EffectFailure::NotFound);
        };
        let pool = seat.pool_id.parse::<nocterm_hub_model::PoolId>().ok();
        let started = match chat.as_deref().map(chat_bytes) {
            Some(Some(chat)) => agents.start_instance_for_chat(agent, pool, chat).await,
            Some(None) => return failed(EffectFailure::Refused),
            None => agents.start_instance(agent, pool).await,
        };
        match started {
            Ok(started) => {
                let mut entry = instance(&started);
                entry.runner_id = entry
                    .runner_id
                    .or_else(|| seat.runner_id.as_slice().try_into().ok());
                Event::Input(Input::Instance(entry))
            }
            Err(error) => failed(agent_failure(&error)),
        }
    }
    async fn store(rpc: Arc<DeviceRpc>, effect: Effect) -> Event {
        if rpc.client().ensure_fresh().await.is_err() {
            return failed(EffectFailure::Unavailable);
        }
        let store = rpc.client().store();
        let answer = match effect {
            Effect::ListCollections { kind } => Self::collections(store, kind).await,
            Effect::GetCollectionKey { collection_id } => {
                Self::collection_key(store, collection_id).await
            }
            Effect::GetObject {
                collection_id,
                object_id,
            } => Self::object(store, collection_id, object_id).await,
            Effect::PutObject {
                collection_id,
                object_id,
                kind,
                expected_version,
                ciphertext,
            } => {
                let request = v1::PutObjectRequest {
                    collection_id,
                    object_id,
                    expected_version,
                    kind,
                    ciphertext,
                };
                Self::put(store, request).await
            }
            Effect::CreateCollection {
                collection_id,
                kind,
                wrapped_key,
                sealed_name,
            } => {
                let request = v1::CreateCollectionRequest {
                    org_id: String::new(),
                    kind,
                    sealed_name,
                    wrapped_key,
                    collection_id,
                };
                Self::create(store, request).await
            }
            _ => return failed(EffectFailure::Refused),
        };
        answer.unwrap_or_else(|status| failed(classify(status.code(), false)))
    }
    async fn collections(mut store: Store, kind: String) -> Result<Event, tonic::Status> {
        let request = v1::ListCollectionsRequest {
            org_id: String::new(),
            kind,
        };
        let list = store.list_collections(request).await?.into_inner();
        Ok(Event::Input(Input::Collections(
            list.collections
                .into_iter()
                .map(|row| CollectionEntry {
                    collection_id: row.collection_id,
                    kind: row.kind,
                    personal: row.personal && row.org_id.is_empty(),
                    deleted: row.deleted,
                })
                .collect(),
        )))
    }
    async fn collection_key(
        mut store: Store,
        collection_id: String,
    ) -> Result<Event, tonic::Status> {
        let request = v1::GetCollectionKeyRequest { collection_id };
        let key = store.get_collection_key(request).await?.into_inner();
        Ok(Event::Input(Input::CollectionKey {
            wrapped_key: key.wrapped_key,
            granted_by: key.granted_by,
        }))
    }
    async fn object(
        mut store: Store,
        collection_id: String,
        object_id: String,
    ) -> Result<Event, tonic::Status> {
        let request = v1::GetObjectRequest {
            collection_id,
            object_id,
        };
        let object = store.get_object(request).await?.into_inner();
        Ok(Event::Input(Input::Object(ObjectEntry {
            collection_id: object.collection_id,
            object_id: object.object_id,
            kind: object.kind,
            version: object.version,
            deleted: object.deleted,
            ciphertext: object.ciphertext,
        })))
    }
    async fn put(mut store: Store, request: v1::PutObjectRequest) -> Result<Event, tonic::Status> {
        let version = store.put_object(request).await?.into_inner().version;
        Ok(Event::Input(Input::Stored { version }))
    }
    async fn create(
        mut store: Store,
        request: v1::CreateCollectionRequest,
    ) -> Result<Event, tonic::Status> {
        store.create_collection(request).await?;
        Ok(Event::Input(Input::Done))
    }
    fn replay(rpc: Arc<DeviceRpc>, request: v1::CloudReplayRequest) -> BoxStream<'static, Event> {
        stream::once(async move {
            let mut signed = match rpc.signed().await {
                Ok(signed) => signed,
                Err(_) => return one(failed(EffectFailure::Unavailable)),
            };
            match signed.replay(request).await {
                Ok(frames) => frames
                    .into_inner()
                    .map(|frame| match frame {
                        Ok(frame) => {
                            replay_frame(frame).map_or(failed(EffectFailure::Refused), Event::Frame)
                        }
                        Err(status) => failed(classify(status.code(), false)),
                    })
                    .chain(stream::once(async { failed(EffectFailure::Ended) }))
                    .boxed(),
                Err(status) => one(failed(classify(status.code(), false))),
            }
        })
        .flatten()
        .boxed()
    }
    fn watch(rpc: Arc<DeviceRpc>, request: v1::CloudWatchRequest) -> BoxStream<'static, Event> {
        stream::once(async move {
            let mut signed = match rpc.signed().await {
                Ok(signed) => signed,
                Err(_) => return one(failed(EffectFailure::Unavailable)),
            };
            match signed.watch(request).await {
                Ok(hints) => hints
                    .into_inner()
                    .map(|hint| match hint {
                        Ok(hint) => Event::Frame(Frame::Hint {
                            head_cursor: hint.head_cursor,
                        }),
                        Err(status) => failed(classify(status.code(), false)),
                    })
                    .chain(stream::once(async { failed(EffectFailure::Ended) }))
                    .boxed(),
                Err(status) => one(failed(classify(status.code(), false))),
            }
        })
        .flatten()
        .boxed()
    }
}

fn chat_bytes(hex: &str) -> Option<[u8; 16]> {
    if hex.len() != 32 {
        return None;
    }
    let mut bytes = [0u8; 16];
    for (index, byte) in bytes.iter_mut().enumerate() {
        *byte = u8::from_str_radix(hex.get(index * 2..index * 2 + 2)?, 16).ok()?;
    }
    Some(bytes)
}

impl EffectRunner for HubEffects {
    fn run(&self, effect: Effect) -> BoxStream<'static, Event> {
        let rpc = self.rpc.clone();
        match effect {
            Effect::Replay {
                selector,
                after_cursor,
                max_records,
                max_bytes,
            } => Self::replay(
                rpc,
                v1::CloudReplayRequest {
                    selector: Some(wire_selector(&selector)),
                    after_cursor,
                    max_records,
                    max_bytes,
                },
            ),
            Effect::Watch {
                selector,
                after_cursor,
            } => Self::watch(
                rpc,
                v1::CloudWatchRequest {
                    selector: Some(wire_selector(&selector)),
                    after_cursor,
                },
            ),
            unary => stream::once(Self::unary(rpc, unary)).boxed(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_status_fails_the_effect_with_what_it_means_for_the_flow() {
        assert_eq!(
            classify(tonic::Code::NotFound, false),
            EffectFailure::NotFound
        );
        assert_eq!(
            classify(tonic::Code::Aborted, false),
            EffectFailure::Conflict
        );
        assert_eq!(
            classify(tonic::Code::Unavailable, false),
            EffectFailure::Unavailable
        );
        // A publication that may have reached the hub is of unknown outcome, not unsent.
        assert_eq!(
            classify(tonic::Code::Unavailable, true),
            EffectFailure::Unknown
        );
        assert_eq!(
            classify(tonic::Code::PermissionDenied, true),
            EffectFailure::Refused
        );
    }

    #[test]
    fn a_replay_record_header_keeps_the_runner_that_published_it() {
        let wire = v1::CloudReplayFrame {
            frame: Some(v1::cloud_replay_frame::Frame::Header(
                v1::CloudDeliveredRecord {
                    publication: Some(v1::CloudPublication {
                        publication_id: vec![1; 16],
                        binding: vec![2],
                        total_bytes: 3,
                        ciphertext_sha256: vec![4; 32],
                        retry_until_ms: 0,
                    }),
                    cursor: 9,
                    origin: Some(v1::CloudPublicationOrigin {
                        account_id: "acct".into(),
                        signing_key: vec![5; 32],
                        issuer: Some(v1::cloud_publication_origin::Issuer::RunnerId(vec![6; 16])),
                    }),
                },
            )),
        };
        let Some(Frame::Header(header)) = replay_frame(wire) else {
            panic!("a header frame");
        };
        assert_eq!(header.cursor, 9);
        assert_eq!(header.origin_runner_id, Some(vec![6; 16]));
        assert_eq!(chat_bytes(&"ab".repeat(16)), Some([0xab; 16]));
        assert_eq!(chat_bytes("xyz"), None);
    }
}
