//! Local resolution filters literals and names before calling native DNS.
use nocterm_connections_model::observation::{
    ObservationError, PublicAddress, PublicAddressResolver,
};
use std::net::{IpAddr, ToSocketAddrs as _};

pub struct NativeResolver;

impl PublicAddressResolver for NativeResolver {
    fn resolve(&self, host: &str) -> Result<Option<PublicAddress>, ObservationError> {
        resolve_with(host, |host| {
            (host, 0)
                .to_socket_addrs()
                .map(|addresses| addresses.map(|address| address.ip()))
        })
    }
}

fn resolve_with<I: IntoIterator<Item = IpAddr>>(
    host: &str,
    resolve: impl FnOnce(&str) -> std::io::Result<I>,
) -> Result<Option<PublicAddress>, ObservationError> {
    let host = host.trim_start_matches('[').trim_end_matches(']');
    if let Ok(ip) = host.parse::<IpAddr>() {
        return Ok(PublicAddress::new(ip));
    }
    if host.is_empty() || host.ends_with(".local") || !host.contains('.') {
        return Ok(None);
    }
    let addresses =
        resolve(host).map_err(|source| ObservationError::Resolution(Box::new(source)))?;
    Ok(addresses.into_iter().find_map(PublicAddress::new))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn literals_and_local_names_never_use_dns() {
        for host in [
            "192.168.1.10",
            "[::1]",
            "pi.local",
            "localhost",
            "",
            "8.8.8.8",
        ] {
            let result =
                resolve_with::<Vec<IpAddr>>(host, |_| panic!("unexpected DNS for {host}")).unwrap();
            assert_eq!(result.is_some(), host == "8.8.8.8");
        }
    }
    #[test]
    fn resolution_selects_first_public_address_only() {
        let found = resolve_with("example.com", |_| {
            Ok(vec![
                "10.0.0.1".parse().unwrap(),
                "8.8.8.8".parse().unwrap(),
            ])
        })
        .unwrap()
        .unwrap();
        assert_eq!(found.as_ip(), "8.8.8.8".parse::<IpAddr>().unwrap());
        assert!(matches!(
            resolve_with::<Vec<IpAddr>>("example.com", |_| Err(std::io::Error::other(
                "DNS failed"
            ))),
            Err(ObservationError::Resolution(_))
        ));
    }
}
