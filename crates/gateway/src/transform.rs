//! Header handling: hop-by-hop stripping, forwarding headers, and configured
//! add/remove transforms.

use gw_core::HeaderTransforms;
use http::header::{HeaderName, HeaderValue};
use http::HeaderMap;

/// Hop-by-hop headers that must not be forwarded (RFC 7230 §6.1) plus `Host`,
/// which the client sets per upstream.
const HOP_BY_HOP: &[&str] = &[
    "connection",
    "keep-alive",
    "proxy-authenticate",
    "proxy-authorization",
    "te",
    "trailer",
    "transfer-encoding",
    "upgrade",
    "host",
    "content-length",
];

pub fn is_hop_by_hop(name: &str) -> bool {
    HOP_BY_HOP.iter().any(|h| name.eq_ignore_ascii_case(h))
}

/// Build the header set to forward upstream: copy end-to-end headers, drop
/// hop-by-hop, and add `X-Forwarded-*` / `X-Real-IP`.
pub fn forwarded_request_headers(
    incoming: &HeaderMap,
    client_ip: &str,
    host: &str,
    scheme: &str,
) -> HeaderMap {
    let mut out = HeaderMap::new();
    for (name, value) in incoming.iter() {
        if !is_hop_by_hop(name.as_str()) {
            out.append(name.clone(), value.clone());
        }
    }

    // Append to any existing X-Forwarded-For chain.
    let xff = match incoming
        .get("x-forwarded-for")
        .and_then(|v| v.to_str().ok())
    {
        Some(existing) => format!("{existing}, {client_ip}"),
        None => client_ip.to_string(),
    };
    set(&mut out, "x-forwarded-for", &xff);
    set(&mut out, "x-real-ip", client_ip);
    set(&mut out, "x-forwarded-host", host);
    set(&mut out, "x-forwarded-proto", scheme);
    out
}

/// Strip hop-by-hop headers from an upstream response before returning it.
pub fn filtered_response_headers(incoming: &HeaderMap) -> HeaderMap {
    let mut out = HeaderMap::new();
    for (name, value) in incoming.iter() {
        if !is_hop_by_hop(name.as_str()) {
            out.append(name.clone(), value.clone());
        }
    }
    out
}

/// Apply configured add/remove transforms to a header map.
pub fn apply_transforms(headers: &mut HeaderMap, t: &HeaderTransforms) {
    for name in &t.remove {
        if let Ok(h) = HeaderName::from_bytes(name.as_bytes()) {
            headers.remove(&h);
        }
    }
    for (name, value) in &t.add {
        if let (Ok(h), Ok(v)) = (
            HeaderName::from_bytes(name.as_bytes()),
            HeaderValue::from_str(value),
        ) {
            headers.insert(h, v);
        }
    }
}

fn set(headers: &mut HeaderMap, name: &'static str, value: &str) {
    if let Ok(v) = HeaderValue::from_str(value) {
        headers.insert(HeaderName::from_static(name), v);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_hop_by_hop_and_adds_forwarding() {
        let mut incoming = HeaderMap::new();
        incoming.insert("connection", "keep-alive".parse().unwrap());
        incoming.insert("x-custom", "v".parse().unwrap());
        let out = forwarded_request_headers(&incoming, "1.2.3.4", "example.com", "http");
        assert!(out.get("connection").is_none(), "hop-by-hop removed");
        assert_eq!(out.get("x-custom").unwrap(), "v");
        assert_eq!(out.get("x-forwarded-for").unwrap(), "1.2.3.4");
        assert_eq!(out.get("x-real-ip").unwrap(), "1.2.3.4");
    }

    #[test]
    fn appends_to_existing_forwarded_for() {
        let mut incoming = HeaderMap::new();
        incoming.insert("x-forwarded-for", "10.0.0.1".parse().unwrap());
        let out = forwarded_request_headers(&incoming, "1.2.3.4", "h", "http");
        assert_eq!(out.get("x-forwarded-for").unwrap(), "10.0.0.1, 1.2.3.4");
    }

    #[test]
    fn add_and_remove_transforms() {
        let mut headers = HeaderMap::new();
        headers.insert("x-drop", "1".parse().unwrap());
        let mut t = HeaderTransforms::default();
        t.remove.push("x-drop".into());
        t.add.insert("x-add".into(), "yes".into());
        apply_transforms(&mut headers, &t);
        assert!(headers.get("x-drop").is_none());
        assert_eq!(headers.get("x-add").unwrap(), "yes");
    }
}
