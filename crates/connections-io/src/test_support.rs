use std::{
    io::{Read as _, Write as _},
    net::TcpListener,
    thread,
};

pub(crate) fn serve(status: u16, body: Vec<u8>) -> (String, thread::JoinHandle<String>) {
    serve_after(status, body, None)
}

pub(crate) fn serve_after(
    status: u16,
    body: Vec<u8>,
    barrier: Option<std::sync::Arc<std::sync::Barrier>>,
) -> (String, thread::JoinHandle<String>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let endpoint = format!("http://{}/", listener.local_addr().unwrap());
    let worker = thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        stream
            .set_read_timeout(Some(std::time::Duration::from_secs(2)))
            .unwrap();
        let mut request = Vec::new();
        while !request.ends_with(b"\r\n\r\n") {
            let mut byte = [0];
            stream.read_exact(&mut byte).unwrap();
            request.push(byte[0]);
        }
        if let Some(barrier) = barrier {
            barrier.wait();
        }
        let header = format!(
            "HTTP/1.1 {status} response\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            body.len()
        );
        stream.write_all(header.as_bytes()).unwrap();
        // An over-limit client may close before consuming the whole body.
        let _ = stream.write_all(&body);
        String::from_utf8(request).unwrap()
    });
    (endpoint, worker)
}
