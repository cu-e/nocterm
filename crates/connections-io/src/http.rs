//! HTTP work is blocking and must be dispatched by the caller off the UI thread.
use nocterm_connections_model::observation::{
    CountryCode, CountryLocator, ObservationError, PublicAddress,
};
use std::{io::Read as _, time::Duration};

pub(crate) const MAX_GEOIP_BYTES: u64 = 4 * 1024;
pub(crate) const MAX_FLAG_BYTES: u64 = 256 * 1024;
const TIMEOUT: Duration = Duration::from_secs(8);

pub(crate) struct HttpCountry {
    endpoint: String,
}
impl Default for HttpCountry {
    fn default() -> Self {
        Self {
            endpoint: "https://api.country.is/".into(),
        }
    }
}
impl HttpCountry {
    #[cfg(test)]
    pub(crate) fn at(endpoint: String) -> Self {
        Self { endpoint }
    }
}
impl CountryLocator for HttpCountry {
    fn country(&self, ip: PublicAddress) -> Result<Option<CountryCode>, ObservationError> {
        #[derive(serde::Deserialize)]
        struct Answer {
            country: Option<String>,
        }
        let body = get(
            &format!("{}{ip}", self.endpoint, ip = ip.as_ip()),
            MAX_GEOIP_BYTES,
        )?;
        let answer: Answer = serde_json::from_slice(&body)
            .map_err(|source| ObservationError::Decode(Box::new(source)))?;
        answer
            .country
            .map(|code| {
                CountryCode::new(&code.to_ascii_lowercase())
                    .map_err(|source| ObservationError::Decode(Box::new(source)))
            })
            .transpose()
    }
}

pub(crate) fn get(url: &str, limit: u64) -> Result<Vec<u8>, ObservationError> {
    let client = reqwest::blocking::Client::builder()
        .timeout(TIMEOUT)
        .user_agent(concat!("nocterm/", env!("CARGO_PKG_VERSION")))
        .build()
        .map_err(|source| ObservationError::Network(Box::new(source.without_url())))?;
    let response = client
        .get(url)
        .send()
        .map_err(|source| ObservationError::Network(Box::new(source.without_url())))?;
    let response = response
        .error_for_status()
        .map_err(|source| ObservationError::Status {
            status: source.status().map_or(0, |status| status.as_u16()),
            source: Box::new(source.without_url()),
        })?;
    let mut body = Vec::new();
    response
        .take(limit + 1)
        .read_to_end(&mut body)
        .map_err(|source| ObservationError::Body(Box::new(source)))?;
    if body.len() as u64 > limit {
        return Err(ObservationError::BodyLimit { limit });
    }
    Ok(body)
}

#[cfg(test)]
mod tests;
