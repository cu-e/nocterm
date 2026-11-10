//! Requests against a local server: what is sent, what comes back, and what never happens.

use std::{
    io::{BufRead as _, BufReader, Read as _, Write as _},
    net::TcpListener,
    sync::mpsc,
    thread,
};

use nocterm_plugin_api::{
    PluginNet,
    capability::net::{MAX_RESPONSE, Method, NetCall, Origin},
};

use crate::HttpPluginNet;

/// A server that answers one request with `reply` and reports the request it read.
fn serve_once(reply: &'static str) -> (String, mpsc::Receiver<String>) {
    serve_bytes(reply.as_bytes().to_vec())
}

/// A server that answers one request with the bytes `reply` and reports the request it read.
fn serve_bytes(reply: Vec<u8>) -> (String, mpsc::Receiver<String>) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("a local port");
    let address = format!("http://{}", listener.local_addr().unwrap());
    let (sender, receiver) = mpsc::channel();
    thread::spawn(move || {
        let (stream, _) = listener.accept().expect("a connection");
        let mut reader = BufReader::new(stream.try_clone().unwrap());
        let mut head = String::new();
        let mut length = 0;
        loop {
            let mut line = String::new();
            reader.read_line(&mut line).unwrap();
            if let Some(value) = line.to_ascii_lowercase().strip_prefix("content-length:") {
                length = value.trim().parse().unwrap();
            }
            head.push_str(&line);
            if line == "\r\n" {
                break;
            }
        }
        let mut body = vec![0; length];
        reader.read_exact(&mut body).unwrap();
        head.push_str(&String::from_utf8_lossy(&body));
        let mut stream = stream;
        stream.write_all(&reply).unwrap();
        let _ = sender.send(head);
    });
    (address, receiver)
}

fn call(origin: &str, method: Method, path: &str, body: Option<&str>) -> NetCall {
    let origin = Origin::parse(origin).unwrap();
    NetCall {
        method,
        url: origin.url(path).unwrap(),
        origin,
        body: body.map(|body| body.as_bytes().to_vec()),
        bytes: false,
        credential: Some(("Authorization".into(), "Bearer a-key".into())),
    }
}

/// An answer of `body` with its length.
fn reply_with(body: &[u8]) -> Vec<u8> {
    let mut reply =
        format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n", body.len()).into_bytes();
    reply.extend_from_slice(body);
    reply
}

#[test]
fn a_bytes_request_sends_raw_bytes_and_answers_with_base64_of_every_byte() {
    let binary: Vec<u8> = (0..=255).collect();
    let (origin, seen) = serve_bytes(reply_with(&binary));
    let mut write = call(&origin, Method::Post, "/files/write?file=/a.bin", None);
    write.body = Some(b"raw text".to_vec());
    write.bytes = true;
    let answer = HttpPluginNet::new().send(&write);
    assert_eq!(answer.status, 200);
    assert_eq!(
        nocterm_plugin_api::base64::decode(&answer.body),
        Some(binary),
        "the answer keeps bytes that are not UTF-8"
    );
    let request = seen.recv().unwrap().to_ascii_lowercase();
    assert!(request.contains("content-type: application/octet-stream"));
    assert!(request.ends_with("raw text"));
}

#[test]
fn a_bytes_answer_larger_than_its_limit_is_refused_and_not_cut() {
    let large = vec![b'x'; nocterm_plugin_api::capability::net::MAX_BYTES_RESPONSE + 1];
    let (origin, _seen) = serve_bytes(reply_with(&large));
    let mut read = call(&origin, Method::Get, "/files/contents?file=/big", None);
    read.bytes = true;
    let answer = HttpPluginNet::new().send(&read);
    assert_eq!(answer.status, 0);
    assert!(answer.body.contains("larger than"), "{}", answer.body);
}

#[test]
fn a_request_carries_json_and_the_credential_and_answers_with_status_and_body() {
    let (origin, seen) = serve_once("HTTP/1.1 200 OK\r\nContent-Length: 13\r\n\r\n{\"servers\":1}");
    let answer = HttpPluginNet::new().send(&call(
        &origin,
        Method::Post,
        "/api/items/abc/start",
        Some("{\"action\":\"start\"}"),
    ));
    assert_eq!(answer.status, 200);
    assert_eq!(answer.body, "{\"servers\":1}");
    let request = seen.recv().unwrap().to_ascii_lowercase();
    assert!(request.starts_with("post /api/items/abc/start http/1.1"));
    assert!(request.contains("authorization: bearer a-key"));
    assert!(request.contains("content-type: application/json"));
    assert!(request.contains("accept: application/json"));
    assert!(!request.contains("cookie"));
    assert!(request.ends_with("{\"action\":\"start\"}"));
}

#[test]
fn a_redirect_is_answered_as_it_came_and_never_followed() {
    let (origin, _) = serve_once(
        "HTTP/1.1 302 Found\r\nLocation: http://127.0.0.1:9/steal\r\nContent-Length: 0\r\n\r\n",
    );
    let answer = HttpPluginNet::new().send(&call(&origin, Method::Get, "/x", None));
    assert_eq!(answer.status, 302);
}

#[test]
fn an_answer_is_cut_at_the_response_limit() {
    let body: &'static str = Box::leak(
        format!(
            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n{}",
            MAX_RESPONSE + 10,
            "x".repeat(MAX_RESPONSE + 10)
        )
        .into_boxed_str(),
    );
    let (origin, _) = serve_once(body);
    let answer = HttpPluginNet::new().send(&call(&origin, Method::Get, "/big", None));
    assert_eq!(answer.status, 200);
    assert_eq!(answer.body.len(), MAX_RESPONSE);
}

#[test]
fn a_call_whose_address_left_its_origin_is_not_sent() {
    let mut stray = call("http://127.0.0.1:9", Method::Get, "/x", None);
    stray.url = "http://127.0.0.2:9/x".into();
    let answer = HttpPluginNet::new().send(&stray);
    assert_eq!(answer.status, 0);
    assert!(answer.body.contains("origin"), "{}", answer.body);
}

#[test]
fn an_unreachable_server_is_a_status_zero_answer_in_plain_words() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let origin = format!("http://{}", listener.local_addr().unwrap());
    drop(listener);
    let answer = HttpPluginNet::new().send(&call(&origin, Method::Get, "/x", None));
    assert_eq!(answer.status, 0);
    assert!(!answer.body.contains("127.0.0.1"), "{}", answer.body);
}
