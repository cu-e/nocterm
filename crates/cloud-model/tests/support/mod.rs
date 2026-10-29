#![allow(dead_code)]
use nocterm_cloud_model::{
    authority::{
        AuthenticationBinding, CommandIssuer, ExecutionAuthority, RunBinding, SessionReceipt,
        TargetBinding,
    },
    grant::GrantOrigin,
    identity::{
        AccountIdentity, AccountScope, ChatControlRevision, CloudChatId, CommandId, Fingerprint,
        GrantEpoch, GrantId, HubIdentity, NamespaceId, RemoteSessionId, RunEpoch, RunId, RunnerId,
        RunnerKeyEpoch, SessionEpoch,
    },
};

pub(crate) fn fingerprint(byte: u8) -> Fingerprint {
    Fingerprint::from_bytes([byte; 32])
}
pub(crate) fn scope(byte: u8) -> AccountScope {
    AccountScope::new(
        HubIdentity::new(NamespaceId::new("instance").unwrap(), fingerprint(byte)),
        AccountIdentity::new(NamespaceId::new("account").unwrap(), fingerprint(byte)),
    )
}
pub(crate) fn target(byte: u8) -> TargetBinding {
    TargetBinding::new(
        fingerprint(byte),
        fingerprint(90),
        NamespaceId::new("remote-user").unwrap(),
        fingerprint(91),
    )
}
pub(crate) fn run(account: u8, chat: u8, id: u8, epoch: u64) -> RunBinding {
    RunBinding::new(
        scope(account),
        RunnerId::from_bytes([42; 16]),
        RunnerKeyEpoch::new(1).unwrap(),
        CloudChatId::from_bytes([chat; 16]),
        RunId::from_bytes([id; 16]),
        RunEpoch::new(epoch).unwrap(),
    )
}
pub(crate) fn authority(binding: RunBinding) -> ExecutionAuthority {
    ExecutionAuthority::new(
        binding,
        SessionReceipt::new(
            RemoteSessionId::from_bytes([1; 16]),
            SessionEpoch::new(1).unwrap(),
            target(1),
            authentication(),
        ),
        GrantId::from_bytes([1; 16]),
        GrantEpoch::new(1).unwrap(),
    )
}

pub(crate) fn authentication() -> AuthenticationBinding {
    AuthenticationBinding::new(fingerprint(80), fingerprint(81))
}
pub(crate) fn origin() -> GrantOrigin {
    GrantOrigin::new(
        CommandIssuer::new(
            NamespaceId::new("device").unwrap(),
            fingerprint(82),
            fingerprint(83),
        ),
        CommandId::from_bytes([1; 16]),
        ChatControlRevision::from_u64(1),
    )
}
