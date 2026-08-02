use axum::http::HeaderMap;
use serde::Deserialize;

use crate::server::ApiError;

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TokenGrant {
    pub token: String,
    #[serde(default)]
    pub read: Vec<String>,
    #[serde(default)]
    pub write: Vec<String>,
}

#[derive(Clone)]
pub struct Authorizer {
    grants: Vec<TokenGrant>,
    allow_anonymous: bool,
}

#[derive(Clone, Copy)]
pub enum Access {
    Read,
    Write,
}

impl Authorizer {
    pub fn new(json: Option<&str>, allow_anonymous: bool) -> anyhow::Result<Self> {
        let grants = match json {
            Some(json) => serde_json::from_str(json)?,
            None => Vec::new(),
        };
        Ok(Self {
            grants,
            allow_anonymous,
        })
    }

    pub fn authorize(&self, headers: &HeaderMap, access: Access) -> Result<String, ApiError> {
        if headers
            .get("mise-cache-protocol")
            .and_then(|value| value.to_str().ok())
            != Some("1")
        {
            return Err(ApiError::upgrade_required());
        }
        let namespace = headers
            .get("mise-cache-namespace")
            .and_then(|value| value.to_str().ok())
            .filter(|value| valid_namespace(value))
            .ok_or_else(|| {
                ApiError::bad_request("a valid Mise-Cache-Namespace header is required")
            })?;

        if self.allow_anonymous && self.grants.is_empty() {
            return Ok(namespace.to_owned());
        }
        let token = headers
            .get(axum::http::header::AUTHORIZATION)
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.strip_prefix("Bearer "))
            .ok_or_else(|| ApiError::unauthorized("a bearer token is required"))?;
        let grant = self
            .grants
            .iter()
            .find(|grant| constant_time_eq(grant.token.as_bytes(), token.as_bytes()))
            .ok_or_else(|| ApiError::unauthorized("invalid bearer token"))?;
        let patterns = match access {
            Access::Read => &grant.read,
            Access::Write => &grant.write,
        };
        if patterns
            .iter()
            .any(|pattern| matches_namespace(pattern, namespace))
        {
            Ok(namespace.to_owned())
        } else {
            Err(ApiError::forbidden(
                "token is not authorized for this namespace",
            ))
        }
    }
}

fn valid_namespace(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 255
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'/'))
        && value
            .split('/')
            .all(|component| !component.is_empty() && component != "." && component != "..")
}

fn matches_namespace(pattern: &str, namespace: &str) -> bool {
    pattern == "*"
        || pattern == namespace
        || pattern.strip_suffix("/*").is_some_and(|prefix| {
            namespace.starts_with(prefix) && namespace.as_bytes().get(prefix.len()) == Some(&b'/')
        })
}

fn constant_time_eq(left: &[u8], right: &[u8]) -> bool {
    if left.len() != right.len() {
        return false;
    }
    left.iter()
        .zip(right)
        .fold(0_u8, |diff, (a, b)| diff | (a ^ b))
        == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn namespace_patterns_do_not_overmatch() {
        assert!(matches_namespace("acme/*", "acme/project"));
        assert!(!matches_namespace("acme/*", "acme"));
        assert!(!matches_namespace("acme/*", "acme-other/project"));
        assert!(matches_namespace("*", "anything"));
    }

    #[test]
    fn rejects_unsafe_namespaces() {
        assert!(valid_namespace("acme/project-a"));
        assert!(!valid_namespace("../project"));
        assert!(!valid_namespace("project name"));
    }
}
