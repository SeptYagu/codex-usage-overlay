//! Narrow, secret-safe URLs for the user's explicit DSH pairing link.
use reqwest::Url;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};

pub(super) struct PairingUrl {
    pub connect: Url,
    pub origin: Url,
}

pub(super) fn loopback_origin(url: &Url) -> Result<Url, String> {
    if !matches!(url.scheme(), "http" | "https")
        || !url.username().is_empty()
        || url.password().is_some()
        || url.fragment().is_some()
        || !matches!(url.host(), Some(::url::Host::Ipv4(ip)) if ip.is_loopback())
            && !matches!(url.host(), Some(::url::Host::Ipv6(ip)) if ip.is_loopback())
            && !matches!(url.host_str(), Some("localhost"))
    {
        return Err(
            "DSH connection requires an HTTP(S) loopback URL without userinfo or fragment".into(),
        );
    }
    Url::parse(&format!("{}/", url.origin().ascii_serialization()))
        .map_err(|_| "Invalid DSH loopback origin".into())
}

pub(super) fn pairing_url(input: &str) -> Result<PairingUrl, String> {
    if input.len() > 16_384 {
        return Err("Invalid DSH connection link".into());
    }
    let mut url = Url::parse(input.trim()).map_err(|_| "Invalid DSH connection link")?;
    let origin = loopback_origin(&url)?;
    if url.path() != "/connect" {
        return Err("Paste the complete DSH /connect launch link".into());
    }
    let pairs: Vec<_> = url.query_pairs().collect();
    let tokens: Vec<_> = pairs.iter().filter(|(key, _)| key == "token").collect();
    if tokens.len() != 1
        || tokens[0].1.is_empty()
        || tokens[0].1.len() > 8192
        || tokens[0].1.chars().any(char::is_control)
        || pairs
            .iter()
            .any(|(key, value)| key != "token" && (key != "interactions" || value != "none"))
        || pairs
            .iter()
            .filter(|(key, _)| key == "interactions")
            .count()
            > 1
    {
        return Err("Invalid DSH connection link parameters".into());
    }
    let token = tokens[0].1.to_string();
    // A connection created by this integration must never start an interaction UI.
    url.query_pairs_mut()
        .clear()
        .append_pair("token", &token)
        .append_pair("interactions", "none");
    Ok(PairingUrl {
        connect: url,
        origin,
    })
}

pub(super) fn mux_redirect(connect: &Url, location: &str) -> Result<Url, String> {
    let mut mux = connect
        .join(location)
        .map_err(|_| "Invalid DSH pairing redirect")?;
    let origin = loopback_origin(connect)?;
    if loopback_origin(&mux)? != origin || mux.path() != "/api/remote.mux" || mux.query().is_some()
    {
        return Err("DSH pairing redirect must stay on the same origin and gateway path".into());
    }
    let scheme = if mux.scheme() == "https" { "wss" } else { "ws" };
    mux.set_scheme(scheme)
        .map_err(|_| "Invalid DSH WebSocket URL")?;
    Ok(mux)
}

pub(super) fn socket_address(url: &Url) -> Result<SocketAddr, String> {
    let ip = match url.host() {
        Some(::url::Host::Ipv4(ip)) if ip.is_loopback() => IpAddr::V4(ip),
        Some(::url::Host::Ipv6(ip)) if ip.is_loopback() => IpAddr::V6(ip),
        Some(::url::Host::Domain("localhost")) => IpAddr::V4(Ipv4Addr::LOCALHOST),
        _ => return Err("DSH WebSocket requires a loopback address".into()),
    };
    Ok(SocketAddr::new(
        ip,
        url.port_or_known_default().ok_or("Invalid DSH port")?,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_only_explicit_loopback_pairing() {
        for host in ["127.0.0.1", "127.0.0.2", "[::1]", "localhost"] {
            let pair = pairing_url(&format!("http://{host}:50479/connect?token=secret")).unwrap();
            assert_eq!(pair.origin.path(), "/");
            assert!(pair.origin.query().is_none());
            assert_eq!(pair.connect.query_pairs().count(), 2);
        }
        for input in [
            "http://example.org/connect?token=SECRET",
            "file:///connect?token=SECRET",
            "http://127.0.0.1.evil/connect?token=SECRET",
            "http://192.168.1.1/connect?token=SECRET",
            "http://user:SECRET@127.0.0.1/connect?token=x",
            "http://127.0.0.1/connect?token=SECRET#x",
            "http://127.0.0.1/connect?token=SECRET&token=other",
            "http://127.0.0.1/connect?token=SECRET&next=x",
            "http://127.0.0.1/connect?token=SECRET&interactions=all",
            "http://127.0.0.1/?token=SECRET",
            "http://127.0.0.1/connect?token=",
            "SECRET",
        ] {
            let error = pairing_url(input).err().expect(input);
            assert!(!error.contains("SECRET"));
        }
    }

    #[test]
    fn rejects_cross_origin_or_unexpected_redirects_without_leaking() {
        let pair = pairing_url("http://127.0.0.1:50479/connect?token=SECRET").unwrap();
        let mux = mux_redirect(&pair.connect, "./api/remote.mux").unwrap();
        assert_eq!(mux.as_str(), "ws://127.0.0.1:50479/api/remote.mux");
        for location in [
            "http://evil.org/api/remote.mux",
            "http://127.0.0.1:2/api/remote.mux",
            "//localhost:50479/api/remote.mux",
            "/api/remote.mux?token=SECRET",
            "/other",
            "http://u:SECRET@127.0.0.1:50479/api/remote.mux",
        ] {
            let error = mux_redirect(&pair.connect, location).unwrap_err();
            assert!(!error.contains("SECRET"));
        }
    }
}
