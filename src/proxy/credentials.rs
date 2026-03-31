use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio::sync::RwLock;

use crate::provision::credentials::{CredentialSource, resolve_credentials};

struct CacheEntry {
    credentials: CredentialSource,
    fetched_at: Instant,
}

/// TTL-based credential cache wrapping the existing `resolve_credentials` waterfall.
///
/// Read-locks for cache hits, write-locks only for refresh — no thundering herd.
pub struct CredentialCache {
    credential_command: Option<String>,
    api_key_command: Option<String>,
    ttl: Duration,
    inner: Arc<RwLock<Option<CacheEntry>>>,
}

impl CredentialCache {
    pub fn new(
        credential_command: Option<String>,
        api_key_command: Option<String>,
        ttl: Duration,
    ) -> Self {
        Self {
            credential_command,
            api_key_command,
            ttl,
            inner: Arc::new(RwLock::new(None)),
        }
    }

    /// Return cached credentials, refreshing if the TTL has expired.
    pub async fn get(&self) -> CredentialSource {
        // Fast path: valid cache entry under read lock.
        {
            let guard = self.inner.read().await;
            if let Some(entry) = &*guard
                && entry.fetched_at.elapsed() < self.ttl
            {
                return entry.credentials.clone();
            }
        }

        // Slow path: refresh under write lock.
        let mut guard = self.inner.write().await;
        // Double-check: another task may have refreshed while we waited.
        if let Some(entry) = &*guard
            && entry.fetched_at.elapsed() < self.ttl
        {
            return entry.credentials.clone();
        }

        let credentials = resolve_credentials(
            self.credential_command.as_deref(),
            self.api_key_command.as_deref(),
        )
        .await;

        tracing::info!(
            "Credential proxy cache refreshed: source={}",
            credentials.label()
        );

        *guard = Some(CacheEntry {
            credentials: credentials.clone(),
            fetched_at: Instant::now(),
        });

        credentials
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_cache_returns_credentials() {
        // Use api_key_command with echo so we don't touch global env vars
        // (which would race with parallel tests that also mutate them).
        let cache = CredentialCache::new(
            None,
            Some("echo sk-test-cmd-key".to_string()),
            Duration::from_secs(60),
        );
        let creds = cache.get().await;

        assert!(
            !matches!(creds, CredentialSource::None),
            "Expected some credentials, got None"
        );
    }

    #[tokio::test]
    async fn test_cache_hit_on_second_call() {
        let cache = CredentialCache::new(None, None, Duration::from_secs(300));
        let first = cache.get().await;
        let second = cache.get().await;
        // Both calls should return the same source label (cache hit)
        assert_eq!(first.label(), second.label());
    }
}
