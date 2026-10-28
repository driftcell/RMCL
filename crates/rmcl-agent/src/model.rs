//! Provider model construction.

use std::time::Duration;

use rig::{
    DynModel,
    operation::Completion,
    providers::{
        anthropic::AnthropicConfig,
        gemini::GeminiConfig,
        openai::{OpenAIConfig, wire::OPENAI},
    },
    rig_reqwest::{
        ReqwestClient,
        reqwest::{self, Proxy},
    },
};

use crate::{AgentError, AgentProvider, AgentSettings};

const REQUEST_TIMEOUT: Duration = Duration::from_secs(180);

/// Builds an erased completion model for the configured provider.
pub fn build_model(
    settings: &AgentSettings,
    api_key: &str,
) -> Result<DynModel<Completion>, AgentError> {
    let model = settings.effective_model()?;
    let base_url = settings.effective_base_url()?;
    let http = http_client(settings.proxy_url()?)?;
    let model = match settings.provider {
        AgentProvider::Anthropic => AnthropicConfig::new(api_key)
            .with_base_url(base_url)
            .connect(http)
            .completion(model)
            .erase(),
        AgentProvider::Gemini => GeminiConfig::new(api_key)
            .with_base_url(base_url)
            .connect(http)
            .completion(model)
            .erase(),
        AgentProvider::OpenAi
        | AgentProvider::DeepSeek
        | AgentProvider::OpenRouter
        | AgentProvider::Qwen
        | AgentProvider::Kimi
        | AgentProvider::Zhipu
        | AgentProvider::Custom => OpenAIConfig::with_key(&OPENAI, api_key)
            .with_base_url(base_url)
            .connect(http)
            .chat(model)
            .erase(),
    };
    Ok(model)
}

fn http_client(proxy: Option<url::Url>) -> Result<ReqwestClient, AgentError> {
    let mut builder = reqwest::Client::builder().timeout(REQUEST_TIMEOUT);
    if let Some(proxy) = proxy {
        builder = builder.proxy(Proxy::all(proxy.as_str()).map_err(AgentError::Proxy)?);
    }
    let client = builder.build().map_err(AgentError::Http)?;
    Ok(ReqwestClient::from(client))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_models_for_native_and_compatible_providers() {
        let openai = AgentSettings::from_config("openai", "", None, None);
        assert!(build_model(&openai, "sk-test").is_ok());

        let anthropic = AgentSettings::from_config("anthropic", "", None, None);
        assert!(build_model(&anthropic, "sk-test").is_ok());

        let gemini = AgentSettings::from_config("gemini", "", None, None);
        assert!(build_model(&gemini, "sk-test").is_ok());

        let custom = AgentSettings::from_config("custom", "model", None, None);
        assert!(build_model(&custom, "sk-test").is_err());
    }

    #[test]
    fn builds_a_proxy_client() {
        let settings =
            AgentSettings::from_config("openai", "", None, Some("socks5://127.0.0.1:1080"));
        assert!(build_model(&settings, "sk-test").is_ok());
    }
}
