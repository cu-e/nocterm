//! The effects of the device core, performed over the hub: the client's Cloud calls, Store and
//! agent calls and signatures. Each effect becomes a stream of answers and frames; nothing here
//! decides what a flow does with them.
//!
//! The Cloud effects map one to one onto `HubClient`'s device calls (contract ADR 0010): the
//! client refreshes its token and signs each request with the device key it holds, which never
//! leaves `nocterm-hub-client`; a failed call answers its effect with `Input::from(error)`.
use futures::stream::{self, BoxStream, StreamExt as _};
use nocterm_cloud_device::flow::{
    CollectionEntry, Effect, EffectFailure, Input, InstanceEntry, InstanceState, ObjectEntry,
};
use nocterm_hub_client::agents::AgentError;
use nocterm_hub_client::{ClientError, CloudFrames, HubClient, StatusCode};
use nocterm_hub_proto::v1;

/// One answer or frame of an effect.
#[derive(Debug)]
pub(crate) enum Event {
    Input(Input),
    Frame(nocterm_cloud_device::flow::Frame),
}

/// Performs effects. Implemented over the hub, and by scripted fakes in tests.
pub(crate) trait EffectRunner: Send + Sync {
    fn run(&self, effect: Effect) -> BoxStream<'static, Event>;
}

/// The effects of one signed-in device on its hub.
pub(crate) struct HubEffects {
    client: HubClient,
}
impl HubEffects {
    pub(super) fn new(client: HubClient) -> Self {
        Self { client }
    }
}

fn failed(failure: EffectFailure) -> Event {
    Event::Input(Input::Failed(failure))
}
fn answered(answer: Result<Input, nocterm_hub_client::CloudCallError>) -> Event {
    Event::Input(answer.unwrap_or_else(Input::from))
}

/// How a Store status fails an effect; nothing of a Store call is of unknown outcome.
fn store_failure(code: tonic::Code) -> EffectFailure {
    use tonic::Code;
    match code {
        Code::NotFound => EffectFailure::NotFound,
        Code::AlreadyExists => EffectFailure::AlreadyExists,
        Code::FailedPrecondition | Code::Aborted => EffectFailure::Conflict,
        Code::Unavailable | Code::DeadlineExceeded | Code::Cancelled | Code::Unknown => {
            EffectFailure::Unavailable
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

/// The frames of a replay or watch; a stream that ends without its last frame answers `Ended`.
fn frames(
    opened: Result<CloudFrames, nocterm_hub_client::CloudCallError>,
) -> BoxStream<'static, Event> {
    match opened {
        Ok(frames) => frames
            .map(|frame| match frame {
                Ok(frame) => Event::Frame(frame),
                Err(error) => Event::Input(Input::from(error)),
            })
            .chain(stream::once(async { failed(EffectFailure::Ended) }))
            .boxed(),
        Err(error) => stream::once(async move { Event::Input(Input::from(error)) }).boxed(),
    }
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

impl HubEffects {
    async fn unary(client: HubClient, effect: Effect) -> Event {
        match effect {
            Effect::Sign(request) => match client.sign_cloud(&request) {
                Ok(signature) => Event::Input(Input::Signature(signature.to_vec())),
                Err(_) => failed(EffectFailure::Refused),
            },
            Effect::Capabilities { nonce } => answered(client.capabilities(nonce).await),
            Effect::Probe(request) => answered(client.probe_head(&request).await),
            Effect::Publish(publication) => answered(client.publish(&publication).await),
            Effect::RunnerStatus { runner_id } => answered(client.runner_status(runner_id).await),
            Effect::ListInstances => match client.agents().list_instances(None).await {
                Ok(listed) => Event::Input(Input::Instances(listed.iter().map(instance).collect())),
                Err(AgentError::Unsupported) => Event::Input(Input::Instances(Vec::new())),
                Err(error) => failed(agent_failure(&error)),
            },
            Effect::StartInstance {
                agent_id,
                pool_id,
                chat_id,
            } => Self::start(client, &agent_id, &pool_id, chat_id.as_deref()).await,
            store => Self::store(client, store).await,
        }
    }
    /// Starts exactly the seat the core named: the agent's pool, for the chat when one is named.
    async fn start(client: HubClient, agent: &str, pool: &str, chat: Option<&str>) -> Event {
        let (Ok(agent), Ok(pool)) = (
            agent.parse::<nocterm_hub_model::AgentId>(),
            pool.parse::<nocterm_hub_model::PoolId>(),
        ) else {
            return failed(EffectFailure::Refused);
        };
        let agents = client.agents();
        let started = match chat.map(chat_bytes) {
            Some(Some(chat)) => {
                agents
                    .start_instance_for_chat(agent, Some(pool), chat)
                    .await
            }
            Some(None) => return failed(EffectFailure::Refused),
            None => agents.start_instance(agent, Some(pool)).await,
        };
        match started {
            Ok(started) => Event::Input(Input::Instance(instance(&started))),
            Err(error) => failed(agent_failure(&error)),
        }
    }
    async fn store(client: HubClient, effect: Effect) -> Event {
        if client.ensure_fresh().await.is_err() {
            return failed(EffectFailure::Unavailable);
        }
        let answer = match effect {
            Effect::ListCollections { kind } => list_collections(&client, kind).await,
            Effect::GetCollectionKey { collection_id } => {
                collection_key(&client, collection_id).await
            }
            Effect::GetObject {
                collection_id,
                object_id,
            } => {
                let request = v1::GetObjectRequest {
                    collection_id,
                    object_id,
                };
                get_object(&client, request).await
            }
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
                put_object(&client, request).await
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
                create_collection(&client, request).await
            }
            _ => return failed(EffectFailure::Refused),
        };
        answer.map_or_else(|status| failed(store_failure(status.code())), Event::Input)
    }
}

type StoreAnswer = Result<Input, tonic::Status>;

async fn list_collections(client: &HubClient, kind: String) -> StoreAnswer {
    let request = v1::ListCollectionsRequest {
        org_id: String::new(),
        kind,
    };
    let list = client.store().list_collections(request).await?;
    Ok(Input::Collections(collections(list.into_inner())))
}
async fn collection_key(client: &HubClient, collection_id: String) -> StoreAnswer {
    let request = v1::GetCollectionKeyRequest { collection_id };
    let key = client
        .store()
        .get_collection_key(request)
        .await?
        .into_inner();
    Ok(Input::CollectionKey {
        wrapped_key: key.wrapped_key,
        granted_by: key.granted_by,
    })
}
async fn get_object(client: &HubClient, request: v1::GetObjectRequest) -> StoreAnswer {
    let object = client.store().get_object(request).await?;
    Ok(Input::Object(object_entry(object.into_inner())))
}
async fn put_object(client: &HubClient, request: v1::PutObjectRequest) -> StoreAnswer {
    let stored = client.store().put_object(request).await?;
    Ok(Input::Stored {
        version: stored.into_inner().version,
    })
}
async fn create_collection(
    client: &HubClient,
    request: v1::CreateCollectionRequest,
) -> StoreAnswer {
    client.store().create_collection(request).await?;
    Ok(Input::Done)
}

fn collections(list: v1::CollectionList) -> Vec<CollectionEntry> {
    list.collections
        .into_iter()
        .map(|row| CollectionEntry {
            collection_id: row.collection_id,
            kind: row.kind,
            personal: row.personal && row.org_id.is_empty(),
            deleted: row.deleted,
        })
        .collect()
}

fn object_entry(object: v1::StoredObject) -> ObjectEntry {
    ObjectEntry {
        collection_id: object.collection_id,
        object_id: object.object_id,
        kind: object.kind,
        version: object.version,
        deleted: object.deleted,
        ciphertext: object.ciphertext,
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
        let client = self.client.clone();
        match effect {
            Effect::Replay {
                selector,
                after_cursor,
                max_records,
                max_bytes,
            } => stream::once(async move {
                frames(
                    client
                        .replay(selector, after_cursor, max_records, max_bytes)
                        .await,
                )
            })
            .flatten()
            .boxed(),
            Effect::Watch {
                selector,
                after_cursor,
            } => stream::once(async move { frames(client.watch(selector, after_cursor).await) })
                .flatten()
                .boxed(),
            unary => stream::once(Self::unary(client, unary)).boxed(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_store_status_fails_the_effect_with_what_it_means_for_the_flow() {
        assert_eq!(
            store_failure(tonic::Code::NotFound),
            EffectFailure::NotFound
        );
        assert_eq!(store_failure(tonic::Code::Aborted), EffectFailure::Conflict);
        assert_eq!(
            store_failure(tonic::Code::Unavailable),
            EffectFailure::Unavailable
        );
        assert_eq!(
            store_failure(tonic::Code::PermissionDenied),
            EffectFailure::Refused
        );
    }

    #[test]
    fn a_seat_keeps_its_pool_and_a_chat_id_reads_from_its_hex() {
        let wire = v1::Instance {
            instance_id: "seat".into(),
            agent_id: "agent".into(),
            pool_id: "pool".into(),
            runner_id: vec![6; 16],
            state: v1::InstanceState::Suspended as i32,
            ..v1::Instance::default()
        };
        let entry = instance(&wire);
        assert_eq!(entry.pool_id, "pool");
        assert_eq!(entry.runner_id, Some([6; 16]));
        assert_eq!(entry.state, InstanceState::Suspended);
        assert_eq!(chat_bytes(&"ab".repeat(16)), Some([0xab; 16]));
        assert_eq!(chat_bytes("xyz"), None);
    }
}
