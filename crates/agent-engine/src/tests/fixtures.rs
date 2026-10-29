use serde_json::{Value, json};
use std::sync::{Arc, Mutex};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
    task::JoinHandle,
};

pub(super) struct Server {
    root: String,
    task: JoinHandle<()>,
    requests: Arc<Mutex<Vec<(String, Value, usize)>>>,
}
impl Server {
    pub(super) async fn start(responses: Vec<String>) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let root = format!("http://{}", listener.local_addr().unwrap());
        let requests = Arc::new(Mutex::new(Vec::new()));
        let captured = requests.clone();
        let task = tokio::spawn(async move {
            for response in responses {
                let (mut stream, _) = listener.accept().await.unwrap();
                let mut bytes = Vec::new();
                let split = loop {
                    let mut chunk = [0u8; 4096];
                    let read = stream.read(&mut chunk).await.unwrap();
                    assert!(read != 0, "complete HTTP headers");
                    bytes.extend_from_slice(&chunk[..read]);
                    if let Some(end) = bytes.windows(4).position(|s| s == b"\r\n\r\n") {
                        break end + 4;
                    }
                    assert!(bytes.len() < 64 * 1024);
                };
                let headers = String::from_utf8(bytes[..split].to_vec()).unwrap();
                let length = headers
                    .lines()
                    .find_map(|line| {
                        line.to_ascii_lowercase()
                            .strip_prefix("content-length:")
                            .map(|n| n.trim().parse::<usize>().unwrap())
                    })
                    .unwrap();
                assert!(length <= 32 * 1024 * 1024);
                while bytes.len() < split + length {
                    let mut chunk = [0u8; 4096];
                    let read = stream.read(&mut chunk).await.unwrap();
                    assert!(read != 0);
                    bytes.extend_from_slice(&chunk[..read]);
                }
                let request = serde_json::from_slice(&bytes[split..split + length]).unwrap();
                captured.lock().unwrap().push((headers, request, length));
                stream.write_all(response.as_bytes()).await.unwrap();
                stream.shutdown().await.unwrap();
            }
        });
        Self {
            root,
            task,
            requests,
        }
    }
    pub(super) fn root(&self) -> &str {
        &self.root
    }
    pub(super) fn requests(&self) -> Vec<(String, Value)> {
        self.requests
            .lock()
            .unwrap()
            .iter()
            .map(|(headers, value, _)| (headers.clone(), value.clone()))
            .collect()
    }
    pub(super) fn request_lengths(&self) -> Vec<usize> {
        self.requests
            .lock()
            .unwrap()
            .iter()
            .map(|(_, _, length)| *length)
            .collect()
    }
}
impl Drop for Server {
    fn drop(&mut self) {
        self.task.abort();
    }
}
pub(super) fn http(body: &str) -> String {
    format!(
        "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    )
}
pub(super) fn data(value: Value) -> String {
    format!("data: {value}\n\n")
}
pub(super) fn chat(delta: Value, finish: Value) -> String {
    data(
        json!({"id":"message-1","object":"chat.completion.chunk","model":"fixture-model","created":0,
        "choices":[{"index":0,"delta":delta,"finish_reason":finish}]}),
    )
}
pub(super) fn chat_text(text: &str, usage: bool) -> String {
    let mut body = chat(json!({"role":"assistant","content":text}), Value::Null);
    body += &chat(json!({}), json!("stop"));
    if usage {
        body += &data(
            json!({"id":"message-1","object":"chat.completion.chunk","model":"fixture-model","choices":[],"usage":{"prompt_tokens":11,"completion_tokens":7,"total_tokens":18}}),
        );
    }
    body + "data: [DONE]\n\n"
}
pub(super) fn chat_tools(arguments: &str, count: usize, finish: bool) -> String {
    let calls: Vec<_> = (0..count).map(|index| json!({"index":index,"id":format!("call-{index}"),"type":"function","function":{"name":"exec_command","arguments":arguments}})).collect();
    let body = chat(json!({"role":"assistant","tool_calls":calls}), Value::Null);
    if finish {
        body + &chat(json!({}), json!("tool_calls"))
            + &data(
                json!({"choices":[],"usage":{"prompt_tokens":11,"completion_tokens":7,"total_tokens":18}}),
            )
            + "data: [DONE]\n\n"
    } else {
        body
    }
}
pub(super) fn anthropic(content: Value, delta: Value, reason: &str) -> String {
    let mut body = data(
        json!({"type":"message_start","message":{"id":"msg_1","type":"message","role":"assistant","model":"fixture-model","content":[],"stop_reason":null,"stop_sequence":null,"usage":{"input_tokens":11,"output_tokens":0}}}),
    );
    body += &data(json!({"type":"content_block_start","index":0,"content_block":content}));
    body += &data(json!({"type":"content_block_delta","index":0,"delta":delta}));
    body += &data(json!({"type":"content_block_stop","index":0}));
    body += &data(
        json!({"type":"message_delta","delta":{"stop_reason":reason,"stop_sequence":null},"usage":{"output_tokens":7}}),
    );
    body + &data(json!({"type":"message_stop"}))
}
