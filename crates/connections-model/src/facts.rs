//! Persisted observations, separate from profile authority.
use crate::{observation::CountryCode, profiles::ProfileId};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap, HashSet};
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
struct Detected {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    os: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    country: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    located: Option<String>,
}
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Known {
    #[serde(default, rename = "server")]
    servers: HashMap<ProfileId, Detected>,
    #[serde(default)]
    geoip: BTreeMap<String, String>,
}
impl Known {
    pub fn os(&self, id: ProfileId) -> Option<&str> {
        self.servers.get(&id)?.os.as_deref()
    }
    pub fn country(&self, id: ProfileId) -> Option<&str> {
        self.servers.get(&id)?.country.as_deref()
    }
    pub fn located_host(&self, id: ProfileId) -> Option<&str> {
        self.servers.get(&id)?.located.as_deref()
    }
    pub fn countries(&self) -> impl Iterator<Item = &str> {
        self.servers
            .values()
            .filter_map(|entry| entry.country.as_deref())
    }
    pub fn geoip(&self) -> &BTreeMap<String, String> {
        &self.geoip
    }
    pub fn retain_profiles(&mut self, ids: &HashSet<ProfileId>) {
        self.servers.retain(|id, _| ids.contains(id));
    }
    pub fn forget_profile(&mut self, id: ProfileId) {
        self.servers.remove(&id);
    }
    pub fn set_os(&mut self, id: ProfileId, os: crate::os::OsId) {
        self.servers.entry(id).or_default().os = Some(os.as_str().to_owned());
    }
    pub fn accept_country(
        &mut self,
        id: ProfileId,
        host: String,
        found: Option<(String, CountryCode)>,
    ) {
        let country = found.as_ref().map(|(_, code)| code.as_str().to_owned());
        if let Some((ip, code)) = found {
            self.geoip.insert(ip, code.as_str().to_owned());
        }
        let entry = self.servers.entry(id).or_default();
        entry.country = country;
        entry.located = Some(host);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn forgetting_an_old_incarnation_preserves_global_geoip_cache() {
        let id = ProfileId::from_bytes([1; 16]);
        let mut known = Known::default();
        known.accept_country(
            id,
            "host".into(),
            Some(("8.8.8.8".into(), CountryCode::new("de").unwrap())),
        );
        known.forget_profile(id);
        assert_eq!(known.country(id), None);
        assert_eq!(known.located_host(id), None);
        assert_eq!(known.geoip().get("8.8.8.8").map(String::as_str), Some("de"));
    }
    #[test]
    fn known_servers_round_trip_without_rewriting_existing_cache_values() {
        let text = r#"[server."00000000-0000-0000-0000-000000000001"]
os = "ubuntu"
country = "de"
located = "example.com"
[server."00000000-0000-0000-0000-000000000002"]
[geoip]
"2a01:4f8::1" = "de"
"1.1.1.1" = "au"
"8.8.8.8" = "../"
"#;
        let known: Known = toml::from_str(text).unwrap();
        let encoded = toml::to_string(&known).unwrap();
        assert_eq!(toml::from_str::<Known>(&encoded).unwrap(), known);
        assert_eq!(toml::from_str::<Known>("").unwrap(), Known::default());
    }
}
