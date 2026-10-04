//! Explicit, bounded requests to Zed's public extension registry.
use crate::{ARCHIVE_LIMIT, ThemeError, valid_id};
use serde::{Deserialize, Serialize};
use std::{io::Read, time::Duration};

/// Public registry metadata, also retained as our installation manifest.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExtensionInfo {
    pub id: String,
    pub name: String,
    pub version: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub authors: Vec<String>,
    #[serde(default)]
    pub repository: Option<String>,
    #[serde(default)]
    pub download_count: u64,
}
/// Injectable so the settings browser never needs network access in tests.
pub trait ThemeRegistry: Send + Sync {
    fn search(&self, query: &str) -> Result<Vec<ExtensionInfo>, ThemeError>;
    fn download(&self, extension: &ExtensionInfo) -> Result<Vec<u8>, ThemeError>;
}
/// Blocking client; run requests on the background executor.
pub struct ZedRegistry {
    client: reqwest::blocking::Client,
    base: reqwest::Url,
}
impl ZedRegistry {
    pub fn new() -> Result<Self, ThemeError> {
        let client = reqwest::blocking::Client::builder()
            .timeout(Duration::from_secs(30))
            .user_agent(concat!("nocterm/", env!("CARGO_PKG_VERSION")))
            .build()
            .map_err(http_error)?;
        Ok(Self {
            client,
            base: reqwest::Url::parse("https://api.zed.dev/").expect("constant URL"),
        })
    }
    fn search_url(&self, query: &str) -> reqwest::Url {
        let mut url = self.base.join("extensions").expect("constant relative URL");
        url.query_pairs_mut()
            .append_pair("max_schema_version", "1")
            .append_pair("provides", "themes")
            .append_pair("filter", query);
        url
    }
    fn download_url(&self, extension: &ExtensionInfo) -> Result<reqwest::Url, ThemeError> {
        if !valid_id(&extension.id) || extension.version.is_empty() {
            return Err(ThemeError::Invalid("invalid extension identity".into()));
        }
        let mut url = self.base.clone();
        url.path_segments_mut().expect("HTTP base URL").extend([
            "extensions",
            &extension.id,
            &extension.version,
            "download",
        ]);
        Ok(url)
    }
    fn get(&self, url: reqwest::Url, limit: usize) -> Result<Vec<u8>, ThemeError> {
        let response = self
            .client
            .get(url)
            .send()
            .and_then(|r| r.error_for_status())
            .map_err(http_error)?;
        if response.content_length().is_some_and(|n| n > limit as u64) {
            return Err(ThemeError::Invalid(
                "registry response exceeds size limit".into(),
            ));
        }
        let mut bytes = Vec::new();
        response.take(limit as u64 + 1).read_to_end(&mut bytes)?;
        if bytes.len() > limit {
            return Err(ThemeError::Invalid(
                "registry response exceeds size limit".into(),
            ));
        }
        Ok(bytes)
    }
}
fn http_error(error: reqwest::Error) -> ThemeError {
    ThemeError::Invalid(format!("Zed registry: {}", error.without_url()))
}
impl ThemeRegistry for ZedRegistry {
    fn search(&self, query: &str) -> Result<Vec<ExtensionInfo>, ThemeError> {
        #[derive(Deserialize)]
        struct Response {
            data: Vec<ExtensionInfo>,
        }
        let response: Response =
            serde_json::from_slice(&self.get(self.search_url(query), 4 * 1024 * 1024)?)?;
        Ok(response
            .data
            .into_iter()
            .filter(|e| valid_id(&e.id))
            .collect())
    }
    fn download(&self, extension: &ExtensionInfo) -> Result<Vec<u8>, ThemeError> {
        self.get(self.download_url(extension)?, ARCHIVE_LIMIT)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn recorded_response_handles_unknown_fields_and_null_descriptions() {
        #[derive(Deserialize)]
        struct Response {
            data: Vec<ExtensionInfo>,
        }
        let response: Response =
            serde_json::from_slice(include_bytes!("../tests/fixtures/search_response.json"))
                .unwrap();
        assert!(response.data.iter().any(|e| e.id == "dracula"));
        let extension: ExtensionInfo = serde_json::from_str(
            r#"{"id":"x","name":"X","version":"1","description":null,"future":true}"#,
        )
        .unwrap();
        assert_eq!(extension.description, None);
    }
    #[test]
    fn live_dracula_search_and_install_when_requested() {
        if std::env::var("NOCTERM_LIVE_ZED").as_deref() != Ok("1") {
            return;
        }
        let registry = ZedRegistry::new().unwrap();
        let extension = registry
            .search("dracula")
            .unwrap()
            .into_iter()
            .find(|e| e.id == "dracula")
            .expect("Dracula in registry");
        let bytes = registry.download(&extension).unwrap();
        let root = tempfile::tempdir().unwrap();
        crate::install_archive(root.path(), &extension, &bytes).unwrap();
        let catalogue = crate::ThemeCatalog::load(&crate::ThemeDirs {
            user: root.path().join("user"),
            installed: root.path().into(),
        });
        assert!(!catalogue.entries().is_empty());
    }

    #[test]
    fn urls_encode_search_and_pin_versions() {
        let registry = ZedRegistry::new().unwrap();
        let url = registry.search_url("a & b");
        let pairs: std::collections::BTreeMap<_, _> = url.query_pairs().collect();
        assert_eq!(pairs.get("max_schema_version").unwrap(), "1");
        assert_eq!(pairs.get("provides").unwrap(), "themes");
        assert_eq!(pairs.get("filter").unwrap(), "a & b");
        let info: ExtensionInfo = serde_json::from_str(
            r#"{"id":"x","name":"X","version":"1.2","description":null,"unknown":0}"#,
        )
        .unwrap();
        assert_eq!(
            registry.download_url(&info).unwrap().path(),
            "/extensions/x/1.2/download"
        );
    }

    #[test]
    fn http_caps_bodies_without_content_length_and_redacts_request_urls() {
        use std::io::{Read as _, Write as _};
        use std::net::TcpListener;
        for (status, body) in [("200 OK", vec![b'x'; 4097]), ("403 Forbidden", Vec::new())] {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let address = listener.local_addr().unwrap();
            let server = std::thread::spawn(move || {
                let (mut stream, _) = listener.accept().unwrap();
                stream
                    .set_read_timeout(Some(Duration::from_secs(5)))
                    .unwrap();
                let mut request = Vec::new();
                let mut byte = [0];
                while !request.ends_with(b"\r\n\r\n") {
                    stream.read_exact(&mut byte).unwrap();
                    request.push(byte[0]);
                }
                write!(stream, "HTTP/1.0 {status}\r\nConnection: close\r\n\r\n").unwrap();
                stream.write_all(&body).unwrap();
            });
            let url =
                reqwest::Url::parse(&format!("http://{address}/download?presigned-secret=test"))
                    .unwrap();
            let error = ZedRegistry::new().unwrap().get(url, 4096).unwrap_err();
            let message = error.to_string();
            if status == "200 OK" {
                assert!(message.contains("size limit"), "{message}");
            } else {
                assert!(message.contains("403"), "{message}");
                assert!(!message.contains("presigned-secret"), "{message}");
                assert!(!message.contains(&address.to_string()), "{message}");
            }
            server.join().unwrap();
        }
    }
}
