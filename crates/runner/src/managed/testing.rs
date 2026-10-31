//! Fixtures shared by the managed-mode tests.
use super::ChatTokens;
use nocterm_agent_engine::{ProviderBinding, ProviderEndpoint, ProviderKind};
use nocterm_cloud_model::identity::{
    AccountIdentity, AccountScope, CloudChatId, Fingerprint, HubIdentity, NamespaceId,
};
use nocterm_hub_io::runner_control::ChatAccessToken;
use std::sync::{Arc, Mutex};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
};

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

/// A local HTTP broker that records each request and answers the scripted SSE
/// replies in order, repeating the last one.
pub(super) struct Broker {
    root: String,
    seen: Arc<Mutex<Vec<(String, String)>>>,
}
impl Broker {
    pub(super) async fn start(replies: Vec<String>) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let root = format!("http://{}", listener.local_addr().unwrap());
        let seen = Arc::new(Mutex::new(Vec::new()));
        let record = seen.clone();
        tokio::spawn(async move {
            let mut served = 0;
            loop {
                let Ok((mut stream, _)) = listener.accept().await else {
                    return;
                };
                let mut bytes = Vec::new();
                let mut chunk = [0u8; 4096];
                let split = loop {
                    let read = stream.read(&mut chunk).await.unwrap();
                    bytes.extend_from_slice(&chunk[..read]);
                    if let Some(end) = bytes.windows(4).position(|w| w == b"\r\n\r\n") {
                        break end + 4;
                    }
                };
                let head = String::from_utf8_lossy(&bytes[..split]).to_ascii_lowercase();
                let length = head
                    .lines()
                    .find_map(|line| line.strip_prefix("content-length:"))
                    .map_or(0, |n| n.trim().parse::<usize>().unwrap());
                while bytes.len() < split + length {
                    let read = stream.read(&mut chunk).await.unwrap();
                    bytes.extend_from_slice(&chunk[..read]);
                }
                let body = String::from_utf8_lossy(&bytes[split..]).into_owned();
                record.lock().unwrap().push((head, body));
                let reply = &replies[served.min(replies.len() - 1)];
                served += 1;
                stream.write_all(reply.as_bytes()).await.unwrap();
                let _ = stream.shutdown().await;
            }
        });
        Self { root, seen }
    }
    pub(super) fn root(&self) -> &str {
        &self.root
    }
    pub(super) fn heads(&self) -> Vec<String> {
        self.seen
            .lock()
            .unwrap()
            .iter()
            .map(|r| r.0.clone())
            .collect()
    }
    pub(super) fn bodies(&self) -> Vec<String> {
        self.seen
            .lock()
            .unwrap()
            .iter()
            .map(|r| r.1.clone())
            .collect()
    }
}
fn sse(deltas: Vec<serde_json::Value>) -> String {
    let data = |value: serde_json::Value| format!("data: {value}\n\n");
    let mut body: String = deltas.into_iter().map(data).collect();
    body += "data: [DONE]\n\n";
    format!(
        "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    )
}
fn chunk(delta: serde_json::Value, finish: serde_json::Value) -> serde_json::Value {
    serde_json::json!({"id":"m","object":"chat.completion.chunk","model":"gpt-x","created":0,
        "choices":[{"index":0,"delta":delta,"finish_reason":finish}]})
}
fn usage() -> serde_json::Value {
    serde_json::json!({"id":"m","object":"chat.completion.chunk","model":"gpt-x","choices":[],
        "usage":{"prompt_tokens":3,"completion_tokens":2,"total_tokens":5}})
}
/// A final assistant message.
pub(super) fn text_reply(text: &str) -> String {
    sse(vec![
        chunk(
            serde_json::json!({"role":"assistant","content":text}),
            serde_json::Value::Null,
        ),
        chunk(serde_json::json!({}), serde_json::json!("stop")),
        usage(),
    ])
}
/// An assistant turn that calls `exec_command` with the given JSON arguments.
pub(super) fn exec_reply(arguments: &str) -> String {
    let call = serde_json::json!([{"index":0,"id":"call-0","type":"function",
        "function":{"name":"exec_command","arguments":arguments}}]);
    sse(vec![
        chunk(
            serde_json::json!({"role":"assistant","tool_calls":call}),
            serde_json::Value::Null,
        ),
        chunk(serde_json::json!({}), serde_json::json!("tool_calls")),
        usage(),
    ])
}
