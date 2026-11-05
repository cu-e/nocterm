//! The real TLS relay carries the original desktop command to an enrolled test runner.
//! This runner claims material, opens the account's provider lease and witnesses empty chats;
//! it performs no inference or tools.
use super::super::{Hub, hub, links, sign_in};
use nocterm_cloud_agents_model::{
    AgentId, ChatHost, ChatRunner, CloudChatLauncher as _, KeyInput, PersonalAgentDraft,
    PersonalAgents as _, ProviderKind, SaveOutcome,
};
use nocterm_cloud_conversation_model::{
    CloudConversation, ConversationAction, ImageMime, ModelName, PromptImage,
};
use nocterm_hub_client::{
    DeviceInfo, HubClient, HubEndpoint,
    cloud::{CloudClient, CloudReplayItem, CloudRunner, CloudSelector},
};
use nocterm_hub_io::{ChatServices, CloudAgentsLink, CloudChatsLink, HubLinks, cloud::enrol};
use nocterm_hub_model::cloud::{
    ChatCoordinate, CloudRoute, CloudTextId, RunnerCoordinate,
    head::{ChatHead, HeadProbeKeys, HeadProbeScope, HeadTarget, HeadValue, HeadWitness},
    payload::{CloudCommandPayload, GrantPayload, ImageMime as WireMime, PromptPart},
    provider::{BootDescriptor, BootWindow},
    runner_auth::RunnerProofSubject,
};
use nocterm_keys::{
    DeviceKey, PublicKey,
    cloud::{
        RunnerBootKey, RunnerEncryptionKey, RunnerSigningKey, open_chat_key_from_account,
        open_command, seal_head_witness,
    },
};
use sha2::{Digest as _, Sha256};
use std::{
    error::Error,
    sync::{Arc, Mutex},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

type TestResult<T = ()> = Result<T, Box<dyn Error + Send + Sync>>;

// Deterministic generated RGBA noise exercises the original 16 KiB relay-frame boundary.
const IMAGE: &[u8] = include_bytes!("../../fixtures/composer-noise.png");

struct Runner {
    transport: CloudRunner,
    scope: HeadProbeScope,
    encryption: RunnerEncryptionKey,
    signing: RunnerSigningKey,
    boot: RunnerBootKey,
    claimed: Mutex<Option<ChatCoordinate>>,
    leased: Mutex<Option<Vec<String>>>,
}

impl Runner {
    async fn start(
        hub: &Hub,
        account: &nocterm_hub_io::HubAccount,
    ) -> TestResult<(Arc<Self>, HubClient, CloudClient)> {
        let authority = std::env::var("NOCTERM_CLOUD_AUTHORITY")?;
        let authority = nocterm_hub_client::parse_fingerprint(&authority)?;
        let endpoint =
            HubEndpoint::new(&hub.address.to_string(), Some(&hub.fingerprint.to_string()))?;
        let device = HubClient::connect(&endpoint, DeviceKey::generate()?, None).await?;
        let signed = device
            .password_login(
                &hub.username,
                &hub.password,
                &DeviceInfo::new("test", "composer transport"),
            )
            .await?;
        let keyset = device
            .open_keyset(&signed.session.account_id, signed.secrets.unlock())
            .await?
            .ok_or("native sign-in did not create the account keys")?;
        let cloud = device.cloud_offer().await?.accept(&authority)?;
        let encryption = RunnerEncryptionKey::generate()?;
        let signing = RunnerSigningKey::generate()?;
        let proof = DeviceKey::generate()?;
        let runner = RunnerCoordinate {
            id: *uuid::Uuid::new_v4().as_bytes(),
            key_epoch: 1,
        };
        let scope = HeadProbeScope {
            namespace: cloud.namespace().clone(),
            runner,
            keys: HeadProbeKeys {
                account: keyset.public_key().to_bytes(),
                runner_encryption: encryption.public_key().to_bytes(),
                runner_signing: signing.public_key(),
            },
        };
        let document = setup_document(hub, &device, &scope, authority, proof.public_key());
        let ticket = enrol(
            account,
            tokio::runtime::Handle::current(),
            document.as_bytes(),
            "Composer transport test".into(),
        )
        .await?;
        let subject = RunnerProofSubject {
            raw_hub_id: CloudTextId::new(device.info().hub_id.clone())?,
            namespace: scope.namespace.clone(),
            runner_id: runner.id,
            proof_key: proof.public_key(),
        };
        let transport = CloudRunner::redeem(&endpoint, subject, authority, proof, ticket).await?;
        let peer = Arc::new(Self {
            transport,
            scope,
            encryption,
            signing,
            boot: RunnerBootKey::generate()?,
            claimed: Mutex::new(None),
            leased: Mutex::new(None),
        });
        peer.announce().await?;
        Ok((peer, device, cloud))
    }

    async fn announce(&self) -> TestResult {
        let now = now_ms();
        let boot = BootDescriptor::new(
            self.scope.namespace.clone(),
            self.scope.runner,
            *uuid::Uuid::new_v4().as_bytes(),
            self.boot.public_key().to_bytes(),
            BootWindow {
                issued_at_ms: now,
                expires_at_ms: now + 600_000,
            },
        )?;
        self.transport
            .heartbeat(self.signing.attest_boot(boot)?.canonical_bytes()?)
            .await?;
        Ok(())
    }

    async fn heads(&self) -> TestResult {
        let probes = self.transport.head_probes(self.scope.clone())?;
        loop {
            let Some(incoming) = probes.next(Duration::from_secs(1)).await? else {
                continue;
            };
            let query = incoming.open(&self.encryption, &self.scope, now_ms())?;
            let HeadTarget::Chat {
                chat_id,
                chat_key_epoch,
            } = &query.query().target
            else {
                continue;
            };
            let (chat_id, chat_key_epoch) = (*chat_id, *chat_key_epoch);
            let claimed = self.claimed.lock().unwrap().clone();
            if !claimed
                .is_some_and(|chat| chat.chat_id() == chat_id && chat.key_epoch() == chat_key_epoch)
            {
                continue;
            }
            let witness = HeadWitness {
                query: query.query().clone(),
                observed_at_ms: now_ms(),
                value: HeadValue::Chat(Box::new(ChatHead {
                    chat_id,
                    chat_key_epoch,
                    current_writer: None,
                    control_revision: 0,
                    deleted: false,
                    head: None,
                    retained_from: 0,
                    snapshot: None,
                })),
            };
            incoming
                .reply(seal_head_witness(
                    &self.encryption,
                    &self.signing.attest_head(witness)?,
                )?)
                .await?;
        }
    }

    async fn prompts(&self) -> TestResult<Vec<ReceivedPrompt>> {
        let account = PublicKey::from_bytes(&self.scope.keys.account)?;
        let mut cursor = 0;
        let mut prompts = Vec::new();
        loop {
            let mut replay = self
                .transport
                .replay(
                    CloudSelector::RunnerInbox(self.scope.runner),
                    cursor,
                    8,
                    1024 * 1024,
                )
                .await?;
            loop {
                match replay.next_record().await? {
                    CloudReplayItem::Record(mut record) => {
                        cursor = record.cursor();
                        let binding = record.metadata().binding().clone();
                        let mut bytes = Vec::new();
                        while let Some(chunk) = record.next_chunk().await? {
                            bytes.extend(chunk);
                        }
                        if let Some(chat) = binding.material_coordinate() {
                            let _key = open_chat_key_from_account(
                                &self.encryption,
                                &account,
                                chat,
                                &bytes,
                            )?;
                            *self.claimed.lock().unwrap() = Some(chat.clone());
                        } else if let Some(route) = binding.command_route()
                            && let Some(prompt) = self.command(route, &bytes)?
                        {
                            prompts.push(prompt);
                            if prompts.len() == 2 {
                                return Ok(prompts);
                            }
                        }
                    }
                    CloudReplayItem::End { .. } => break,
                    CloudReplayItem::Gap { .. } => {
                        return Err("the fresh runner inbox has a gap".into());
                    }
                }
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }

    fn command(&self, route: &CloudRoute, bytes: &[u8]) -> TestResult<Option<ReceivedPrompt>> {
        let account = PublicKey::from_bytes(&self.scope.keys.account)?;
        let opened = open_command(&self.encryption, &account, route, bytes)?;
        let command = opened.command()?;
        match CloudCommandPayload::from_command(&command)? {
            CloudCommandPayload::IssueGrant(grant) => {
                let GrantPayload::Provider {
                    context,
                    allowed_models,
                    capsule,
                    ..
                } = *grant
                else {
                    return Ok(None);
                };
                let key = self
                    .boot
                    .open_provider_lease(&account, &context, capsule, now_ms())?;
                assert_eq!(key.secret(), b"composer-test-credential");
                assert_eq!(context.provider_reference().as_str(), "openai_chat");
                let models: Vec<_> = allowed_models
                    .iter()
                    .map(|model| model.as_str().to_owned())
                    .collect();
                assert_eq!(models, ["first-model", "second-model"]);
                *self.leased.lock().unwrap() = Some(models);
                Ok(None)
            }
            CloudCommandPayload::SubmitPrompt(prompt) => {
                let prompt = ReceivedPrompt::of(&prompt);
                let leased = self.leased.lock().unwrap();
                assert!(
                    leased
                        .as_ref()
                        .is_some_and(|models| models.contains(&prompt.model)),
                    "a prompt must name a model in the actual signed provider grant"
                );
                assert_eq!(prompt.provider, "openai_chat");
                Ok(Some(prompt))
            }
            _ => Ok(None),
        }
    }
}

fn setup_document(
    hub: &Hub,
    device: &HubClient,
    scope: &HeadProbeScope,
    authority: [u8; 32],
    proof: [u8; 32],
) -> String {
    format!(
        "format = \"nocterm-cloud-runner-setup-v1\"\nhub_address = \"{}\"\nraw_hub_id = \"{}\"\nhub_namespace = \"{}\"\nhub_authority = \"{}\"\naccount_id = \"{}\"\naccount_key_fingerprint = \"{}\"\nrunner = \"{}\"\nrunner_key_epoch = 1\nencryption_key = \"{}\"\nsigning_key = \"{}\"\nproof_key = \"{}\"\n",
        hub.address,
        device.info().hub_id,
        scope.namespace.hub_instance_id.as_str(),
        hex::encode(authority),
        scope.namespace.account_id.as_str(),
        hex::encode(Sha256::digest(scope.keys.account)),
        hex::encode(scope.runner.id),
        hex::encode(scope.keys.runner_encryption),
        hex::encode(scope.keys.runner_signing),
        hex::encode(proof)
    )
}

#[derive(Debug)]
struct ReceivedPrompt {
    provider: String,
    model: String,
    text: String,
    images: Vec<(WireMime, Vec<u8>)>,
}
impl ReceivedPrompt {
    fn of(prompt: &nocterm_hub_model::cloud::payload::PromptPayload<'_>) -> Self {
        Self {
            provider: prompt.model.provider.as_str().to_owned(),
            model: prompt.model.model.as_str().to_owned(),
            text: prompt
                .content
                .iter()
                .filter_map(|part| {
                    if let PromptPart::Text(text) = part {
                        Some(*text)
                    } else {
                        None
                    }
                })
                .collect(),
            images: prompt
                .content
                .iter()
                .filter_map(|part| {
                    if let PromptPart::Image { mime, bytes } = part {
                        Some((*mime, bytes.to_vec()))
                    } else {
                        None
                    }
                })
                .collect(),
        }
    }
}

fn now_ms() -> i64 {
    i64::try_from(
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_millis(),
    )
    .unwrap()
}

fn isolated_account() -> Option<Hub> {
    let mut hub = hub()?;
    let credentials = (
        std::env::var("NOCTERM_HUB_COMPOSER_USERNAME"),
        std::env::var("NOCTERM_HUB_COMPOSER_PASSWORD"),
    );
    let (Ok(username), Ok(password)) = credentials else {
        assert!(
            std::env::var_os("NOCTERM_REQUIRE_HUB").is_none(),
            "the composer transport test requires a disposable NOCTERM_HUB_COMPOSER_USERNAME/PASSWORD account"
        );
        return None;
    };
    hub.username = username;
    hub.password = password;
    Some(hub)
}

async fn saved_agent(
    links: Arc<HubLinks>,
    directory: &std::path::Path,
    hub: &Hub,
) -> TestResult<(CloudAgentsLink, AgentId)> {
    let agents = CloudAgentsLink::new(links, directory.join("pins"));
    let draft = PersonalAgentDraft::new(
        "Composer transport",
        ProviderKind::OpenAiChat,
        "",
        "first-model",
        KeyInput::Replace(nocterm_session::Secret::new("composer-test-credential")),
        false,
        None,
    )?
    .with_models(vec!["first-model".into(), "second-model".into()])?;
    let SaveOutcome::Saved(agent) = agents.save(&hub.id(), None, draft).await? else {
        return Err("an own-key agent must not require pool trust".into());
    };
    Ok((agents, agent))
}

async fn send_prompts(conversation: &Arc<dyn CloudConversation>) -> TestResult {
    assert!(IMAGE.len() > 16 * 1024);
    assert!((128 * 1024..=1024 * 1024).contains(&IMAGE.len()));
    assert!(conversation.can_prompt());
    assert_eq!(
        conversation
            .models()
            .iter()
            .map(|model| model.model.as_str())
            .collect::<Vec<_>>(),
        ["first-model", "second-model"]
    );
    for (text, model, images) in [
        ("first prompt", None, Vec::new()),
        (
            "",
            Some("second-model"),
            vec![PromptImage {
                mime: ImageMime::Png,
                bytes: IMAGE.into(),
            }],
        ),
    ] {
        conversation
            .deliver(ConversationAction::Prompt {
                text: text.into(),
                server: None,
                model: model.map(|model| ModelName {
                    provider: "openai_chat".into(),
                    model: model.into(),
                }),
                images,
            })
            .await?;
    }
    Ok(())
}

#[tokio::test]
async fn an_image_only_prompt_keeps_the_explicit_second_model_through_the_live_hub() -> TestResult {
    let Some(hub) = isolated_account() else {
        return Ok(());
    };
    // A separate account prevents enrollment records from altering other live fixtures.
    let directory = tempfile::tempdir()?;
    let links = links(&directory);
    sign_in(&links, &hub)?;
    let (agents, agent) = saved_agent(links.clone(), directory.path(), &hub).await?;
    let account = links.open_account(&hub.id())?;
    let (runner, device, cloud) = Runner::start(&hub, &account).await?;
    let heads = tokio::spawn({
        let runner = runner.clone();
        async move { runner.heads().await }
    });
    let mut received = tokio::spawn({
        let runner = runner.clone();
        async move { runner.prompts().await }
    });
    let result = tokio::time::timeout(Duration::from_secs(30), async {
        let services = ChatServices::native(&links, directory.path().join("chats"))?;
        let chats = CloudChatsLink::new(links.clone(), services)?;
        let opened = chats
            .create(
                &hub.id(),
                ChatHost::OwnAgent {
                    runner: ChatRunner {
                        id: runner.scope.runner.id,
                        label: "Composer transport test".into(),
                    },
                    agent: agent.clone(),
                    model: "first-model".into(),
                },
            )
            .await?;
        send_prompts(&opened.conversation).await?;
        (&mut received).await?
    })
    .await;
    heads.abort();
    received.abort();
    assert!(cloud.revoke_runner(runner.scope.runner.id).await?);
    agents.delete(&hub.id(), &agent, 1).await?;
    device.sign_out();
    let _ = nocterm_hubs_model::HubSignIn::sign_out(&*links, &hub.id()).await;
    let prompts = result??;
    assert_eq!(prompts[0].model, "first-model");
    assert_eq!(prompts[1].provider, "openai_chat");
    assert_eq!(prompts[1].model, "second-model");
    assert!(prompts[1].text.is_empty(), "{:?}", prompts[1]);
    assert_eq!(prompts[1].images, [(WireMime::Png, IMAGE.to_vec())]);
    Ok(())
}
