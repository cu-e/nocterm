//! Fixtures shared by the managed-mode tests.
use super::ChatTokens;
use nocterm_agent_engine::{ProviderBinding, ProviderEndpoint, ProviderKind};
use nocterm_cloud_model::identity::{
    AccountIdentity, AccountScope, CloudChatId, Fingerprint, HubIdentity, NamespaceId,
};
use nocterm_hub_model::agents::ChatAccessToken;

pub(super) fn namespace(text: &str) -> NamespaceId {
    NamespaceId::new(text.to_owned()).unwrap()
}
pub(super) fn scope() -> AccountScope {
    AccountScope::new(
        HubIdentity::new(namespace("hub"), Fingerprint::from_bytes([1; 32])),
        AccountIdentity::new(namespace("account"), Fingerprint::from_bytes([2; 32])),
    )
}
pub(super) fn binding(root: &str) -> ProviderBinding {
    ProviderBinding::new(
        scope(),
        ProviderKind::OpenAiChat,
        ProviderEndpoint::broker(root).unwrap(),
        vec![namespace("gpt-x")],
    )
    .unwrap()
}
/// Tokens over a broker that is never contacted. Needs a Tokio runtime.
pub(super) fn tokens() -> ChatTokens {
    ChatTokens::new(
        binding("http://127.0.0.1:9"),
        tokio::runtime::Handle::current(),
    )
}
pub(super) fn chat(byte: u8) -> CloudChatId {
    CloudChatId::from_bytes([byte; 16])
}
pub(super) fn token(text: &str) -> ChatAccessToken {
    ChatAccessToken::new(format!("nvt1_{text}")).unwrap()
}
