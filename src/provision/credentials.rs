/// Credential source, in priority order
#[derive(Debug, Clone, PartialEq)]
pub enum CredentialSource {
    Keychain(String),
    EnvVar(String),
    Command(String),
    File(String),
    ApiKey(String),
    ApiKeyCommand(String),
    ProxyEnv {
        provider: String,
        vars: Vec<(String, String)>,
    },
    None,
}

impl CredentialSource {
    pub fn is_none(&self) -> bool {
        matches!(self, CredentialSource::None)
    }

    pub fn label(&self) -> &'static str {
        match self {
            CredentialSource::Keychain(_) => "macOS Keychain",
            CredentialSource::EnvVar(_) => "CLAUDE_CODE_OAUTH_TOKEN env var",
            CredentialSource::Command(_) => "credential command",
            CredentialSource::File(_) => "credentials file",
            CredentialSource::ApiKey(_) => "ANTHROPIC_API_KEY env var",
            CredentialSource::ApiKeyCommand(_) => "API key command",
            CredentialSource::ProxyEnv { provider, .. } => match provider.as_str() {
                "bedrock" => "AWS Bedrock",
                "vertex" => "Google Vertex",
                _ => "proxy",
            },
            CredentialSource::None => "none",
        }
    }
}

/// Credential waterfall: try each source in priority order, return first match.
pub async fn resolve_credentials(
    credential_command: Option<&str>,
    api_key_command: Option<&str>,
) -> CredentialSource {
    // 1. macOS Keychain
    if let Some(cred) = try_keychain().await {
        return CredentialSource::Keychain(cred);
    }

    // 2. CLAUDE_CODE_OAUTH_TOKEN env var
    if let Ok(token) = std::env::var("CLAUDE_CODE_OAUTH_TOKEN") {
        if !token.is_empty() {
            return CredentialSource::EnvVar(token);
        }
    }

    // 3. Configured credential command
    if let Some(cmd) = credential_command {
        if let Some(cred) = try_command(cmd).await {
            return CredentialSource::Command(cred);
        }
    }

    // 4. ~/.claude/.credentials.json file
    if let Some(cred) = try_credentials_file().await {
        return CredentialSource::File(cred);
    }

    // 5. ANTHROPIC_API_KEY env var
    if let Ok(key) = std::env::var("ANTHROPIC_API_KEY") {
        if !key.is_empty() {
            return CredentialSource::ApiKey(key);
        }
    }

    // 6. Configured API key command
    if let Some(cmd) = api_key_command {
        if let Some(key) = try_command(cmd).await {
            return CredentialSource::ApiKeyCommand(key);
        }
    }

    // 7. Bedrock/Vertex/Proxy env vars
    if let Some(proxy) = try_proxy_env() {
        return proxy;
    }

    // 8. None
    CredentialSource::None
}

async fn try_keychain() -> Option<String> {
    let output = tokio::process::Command::new("security")
        .args([
            "find-generic-password",
            "-s",
            "Claude Code-credentials",
            "-w",
        ])
        .output()
        .await
        .ok()?;

    if output.status.success() {
        let cred = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if !cred.is_empty() {
            return Some(cred);
        }
    }
    None
}

async fn try_command(cmd: &str) -> Option<String> {
    let output = tokio::process::Command::new("sh")
        .args(["-c", cmd])
        .output()
        .await
        .ok()?;

    if output.status.success() {
        let result = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if !result.is_empty() {
            return Some(result);
        }
    }
    None
}

async fn try_credentials_file() -> Option<String> {
    let home = dirs::home_dir()?;
    let cred_path = home.join(".claude").join(".credentials.json");
    let contents = tokio::fs::read_to_string(&cred_path).await.ok()?;
    if !contents.trim().is_empty() {
        Some(contents)
    } else {
        None
    }
}

fn try_proxy_env() -> Option<CredentialSource> {
    let proxy_vars = [
        ("CLAUDE_CODE_USE_BEDROCK", "bedrock"),
        ("CLAUDE_CODE_USE_VERTEX", "vertex"),
        ("ANTHROPIC_BASE_URL", "proxy"),
    ];

    for (var, provider) in &proxy_vars {
        if let Ok(val) = std::env::var(var) {
            if !val.is_empty() {
                let mut vars = vec![(var.to_string(), val)];
                // Collect related env vars
                match *provider {
                    "bedrock" => {
                        for key in ["AWS_REGION", "AWS_ACCESS_KEY_ID", "AWS_SECRET_ACCESS_KEY"] {
                            if let Ok(v) = std::env::var(key) {
                                vars.push((key.to_string(), v));
                            }
                        }
                    }
                    "vertex" => {
                        for key in [
                            "CLOUD_ML_REGION",
                            "ANTHROPIC_VERTEX_PROJECT_ID",
                            "GOOGLE_APPLICATION_CREDENTIALS",
                        ] {
                            if let Ok(v) = std::env::var(key) {
                                vars.push((key.to_string(), v));
                            }
                        }
                    }
                    _ => {}
                }
                return Some(CredentialSource::ProxyEnv {
                    provider: provider.to_string(),
                    vars,
                });
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_waterfall_with_no_credentials() {
        // Clear relevant env vars for this test
        std::env::remove_var("CLAUDE_CODE_OAUTH_TOKEN");
        std::env::remove_var("ANTHROPIC_API_KEY");
        std::env::remove_var("CLAUDE_CODE_USE_BEDROCK");
        std::env::remove_var("CLAUDE_CODE_USE_VERTEX");
        std::env::remove_var("ANTHROPIC_BASE_URL");

        let result = resolve_credentials(None, None).await;
        // May find keychain or credentials file, but with no env vars
        // the result should be predictable in test environment
        // Just verify it doesn't panic and returns a valid source
        match result {
            CredentialSource::None
            | CredentialSource::Keychain(_)
            | CredentialSource::File(_) => {}
            other => panic!("Unexpected credential source: {other:?}"),
        }
    }

    #[tokio::test]
    async fn test_waterfall_env_var_priority() {
        std::env::set_var("CLAUDE_CODE_OAUTH_TOKEN", "test-oauth-token");
        let result = resolve_credentials(None, None).await;
        std::env::remove_var("CLAUDE_CODE_OAUTH_TOKEN");

        // Should find the env var (unless keychain succeeds first)
        assert!(
            matches!(result, CredentialSource::Keychain(_) | CredentialSource::EnvVar(_)),
            "Expected Keychain or EnvVar, got: {result:?}"
        );
    }

    #[tokio::test]
    async fn test_waterfall_api_key() {
        std::env::remove_var("CLAUDE_CODE_OAUTH_TOKEN");
        std::env::set_var("ANTHROPIC_API_KEY", "sk-test-key");
        let result = resolve_credentials(None, None).await;
        std::env::remove_var("ANTHROPIC_API_KEY");

        assert!(
            matches!(
                result,
                CredentialSource::Keychain(_)
                    | CredentialSource::File(_)
                    | CredentialSource::ApiKey(_)
            ),
            "Expected Keychain, File, or ApiKey, got: {result:?}"
        );
    }

    #[tokio::test]
    async fn test_credential_command() {
        let result = resolve_credentials(Some("echo test-cred"), None).await;
        assert!(
            matches!(
                result,
                CredentialSource::Keychain(_)
                    | CredentialSource::Command(_)
                    | CredentialSource::File(_)
            ),
            "Expected command result, got: {result:?}"
        );
    }

    #[test]
    fn test_credential_source_is_none() {
        assert!(CredentialSource::None.is_none());
        assert!(!CredentialSource::ApiKey("key".into()).is_none());
    }
}
