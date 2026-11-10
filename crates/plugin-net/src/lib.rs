//! The network of user plugins: the `PluginNet` port of `nocterm-plugin-api` over a blocking
//! HTTP client.
//!
//! Owns: the client configuration (no cookies, no redirects followed, a timeout, rustls), the
//! check that the parsed address is still at the origin the user entered, the request headers
//! (`Accept`, `Content-Type` JSON or raw bytes, the credential header, a plugin user agent),
//! reading at most `MAX_RESPONSE` bytes of an answer, and for a `bytes` request answering with
//! base64 of at most `MAX_BYTES_RESPONSE` bytes, refusing a larger answer rather than cutting it.
//! Does not own: which address or credential a request carries (the host builds the call from
//! the plugin's settings), rate limits, consent, or delivering the answer to the plugin.
//! Ports used: none. Ports provided: `PluginNet`. Settings: none. Globals: none.
//! Invariants: a request never leaves the origin it was built for; a redirect is answered as
//! it came, never followed; the credential is sent only in its header and never logged; a
//! failure is a status-0 answer with words for the plugin, not an error of the application.
//! Calls block: the host runs them on a background thread.

use std::{io::Read as _, time::Duration};

use nocterm_plugin_api::{
    PluginNet,
    capability::net::{MAX_BYTES_RESPONSE, MAX_RESPONSE, Method, NetAnswer, NetCall, TIMEOUT_MS},
};

/// The client user plugins' requests go through.
pub struct HttpPluginNet {
    client: Option<reqwest::blocking::Client>,
}

impl Default for HttpPluginNet {
    fn default() -> Self {
        Self::new()
    }
}

impl HttpPluginNet {
    pub fn new() -> Self {
        let client = reqwest::blocking::Client::builder()
            .timeout(Duration::from_millis(u64::from(TIMEOUT_MS)))
            .redirect(reqwest::redirect::Policy::none())
            .user_agent(concat!("nocterm-plugin/", env!("CARGO_PKG_VERSION")))
            .build()
            .ok();
        Self { client }
    }
}

impl PluginNet for HttpPluginNet {
    fn send(&self, call: &NetCall) -> NetAnswer {
        let Some(client) = &self.client else {
            return NetAnswer::failed("the HTTP client could not start");
        };
        let url = match checked_url(call) {
            Ok(url) => url,
            Err(answer) => return answer,
        };
        let mut request = match call.method {
            Method::Get => client.get(url),
            Method::Post => client.post(url),
            Method::Put => client.put(url),
            Method::Delete => client.delete(url),
        }
        .header(reqwest::header::ACCEPT, "application/json");
        if let Some(body) = &call.body {
            let kind = if call.bytes {
                "application/octet-stream"
            } else {
                "application/json"
            };
            request = request
                .header(reqwest::header::CONTENT_TYPE, kind)
                .body(body.clone());
        }
        if let Some((header, value)) = &call.credential {
            request = request.header(header.as_str(), value.as_str());
        }
        match request.send() {
            Ok(response) if call.bytes => answer_bytes(response),
            Ok(response) => answer(response),
            Err(error) => NetAnswer::failed(failure(&error)),
        }
    }
}

/// The address as the client parses it, refused unless it is at the call's origin.
fn checked_url(call: &NetCall) -> Result<reqwest::Url, NetAnswer> {
    let url = reqwest::Url::parse(&call.url)
        .map_err(|_| NetAnswer::failed("the address could not be read"))?;
    let scheme = if call.origin.is_secure() {
        "https"
    } else {
        "http"
    };
    let authority = match (url.host_str(), url.port()) {
        (Some(host), Some(port)) => format!("{host}:{port}"),
        (Some(host), None) => host.to_owned(),
        (None, _) => String::new(),
    };
    let same = url.scheme() == scheme
        && url.username().is_empty()
        && url.password().is_none()
        && authority.eq_ignore_ascii_case(call.origin.authority());
    if same {
        Ok(url)
    } else {
        Err(NetAnswer::failed(
            "the address left the origin set in the settings",
        ))
    }
}

fn answer(response: reqwest::blocking::Response) -> NetAnswer {
    let status = response.status().as_u16();
    let mut bytes = Vec::new();
    if let Err(error) = response.take(MAX_RESPONSE as u64).read_to_end(&mut bytes) {
        return NetAnswer::failed(format!("the answer broke off: {error}"));
    }
    NetAnswer {
        status,
        body: String::from_utf8_lossy(&bytes).into_owned(),
    }
}

/// The answer to a `bytes` request: its bytes as base64, or status 0 when there are more than
/// [`MAX_BYTES_RESPONSE`], since a cut file would be a wrong one.
fn answer_bytes(response: reqwest::blocking::Response) -> NetAnswer {
    let status = response.status().as_u16();
    let mut bytes = Vec::new();
    let limit = MAX_BYTES_RESPONSE as u64 + 1;
    if let Err(error) = response.take(limit).read_to_end(&mut bytes) {
        return NetAnswer::failed(format!("the answer broke off: {error}"));
    }
    if bytes.len() > MAX_BYTES_RESPONSE {
        return NetAnswer::failed(format!(
            "the answer is larger than {} KiB",
            MAX_BYTES_RESPONSE / 1024
        ));
    }
    NetAnswer {
        status,
        body: nocterm_plugin_api::base64::encode(&bytes),
    }
}

/// Words for a failed request, without the address or anything the request carried.
fn failure(error: &reqwest::Error) -> &'static str {
    if error.is_timeout() {
        "the server did not answer in time"
    } else if error.is_connect() {
        "the server could not be reached"
    } else {
        "the request failed"
    }
}

#[cfg(test)]
mod tests;
