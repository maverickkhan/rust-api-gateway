//! Per-route authentication: API keys and HS256 JWT validation.

use gw_core::AuthPolicy;
use http::HeaderMap;
use jsonwebtoken::{decode, Algorithm, DecodingKey, Validation};
use std::collections::HashMap;

/// Check a request against a route's auth policy. `Ok(())` means authorized.
pub fn authorize(policy: &AuthPolicy, headers: &HeaderMap) -> Result<(), &'static str> {
    match policy {
        AuthPolicy::None => Ok(()),
        AuthPolicy::ApiKey { keys } => {
            let presented = extract_api_key(headers).ok_or("missing API key")?;
            if keys.iter().any(|k| k == &presented) {
                Ok(())
            } else {
                Err("invalid API key")
            }
        }
        AuthPolicy::Jwt {
            secret,
            required_claims,
        } => validate_jwt(headers, secret, required_claims),
    }
}

fn extract_api_key(headers: &HeaderMap) -> Option<String> {
    if let Some(v) = headers.get("x-api-key").and_then(|v| v.to_str().ok()) {
        if !v.is_empty() {
            return Some(v.to_string());
        }
    }
    let auth = headers.get(http::header::AUTHORIZATION)?.to_str().ok()?;
    let key = auth.strip_prefix("Bearer ").unwrap_or(auth).trim();
    (!key.is_empty()).then(|| key.to_string())
}

fn bearer_token(headers: &HeaderMap) -> Option<String> {
    let auth = headers.get(http::header::AUTHORIZATION)?.to_str().ok()?;
    auth.strip_prefix("Bearer ")
        .map(|t| t.trim().to_string())
        .filter(|t| !t.is_empty())
}

fn validate_jwt(
    headers: &HeaderMap,
    secret: &str,
    required: &HashMap<String, String>,
) -> Result<(), &'static str> {
    let token = bearer_token(headers).ok_or("missing bearer token")?;
    let mut validation = Validation::new(Algorithm::HS256);
    validation.validate_exp = true;
    // Don't require a specific audience; callers gate via required_claims.
    validation.validate_aud = false;

    let data = decode::<HashMap<String, serde_json::Value>>(
        &token,
        &DecodingKey::from_secret(secret.as_bytes()),
        &validation,
    )
    .map_err(|_| "invalid or expired token")?;

    for (k, want) in required {
        let got = data.claims.get(k).and_then(value_as_string);
        if got.as_deref() != Some(want.as_str()) {
            return Err("required claim not satisfied");
        }
    }
    Ok(())
}

fn value_as_string(v: &serde_json::Value) -> Option<String> {
    match v {
        serde_json::Value::String(s) => Some(s.clone()),
        serde_json::Value::Number(n) => Some(n.to_string()),
        serde_json::Value::Bool(b) => Some(b.to_string()),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use jsonwebtoken::{encode, EncodingKey, Header};
    use std::collections::HashMap;

    fn headers_with(name: &str, value: &str) -> HeaderMap {
        let mut h = HeaderMap::new();
        h.insert(
            http::HeaderName::from_bytes(name.as_bytes()).unwrap(),
            value.parse().unwrap(),
        );
        h
    }

    #[test]
    fn api_key_accepts_and_rejects() {
        let policy = AuthPolicy::ApiKey {
            keys: vec!["good".into()],
        };
        assert!(authorize(&policy, &headers_with("x-api-key", "good")).is_ok());
        assert!(authorize(&policy, &headers_with("x-api-key", "bad")).is_err());
        assert!(authorize(&policy, &HeaderMap::new()).is_err());
    }

    #[test]
    fn none_policy_allows_all() {
        assert!(authorize(&AuthPolicy::None, &HeaderMap::new()).is_ok());
    }

    #[test]
    fn jwt_valid_token_is_accepted() {
        let secret = "topsecret";
        let mut claims: HashMap<String, serde_json::Value> = HashMap::new();
        claims.insert("sub".into(), "user-1".into());
        claims.insert("role".into(), "admin".into());
        // exp far in the future.
        claims.insert("exp".into(), serde_json::json!(9_999_999_999i64));
        let token = encode(
            &Header::new(Algorithm::HS256),
            &claims,
            &EncodingKey::from_secret(secret.as_bytes()),
        )
        .unwrap();

        let mut required = HashMap::new();
        required.insert("role".to_string(), "admin".to_string());
        let policy = AuthPolicy::Jwt {
            secret: secret.into(),
            required_claims: required,
        };
        assert!(authorize(
            &policy,
            &headers_with("authorization", &format!("Bearer {token}"))
        )
        .is_ok());
    }

    #[test]
    fn jwt_wrong_secret_is_rejected() {
        let mut claims: HashMap<String, serde_json::Value> = HashMap::new();
        claims.insert("exp".into(), serde_json::json!(9_999_999_999i64));
        let token = encode(
            &Header::new(Algorithm::HS256),
            &claims,
            &EncodingKey::from_secret(b"other"),
        )
        .unwrap();
        let policy = AuthPolicy::Jwt {
            secret: "topsecret".into(),
            required_claims: HashMap::new(),
        };
        assert!(authorize(
            &policy,
            &headers_with("authorization", &format!("Bearer {token}"))
        )
        .is_err());
    }

    #[test]
    fn jwt_missing_required_claim_is_rejected() {
        let secret = "s";
        let mut claims: HashMap<String, serde_json::Value> = HashMap::new();
        claims.insert("exp".into(), serde_json::json!(9_999_999_999i64));
        claims.insert("role".into(), "user".into());
        let token = encode(
            &Header::new(Algorithm::HS256),
            &claims,
            &EncodingKey::from_secret(secret.as_bytes()),
        )
        .unwrap();
        let mut required = HashMap::new();
        required.insert("role".to_string(), "admin".to_string());
        let policy = AuthPolicy::Jwt {
            secret: secret.into(),
            required_claims: required,
        };
        assert!(authorize(
            &policy,
            &headers_with("authorization", &format!("Bearer {token}"))
        )
        .is_err());
    }
}
