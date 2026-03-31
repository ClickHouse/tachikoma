use std::convert::Infallible;
use std::sync::Arc;

use bytes::Bytes;
use http_body_util::{BodyExt, Full};
use hyper::body::Incoming;
use hyper::{Method, Request, Response, StatusCode};
use hyper_tls::HttpsConnector;
use hyper_util::client::legacy::{Client, connect::HttpConnector};
use hyper_util::rt::TokioExecutor;

use super::credentials::CredentialCache;
use crate::provision::credentials::CredentialSource;

pub type BoxBody = http_body_util::combinators::BoxBody<Bytes, hyper::Error>;
pub type HttpsClient = Client<HttpsConnector<HttpConnector>, Full<Bytes>>;

pub fn build_client() -> HttpsClient {
    let connector = HttpsConnector::new();
    Client::builder(TokioExecutor::new()).build(connector)
}

fn static_body(s: &'static str) -> BoxBody {
    Full::new(Bytes::from(s))
        .map_err(|e: Infallible| match e {})
        .boxed()
}

fn error_response(status: StatusCode, msg: &'static str) -> Response<BoxBody> {
    Response::builder()
        .status(status)
        .header("content-type", "text/plain")
        .body(static_body(msg))
        .unwrap()
}

/// Try to extract an OAuth access token from a Claude credentials JSON file.
///
/// Handles the standard format:
/// `{"claudeAiOauth": {"accessToken": "<token>", ...}}`
fn extract_file_token(json: &str) -> Option<String> {
    let v: serde_json::Value = serde_json::from_str(json).ok()?;
    if let Some(t) = v
        .pointer("/claudeAiOauth/accessToken")
        .and_then(|v| v.as_str())
    {
        return Some(t.to_string());
    }
    // Fallback: direct accessToken field
    v.get("accessToken")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
}

/// Main request handler for the credential proxy.
///
/// - `GET /health` → 200 JSON status
/// - `* /v1/*` → forwarded to `api.anthropic.com` with fresh credentials
/// - everything else → 403
pub async fn handle_request(
    req: Request<Incoming>,
    cache: Arc<CredentialCache>,
    client: Arc<HttpsClient>,
) -> Result<Response<BoxBody>, Infallible> {
    let path = req.uri().path();

    // Health check endpoint (no auth required)
    if req.method() == Method::GET && path == "/health" {
        return Ok(Response::builder()
            .status(StatusCode::OK)
            .header("content-type", "application/json")
            .body(static_body(r#"{"status":"ok","proxy":"tachikoma"}"#))
            .unwrap());
    }

    // Only forward /v1/* — reject everything else, including traversal attempts
    if !path.starts_with("/v1/") || path.contains("..") {
        return Ok(error_response(
            StatusCode::FORBIDDEN,
            "Only /v1/* paths are forwarded by this proxy",
        ));
    }

    // Resolve fresh credentials from the cache
    let creds = cache.get().await;
    if creds.is_none() {
        tracing::warn!("Credential proxy: no credentials available");
        return Ok(error_response(
            StatusCode::SERVICE_UNAVAILABLE,
            "No credentials available on host",
        ));
    }

    // Build the upstream URL
    let path_and_query = req
        .uri()
        .path_and_query()
        .map(|pq| pq.as_str())
        .unwrap_or(path);
    let upstream_url = format!("https://api.anthropic.com{path_and_query}");

    // Consume and buffer the incoming request body (10 MB limit)
    const MAX_BODY_BYTES: usize = 10 * 1024 * 1024;
    let (parts, body) = req.into_parts();
    let body_bytes = match http_body_util::Limited::new(body, MAX_BODY_BYTES)
        .collect()
        .await
    {
        Ok(collected) => collected.to_bytes(),
        Err(_) => {
            return Ok(error_response(
                StatusCode::PAYLOAD_TOO_LARGE,
                "Request body exceeds 10 MB limit",
            ));
        }
    };

    // Start building the upstream request
    let mut upstream_builder = Request::builder().method(parts.method).uri(upstream_url);

    // Forward all original headers except Host (set by hyper) and Authorization/x-api-key
    // (which we inject below based on the resolved credential source)
    for (name, value) in &parts.headers {
        let lname = name.as_str();
        if lname != "host" && lname != "authorization" && lname != "x-api-key" {
            upstream_builder = upstream_builder.header(name, value);
        }
    }

    // Inject the auth header appropriate for the credential source
    upstream_builder = match &creds {
        CredentialSource::Keychain(key)
        | CredentialSource::ApiKey(key)
        | CredentialSource::ApiKeyCommand(key) => {
            upstream_builder.header("x-api-key", key.as_str())
        }

        CredentialSource::EnvVar(token) | CredentialSource::Command(token) => {
            upstream_builder.header("authorization", format!("Bearer {token}"))
        }

        CredentialSource::File(json_data) => match extract_file_token(json_data) {
            Some(token) => upstream_builder.header("authorization", format!("Bearer {token}")),
            None => {
                tracing::warn!(
                    "Credential proxy: cannot extract access token from credentials file"
                );
                return Ok(error_response(
                    StatusCode::SERVICE_UNAVAILABLE,
                    "Cannot extract OAuth token from credentials file",
                ));
            }
        },

        CredentialSource::ProxyEnv { .. } => {
            // ProxyEnv delegates to a different backend (Bedrock/Vertex);
            // proxying to api.anthropic.com is not applicable.
            return Ok(error_response(
                StatusCode::SERVICE_UNAVAILABLE,
                "ProxyEnv credentials are not compatible with the credential proxy",
            ));
        }

        CredentialSource::None => {
            return Ok(error_response(
                StatusCode::SERVICE_UNAVAILABLE,
                "No credentials available on host",
            ));
        }
    };

    let upstream_req = match upstream_builder.body(Full::new(body_bytes)) {
        Ok(r) => r,
        Err(e) => {
            tracing::error!("Failed to build upstream request: {e}");
            return Ok(error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "Failed to build upstream request",
            ));
        }
    };

    // Forward to api.anthropic.com and stream the response back
    match client.request(upstream_req).await {
        Ok(response) => {
            let status = response.status();
            tracing::debug!("Upstream response: {status}");
            let (resp_parts, resp_body) = response.into_parts();
            Ok(Response::from_parts(resp_parts, resp_body.boxed()))
        }
        Err(e) => {
            tracing::error!("Upstream request to api.anthropic.com failed: {e}");
            Ok(error_response(
                StatusCode::BAD_GATEWAY,
                "Upstream request to api.anthropic.com failed",
            ))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_path_allowlist_logic() {
        let allowed = ["/v1/messages", "/v1/complete", "/v1/models"];
        let blocked = ["/health", "/admin", "/", "/v2/messages", "v1/messages"];
        for p in &allowed {
            assert!(
                p.starts_with("/v1/") && !p.contains(".."),
                "{p} should be allowed"
            );
        }
        for p in &blocked {
            assert!(
                !p.starts_with("/v1/") || p.contains(".."),
                "{p} should be blocked"
            );
        }
    }

    #[test]
    fn test_path_traversal_blocked() {
        let traversal_paths = ["/v1/../admin", "/v1/../../etc/passwd"];
        for p in &traversal_paths {
            assert!(p.contains(".."), "{p} should be blocked by traversal check");
        }
    }

    #[test]
    fn test_extract_file_token_standard_format() {
        let json = r#"{"claudeAiOauth":{"accessToken":"sk-ant-oau01-test","expiresAt":"2099-01-01","refreshToken":"rt-test"}}"#;
        assert_eq!(
            extract_file_token(json),
            Some("sk-ant-oau01-test".to_string())
        );
    }

    #[test]
    fn test_extract_file_token_fallback_format() {
        let json = r#"{"accessToken":"direct-token"}"#;
        assert_eq!(extract_file_token(json), Some("direct-token".to_string()));
    }

    #[test]
    fn test_extract_file_token_invalid_json() {
        assert_eq!(extract_file_token("not json"), None);
    }

    #[test]
    fn test_extract_file_token_missing_field() {
        let json = r#"{"claudeAiOauth":{"refreshToken":"rt-only"}}"#;
        assert_eq!(extract_file_token(json), None);
    }
}
