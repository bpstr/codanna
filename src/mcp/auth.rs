//! Operator authentication for both network transports.
//!
//! This is a single-trust-domain bearer credential, not an OAuth server or
//! multi-tenant identity system. Stdio continues to use the local process boundary.

use axum::{
    Router,
    extract::{Request, State},
    http::{HeaderMap, StatusCode, header},
    middleware::{self, Next},
    response::{IntoResponse, Response},
};
use sha2::{Digest, Sha256};
use std::net::SocketAddr;
use subtle::ConstantTimeEq;

#[derive(Clone)]
pub(crate) struct NetworkAuth {
    digest: [u8; 32],
}

impl NetworkAuth {
    pub(crate) fn from_env() -> anyhow::Result<Self> {
        let token = std::env::var("CODANNA_MCP_TOKEN").map_err(|_| {
            anyhow::anyhow!("Network MCP requires CODANNA_MCP_TOKEN. Configure a random bearer token of at least 32 characters; no default token is accepted.")
        })?;
        Self::new(&token)
    }

    fn new(token: &str) -> anyhow::Result<Self> {
        anyhow::ensure!(
            (32..=512).contains(&token.len())
                && token
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"-._~+/=".contains(&b)),
            "CODANNA_MCP_TOKEN must contain 32-512 bearer-token characters; generate a random credential"
        );
        Ok(Self {
            digest: Sha256::digest(token.as_bytes()).into(),
        })
    }

    fn accepts(&self, headers: &HeaderMap) -> bool {
        let mut values = headers.get_all(header::AUTHORIZATION).iter();
        let Some(value) = values.next() else {
            return false;
        };
        if values.next().is_some() {
            return false;
        }
        let Ok(value) = value.to_str() else {
            return false;
        };
        let Some((scheme, token)) = value.split_once(' ') else {
            return false;
        };
        if !scheme.eq_ignore_ascii_case("Bearer") || !(32..=512).contains(&token.len()) {
            return false;
        }
        let supplied: [u8; 32] = Sha256::digest(token.as_bytes()).into();
        bool::from(self.digest.ct_eq(&supplied))
    }
}

/// Plain HTTP is local-only even with authentication: a bearer token must not
/// travel in cleartext over a network. TLS termination may proxy to loopback.
pub(crate) fn validate_bind(bind: &str, tls: bool) -> anyhow::Result<SocketAddr> {
    let address: SocketAddr = bind
        .parse()
        .map_err(|_| anyhow::anyhow!("MCP bind must be an IP socket address"))?;
    anyhow::ensure!(
        tls || address.ip().is_loopback(),
        "Non-loopback MCP requires HTTPS; plain HTTP may bind only to loopback"
    );
    Ok(address)
}

async fn authorize(State(auth): State<NetworkAuth>, request: Request, next: Next) -> Response {
    if auth.accepts(request.headers()) {
        next.run(request).await
    } else {
        // Never log headers, cookies, query parameters, bodies or token values.
        (
            StatusCode::UNAUTHORIZED,
            [(header::WWW_AUTHENTICATE, "Bearer")],
        )
            .into_response()
    }
}

/// Only the health endpoint is public. No token issuance, callback redirects,
/// registration or script-bearing authorization pages are exposed.
pub(crate) fn network_router(mcp: Router, auth: NetworkAuth) -> Router {
    let protected = mcp.route_layer(middleware::from_fn_with_state(auth, authorize));
    Router::new()
        .route("/health", axum::routing::get(|| async { "OK" }))
        .merge(protected)
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use tower::ServiceExt;

    const TOKEN: &str = "review-fixture-credential-not-for-production-123456";

    #[test]
    fn hardening_review_network_auth_has_no_default_or_ambiguous_header() {
        assert!(NetworkAuth::new("").is_err());
        assert!(NetworkAuth::new("mcp-access-token-dummy").is_err());
        let auth = NetworkAuth::new(TOKEN).unwrap();
        let mut headers = HeaderMap::new();
        assert!(!auth.accepts(&headers));
        headers.insert(
            header::AUTHORIZATION,
            format!("Bearer {TOKEN}").parse().unwrap(),
        );
        assert!(auth.accepts(&headers));
        headers.append(
            header::AUTHORIZATION,
            format!("Bearer {TOKEN}").parse().unwrap(),
        );
        assert!(!auth.accepts(&headers));
    }

    #[test]
    fn hardening_review_network_bind_requires_tls_off_loopback() {
        assert!(validate_bind("127.0.0.1:8080", false).is_ok());
        assert!(validate_bind("[::1]:8080", false).is_ok());
        assert!(validate_bind("0.0.0.0:8080", false).is_err());
        assert!(validate_bind("[::]:8080", false).is_err());
        assert!(validate_bind("0.0.0.0:8443", true).is_ok());
    }

    #[tokio::test]
    async fn hardening_review_network_routes_require_auth_and_remove_oauth() {
        let app = network_router(
            Router::new().route("/mcp", axum::routing::any(|| async { "protected" })),
            NetworkAuth::new(TOKEN).unwrap(),
        );
        for method in ["GET", "POST", "DELETE", "OPTIONS"] {
            for token in [
                None,
                Some("Bearer mcp-access-token-dummy"),
                Some("Bearer wrong-token-that-is-long-enough-123456"),
            ] {
                let mut builder = axum::http::Request::builder().method(method).uri("/mcp");
                if let Some(value) = token {
                    builder = builder.header(header::AUTHORIZATION, value);
                }
                let response = app
                    .clone()
                    .oneshot(builder.body(Body::empty()).unwrap())
                    .await
                    .unwrap();
                assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
            }
        }
        let response = app
            .clone()
            .oneshot(
                axum::http::Request::builder()
                    .uri("/mcp")
                    .header(header::AUTHORIZATION, format!("Bearer {TOKEN}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        for path in [
            "/oauth/token",
            "/oauth/register",
            "/oauth/authorize?redirect_uri=https://invalid.example&state=%3Cscript%3E",
            "/.well-known/oauth-authorization-server",
        ] {
            let response = app
                .clone()
                .oneshot(
                    axum::http::Request::builder()
                        .uri(path)
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::NOT_FOUND);
        }
        let response = app
            .oneshot(
                axum::http::Request::builder()
                    .uri("/health")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
    }
}
