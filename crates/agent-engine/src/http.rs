//! Exact-destination bounded transport; Rig owns the provider wire and SSE decoder.
use crate::{call::ModelCalls, config::ProviderKind, lease::ProviderLease};
use futures::StreamExt;
use nocterm_cloud_model::{engine::EngineError, payload::MAX_LOGICAL_PAYLOAD_BYTES};
use nocterm_core::SharedCause;
use reqwest::dns::{Addrs, Name, Resolve, Resolving};
use rig_core::http_client::{
    Bytes, Error, HttpClientExt, LazyBody, MultipartForm, Request, Response, StreamingResponse,
};
use std::{
    net::{IpAddr, Ipv4Addr},
    sync::Arc,
    time::Duration,
};

#[derive(Clone)]
pub(crate) struct ProviderHttp {
    client: reqwest::Client,
    destination: Arc<str>,
    lease: ProviderLease,
    kind: ProviderKind,
    calls: Option<ModelCalls>,
}
impl ProviderHttp {
    pub(crate) fn new(lease: ProviderLease) -> Result<Self, EngineError> {
        let kind = lease.binding().kind();
        let endpoint = lease.binding().endpoint();
        let client = reqwest::Client::builder()
            .https_only(!endpoint.fixture())
            .no_proxy()
            .redirect_policy(reqwest::redirect::Policy::none())
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(300))
            .dns_resolver(Arc::new(PublicDns))
            .build()
            .map_err(|error| {
                EngineError::Provider(SharedCause::new(TransportFailure::Request(error)))
            })?;
        Ok(Self {
            client,
            destination: endpoint.request_url(kind).into(),
            lease,
            kind,
            calls: None,
        })
    }
    /// Each model receives its own immutable run owner, never a shared mutable
    /// factory-wide current-call pointer. An unscoped template refuses HTTP.
    pub(crate) fn for_engine(&self, calls: ModelCalls) -> Self {
        let mut transport = self.clone();
        transport.calls = Some(calls);
        transport
    }
    async fn stream(&self, request: Request<Bytes>) -> Result<StreamingResponse, Box<Error>> {
        if request.uri() != self.destination.as_ref()
            || request.method() != rig_core::http_client::Method::POST
            || request.body().len() > MAX_LOGICAL_PAYLOAD_BYTES
        {
            return Err(Box::new(Error::instance(TransportFailure::Refused)));
        }
        let calls = self
            .calls
            .as_ref()
            .ok_or_else(|| Error::instance(TransportFailure::Refused))?;
        let scope = calls
            .scope()
            .map_err(|error| Error::instance(TransportFailure::Admission(error)))?;
        let admitted = scope
            .reserve(request.body().len())
            .await
            .map_err(|error| Error::instance(TransportFailure::Admission(error)))?;
        let (mut parts, body) = request.into_parts();
        let length = body.len();
        // This pinned reqwest fork retries reusable bodies internally. A
        // one-shot body prevents a second network attempt below our once gate.
        let body = request_body(body);
        parts.headers.remove("authorization");
        parts.headers.remove("x-api-key");
        let mut outgoing = self
            .client
            .post(self.destination.as_ref())
            .headers(parts.headers)
            .header(reqwest::header::CONTENT_LENGTH, length)
            .body(body);
        let header = self.lease.header(self.kind).map_err(Error::instance)?;
        outgoing = match self.kind {
            ProviderKind::OpenAiChat => outgoing.header("authorization", header),
            ProviderKind::AnthropicMessages => outgoing.header("x-api-key", header),
        };
        scope
            .check(&admitted)
            .map_err(|error| Error::instance(TransportFailure::Admission(error)))?;
        let response = outgoing.send().await.map_err(request_error)?;
        let status = response.status();
        if !status.is_success() {
            // Do not retain an error body which can echo credentials or prompts.
            return Err(Box::new(Error::non_success_with_details(
                status,
                Default::default(),
                "provider refused request".into(),
            )));
        }
        if response
            .content_length()
            .is_some_and(|length| length > MAX_LOGICAL_PAYLOAD_BYTES as u64)
        {
            return Err(Box::new(Error::instance(TransportFailure::Capacity)));
        }
        let mut builder = Response::builder().status(status);
        if let Some(headers) = builder.headers_mut() {
            *headers = response.headers().clone();
        }
        let stream = futures::stream::try_unfold(
            (Box::pin(response.bytes_stream()), 0usize),
            |(mut stream, count)| async move {
                let Some(chunk) = stream.next().await else {
                    return Ok(None);
                };
                let chunk = chunk.map_err(request_error)?;
                let count = count.saturating_add(chunk.len());
                if count > MAX_LOGICAL_PAYLOAD_BYTES {
                    return Err(Error::instance(TransportFailure::Capacity));
                }
                Ok(Some((chunk, (stream, count))))
            },
        );
        builder
            .body(Box::pin(stream) as rig_core::http_client::BoxedStream)
            .map_err(|error| Box::new(Error::from(error)))
    }
}
impl HttpClientExt for ProviderHttp {
    fn send<T, U>(
        &self,
        _request: Request<T>,
    ) -> impl Future<Output = Result<Response<LazyBody<U>>, Error>> + Send + 'static
    where
        T: Into<Bytes> + Send,
        U: From<Bytes> + Send + 'static,
    {
        std::future::ready(Err(Error::instance(TransportFailure::Refused)))
    }
    fn send_multipart<U>(
        &self,
        _request: Request<MultipartForm>,
    ) -> impl Future<Output = Result<Response<LazyBody<U>>, Error>> + Send + 'static
    where
        U: From<Bytes> + Send + 'static,
    {
        std::future::ready(Err(Error::instance(TransportFailure::Refused)))
    }
    fn send_streaming<T>(
        &self,
        request: Request<T>,
    ) -> impl Future<Output = Result<StreamingResponse, Error>> + Send
    where
        T: Into<Bytes> + Send,
    {
        let request = request.map(Into::into);
        async move { self.stream(request).await.map_err(|error| *error) }
    }
}
fn request_body(body: Bytes) -> reqwest::Body {
    reqwest::Body::wrap_stream(futures::stream::once(std::future::ready(Ok::<
        _,
        std::io::Error,
    >(body))))
}

fn request_error(error: reqwest::Error) -> Error {
    Error::instance(TransportFailure::Request(error))
}

struct PublicDns;
impl Resolve for PublicDns {
    fn resolve(&self, name: Name) -> Resolving {
        Box::pin(async move {
            let addresses: Vec<_> = tokio::net::lookup_host((name.as_str(), 0)).await?.collect();
            if addresses.is_empty() || addresses.iter().any(|address| !is_public(address.ip())) {
                return Err(
                    Box::new(TransportFailure::Refused) as Box<dyn std::error::Error + Send + Sync>
                );
            }
            Ok(Box::new(addresses.into_iter()) as Addrs)
        })
    }
}
pub(crate) fn is_public(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ip) => public_v4(ip),
        IpAddr::V6(ip) => {
            let s = ip.segments();
            (s[0] & 0xe000) == 0x2000
                && s[0] != 0x2002
                && !(s[0] == 0x2001 && (s[1] < 0x0200 || s[1] == 0x0db8))
                && !(s[0] == 0x3fff && s[1] < 0x1000)
        }
    }
}
fn public_v4(ip: Ipv4Addr) -> bool {
    let [a, b, c, _] = ip.octets();
    !(matches!(a, 0 | 10 | 127 | 224..=255)
        || (a == 100 && (64..=127).contains(&b))
        || (a == 169 && b == 254)
        || (a == 172 && (16..=31).contains(&b))
        || (a == 192 && b == 168)
        || (a == 192 && b == 0 && (c == 0 || c == 2))
        || (a == 198 && (b == 18 || b == 19 || (b == 51 && c == 100)))
        || (a == 203 && b == 0 && c == 113))
}
#[derive(thiserror::Error)]
enum TransportFailure {
    #[error("the host refused model-call admission")]
    Admission(#[source] EngineError),
    #[error("provider destination or operation refused")]
    Refused,
    #[error("provider response exceeded its byte limit")]
    Capacity,
    #[error("provider transport failed")]
    Request(#[source] reqwest::Error),
}
impl std::fmt::Debug for TransportFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Display::fmt(self, f)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn an_admitted_body_cannot_be_replayed_by_the_pinned_transport() {
        let request = reqwest::Client::new()
            .post("http://127.0.0.1/fixture")
            .body(request_body(Bytes::from_static(b"one request")))
            .build()
            .unwrap();
        assert!(request.try_clone().is_none());
    }
}
