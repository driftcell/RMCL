//! Provider presets, settings, and key storage for the built-in AI assistant.

use std::sync::Arc;

use rmcl_platform::{SecretStore, SecretStoreError};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use url::Url;

pub const AGENT_KEYRING_SERVICE: &str = "com.rmcl.launcher";

/// Providers offered by the first version of the assistant.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentProvider {
    #[default]
    OpenAi,
    Anthropic,
    Gemini,
    DeepSeek,
    OpenRouter,
    Qwen,
    Kimi,
    Zhipu,
    Custom,
}

impl AgentProvider {
    pub const ALL: [Self; 9] = [
        Self::OpenAi,
        Self::Anthropic,
        Self::Gemini,
        Self::DeepSeek,
        Self::OpenRouter,
        Self::Qwen,
        Self::Kimi,
        Self::Zhipu,
        Self::Custom,
    ];

    pub const fn id(self) -> &'static str {
        match self {
            Self::OpenAi => "openai",
            Self::Anthropic => "anthropic",
            Self::Gemini => "gemini",
            Self::DeepSeek => "deepseek",
            Self::OpenRouter => "openrouter",
            Self::Qwen => "qwen",
            Self::Kimi => "kimi",
            Self::Zhipu => "zhipu",
            Self::Custom => "custom",
        }
    }

    pub const fn label(self) -> &'static str {
        match self {
            Self::OpenAi => "OpenAI",
            Self::Anthropic => "Anthropic",
            Self::Gemini => "Google Gemini",
            Self::DeepSeek => "DeepSeek",
            Self::OpenRouter => "OpenRouter",
            Self::Qwen => "通义千问",
            Self::Kimi => "Kimi",
            Self::Zhipu => "智谱 GLM",
            Self::Custom => "自定义 OpenAI 兼容",
        }
    }

    pub const fn default_base_url(self) -> Option<&'static str> {
        match self {
            Self::OpenAi => Some("https://api.openai.com/v1"),
            Self::Anthropic => Some("https://api.anthropic.com"),
            Self::Gemini => Some("https://generativelanguage.googleapis.com/v1beta"),
            Self::DeepSeek => Some("https://api.deepseek.com/v1"),
            Self::OpenRouter => Some("https://openrouter.ai/api/v1"),
            Self::Qwen => Some("https://dashscope.aliyuncs.com/compatible-mode/v1"),
            Self::Kimi => Some("https://api.moonshot.cn/v1"),
            Self::Zhipu => Some("https://open.bigmodel.cn/api/paas/v4"),
            Self::Custom => None,
        }
    }

    pub const fn default_model(self) -> &'static str {
        match self {
            Self::OpenAi => "gpt-5.6",
            Self::Anthropic => "claude-sonnet-5-5",
            Self::Gemini => "gemini-3.8-flash",
            Self::DeepSeek => "deepseek-v4-flash",
            Self::OpenRouter => "openai/gpt-5-mini",
            Self::Qwen => "qwen3-max",
            Self::Kimi => "kimi-k3",
            Self::Zhipu => "glm-4.6",
            Self::Custom => "",
        }
    }

    pub const fn is_openai_compatible(self) -> bool {
        !matches!(self, Self::Anthropic | Self::Gemini)
    }

    pub fn from_id(value: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|provider| provider.id() == value.trim().to_ascii_lowercase())
    }
}

/// User-editable assistant settings. API keys are stored separately.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct AgentSettings {
    pub provider: AgentProvider,
    pub model: String,
    pub base_url: Option<String>,
    pub proxy: Option<String>,
}

impl AgentSettings {
    pub fn from_config(
        provider: &str,
        model: &str,
        base_url: Option<&str>,
        proxy: Option<&str>,
    ) -> Self {
        Self {
            provider: AgentProvider::from_id(provider).unwrap_or_default(),
            model: model.trim().to_owned(),
            base_url: non_empty(base_url),
            proxy: non_empty(proxy),
        }
    }

    pub fn effective_model(&self) -> Result<&str, AgentError> {
        if !self.model.trim().is_empty() {
            return Ok(self.model.trim());
        }
        let model = self.provider.default_model();
        if model.is_empty() {
            Err(AgentError::MissingModel)
        } else {
            Ok(model)
        }
    }

    pub fn effective_base_url(&self) -> Result<&str, AgentError> {
        if let Some(base_url) = self.base_url.as_deref() {
            validate_base_url(base_url)?;
            return Ok(base_url);
        }
        self.provider
            .default_base_url()
            .ok_or(AgentError::MissingBaseUrl)
    }

    pub fn proxy_url(&self) -> Result<Option<Url>, AgentError> {
        self.proxy.as_deref().map(parse_proxy_url).transpose()
    }
}

fn non_empty(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
}

fn validate_base_url(value: &str) -> Result<(), AgentError> {
    let url = Url::parse(value).map_err(|source| AgentError::InvalidBaseUrl {
        value: value.to_owned(),
        source,
    })?;
    if !matches!(url.scheme(), "http" | "https") || url.host_str().is_none() {
        return Err(AgentError::InvalidBaseUrlValue(value.to_owned()));
    }
    Ok(())
}

fn parse_proxy_url(value: &str) -> Result<Url, AgentError> {
    let url = Url::parse(value).map_err(|source| AgentError::InvalidProxyUrl {
        value: value.to_owned(),
        source,
    })?;
    if !matches!(url.scheme(), "http" | "https" | "socks5" | "socks5h") || url.host_str().is_none()
    {
        return Err(AgentError::InvalidProxyUrlValue(value.to_owned()));
    }
    Ok(url)
}

/// Stores one versioned JSON credential per provider in the system keyring.
#[derive(Clone)]
pub struct AgentKeyStore {
    secrets: Arc<dyn SecretStore>,
}

impl AgentKeyStore {
    pub fn new(secrets: Arc<dyn SecretStore>) -> Self {
        Self { secrets }
    }

    pub fn key_name(provider: AgentProvider) -> String {
        format!("agent:{}", provider.id())
    }

    pub fn load(&self, provider: AgentProvider) -> Result<Option<String>, AgentError> {
        let Some(value) = self.secrets.get(&Self::key_name(provider))? else {
            return Ok(None);
        };
        let stored: StoredAgentKey = serde_json::from_str(&value)
            .map_err(|source| AgentError::InvalidStoredKey { provider, source })?;
        if stored.version != 1 || stored.api_key.trim().is_empty() {
            return Err(AgentError::UnsupportedStoredKey {
                provider,
                version: stored.version,
            });
        }
        Ok(Some(stored.api_key))
    }

    pub fn save(&self, provider: AgentProvider, api_key: &str) -> Result<(), AgentError> {
        let api_key = api_key.trim();
        if api_key.is_empty() {
            return Err(AgentError::BlankKey);
        }
        let value = serde_json::to_string(&StoredAgentKey {
            version: 1,
            api_key: api_key.to_owned(),
        })
        .map_err(AgentError::EncodeKey)?;
        self.secrets.set(&Self::key_name(provider), &value)?;
        Ok(())
    }

    pub fn delete(&self, provider: AgentProvider) -> Result<(), AgentError> {
        self.secrets.delete(&Self::key_name(provider))?;
        Ok(())
    }

    pub fn has_key(&self, provider: AgentProvider) -> Result<bool, AgentError> {
        Ok(self.load(provider)?.is_some())
    }
}

#[derive(Debug, Serialize, Deserialize)]
struct StoredAgentKey {
    version: u32,
    api_key: String,
}

#[derive(Debug, Error)]
pub enum AgentError {
    #[error("assistant model is not configured")]
    MissingModel,
    #[error("assistant base URL is required for a custom provider")]
    MissingBaseUrl,
    #[error("assistant API key is blank")]
    BlankKey,
    #[error("invalid assistant base URL `{value}`: {source}")]
    InvalidBaseUrl {
        value: String,
        #[source]
        source: url::ParseError,
    },
    #[error("assistant base URL `{0}` must use HTTP(S) and include a host")]
    InvalidBaseUrlValue(String),
    #[error("invalid assistant proxy URL `{value}`: {source}")]
    InvalidProxyUrl {
        value: String,
        #[source]
        source: url::ParseError,
    },
    #[error("assistant proxy URL `{0}` must use HTTP(S) or SOCKS5 and include a host")]
    InvalidProxyUrlValue(String),
    #[error("stored assistant credential for {provider:?} is invalid: {source}")]
    InvalidStoredKey {
        provider: AgentProvider,
        #[source]
        source: serde_json::Error,
    },
    #[error("stored assistant credential for {provider:?} uses unsupported version {version}")]
    UnsupportedStoredKey {
        provider: AgentProvider,
        version: u32,
    },
    #[error("failed to encode assistant credential: {0}")]
    EncodeKey(#[source] serde_json::Error),
    #[error("failed to configure assistant HTTP client: {0}")]
    Http(#[source] rig::rig_reqwest::reqwest::Error),
    #[error("failed to configure assistant proxy: {0}")]
    Proxy(#[source] rig::rig_reqwest::reqwest::Error),
    #[error("assistant MCP connection failed: {0}")]
    Mcp(String),
    #[error("no API key is saved for {0:?}")]
    MissingKey(AgentProvider),
    #[error("failed to expose an assistant tool: {0}")]
    ToolConversion(String),
    #[error(transparent)]
    SecretStore(#[from] SecretStoreError),
}

#[cfg(test)]
mod tests {
    use rmcl_platform::MemorySecretStore;

    use super::*;

    #[test]
    fn provider_presets_cover_expected_defaults() {
        assert_eq!(AgentProvider::from_id("qwen"), Some(AgentProvider::Qwen));
        assert_eq!(
            AgentProvider::from_id("OPENAI"),
            Some(AgentProvider::OpenAi)
        );
        assert!(AgentProvider::Custom.default_model().is_empty());
        assert!(AgentProvider::Anthropic.default_base_url().is_some());
        assert!(AgentProvider::Gemini.default_base_url().is_some());
        assert!(AgentProvider::DeepSeek.is_openai_compatible());
    }

    #[test]
    fn settings_resolve_defaults_and_validate_proxy() {
        let settings =
            AgentSettings::from_config("deepseek", "", None, Some("socks5://127.0.0.1:7890"));
        assert_eq!(settings.effective_model().unwrap(), "deepseek-v4-flash");
        assert_eq!(
            settings.effective_base_url().unwrap(),
            "https://api.deepseek.com/v1"
        );
        assert_eq!(settings.proxy_url().unwrap().unwrap().scheme(), "socks5");

        let custom = AgentSettings::from_config("custom", "my-model", None, None);
        assert!(matches!(
            custom.effective_base_url(),
            Err(AgentError::MissingBaseUrl)
        ));
        assert!(
            AgentSettings::from_config("openai", "", None, Some("ftp://x"))
                .proxy_url()
                .is_err()
        );
    }

    #[test]
    fn key_store_round_trips_versioned_json() {
        let secrets = Arc::new(MemorySecretStore::default());
        let store = AgentKeyStore::new(secrets.clone());
        assert!(!store.has_key(AgentProvider::OpenAi).unwrap());
        store.save(AgentProvider::OpenAi, "sk-test").unwrap();
        assert!(store.has_key(AgentProvider::OpenAi).unwrap());
        assert_eq!(
            store.load(AgentProvider::OpenAi).unwrap().as_deref(),
            Some("sk-test")
        );

        let raw = secrets.get("agent:openai").unwrap().unwrap();
        let value: serde_json::Value = serde_json::from_str(&raw).unwrap();
        assert_eq!(value["version"], 1);
        assert_eq!(value["api_key"], "sk-test");

        store.delete(AgentProvider::OpenAi).unwrap();
        assert!(!store.has_key(AgentProvider::OpenAi).unwrap());
    }

    #[test]
    fn key_store_rejects_blank_and_unknown_versions() {
        let secrets = Arc::new(MemorySecretStore::default());
        let store = AgentKeyStore::new(secrets.clone());
        assert!(matches!(
            store.save(AgentProvider::OpenAi, "  "),
            Err(AgentError::BlankKey)
        ));
        secrets
            .set("agent:openai", r#"{"version":2,"api_key":"x"}"#)
            .unwrap();
        assert!(matches!(
            store.load(AgentProvider::OpenAi),
            Err(AgentError::UnsupportedStoredKey { version: 2, .. })
        ));
    }
}
