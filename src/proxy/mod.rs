pub mod credentials;
pub mod server;

use std::net::SocketAddr;
use std::sync::Arc;

use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper_util::rt::TokioIo;

use credentials::CredentialCache;

/// Configuration for the credential proxy server.
pub struct ProxyConfig {
    /// Address to bind to (e.g. "192.168.64.1")
    pub bind: String,
    /// Port to listen on (e.g. 19280)
    pub port: u16,
    /// Credential cache TTL in seconds
    pub ttl_secs: u64,
    /// Optional shell command to resolve OAuth token
    pub credential_command: Option<String>,
    /// Optional shell command to resolve API key
    pub api_key_command: Option<String>,
}

/// Start the credential proxy, accepting connections indefinitely.
///
/// Binds to `config.bind:config.port` and forwards `/v1/*` requests to
/// `api.anthropic.com` with fresh credentials resolved from the host waterfall.
pub async fn start_proxy(config: ProxyConfig) -> crate::Result<()> {
    let cache = Arc::new(CredentialCache::new(
        config.credential_command,
        config.api_key_command,
        std::time::Duration::from_secs(config.ttl_secs),
    ));

    let client = Arc::new(server::build_client());

    let addr: SocketAddr = format!("{}:{}", config.bind, config.port)
        .parse()
        .map_err(|e| {
            crate::TachikomaError::Proxy(format!(
                "Invalid proxy bind address '{}:{}': {e}",
                config.bind, config.port
            ))
        })?;

    let listener = tokio::net::TcpListener::bind(addr).await.map_err(|e| {
        crate::TachikomaError::Proxy(format!("Failed to bind credential proxy to {addr}: {e}"))
    })?;

    tracing::info!("Credential proxy listening on http://{addr}");
    println!("Credential proxy listening on http://{addr}");

    loop {
        let (stream, peer) = match listener.accept().await {
            Ok(conn) => conn,
            Err(e) => {
                // Transient errors (EMFILE, ENFILE, etc.) should not kill the server.
                tracing::warn!("Credential proxy: failed to accept connection: {e}");
                continue;
            }
        };
        tracing::debug!("Proxy: connection from {peer}");

        let cache = Arc::clone(&cache);
        let client = Arc::clone(&client);

        tokio::spawn(async move {
            let io = TokioIo::new(stream);
            let svc = service_fn(move |req| {
                let cache = Arc::clone(&cache);
                let client = Arc::clone(&client);
                server::handle_request(req, cache, client)
            });

            if let Err(e) = http1::Builder::new().serve_connection(io, svc).await {
                tracing::debug!("Proxy connection closed: {e}");
            }
        });
    }
}

/// TCP-probe whether the proxy is reachable at the given address and port.
/// Returns `true` if a connection can be established.
pub async fn is_proxy_reachable(bind: &str, port: u16) -> bool {
    tokio::net::TcpStream::connect(format!("{bind}:{port}"))
        .await
        .is_ok()
}
