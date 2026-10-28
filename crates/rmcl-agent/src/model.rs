//! Provider model construction.

use std::time::Duration;

use rig::{
    DynModel,
    operation::Completion,
    providers::{
        anthropic::AnthropicConfig,
        gemini::GeminiConfig,
        openai::{
            OpenAIConfig,
            wire::{Auth, OPENAI},
        },
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
    let base_url = if settings.provider == AgentProvider::Gemini {
        gemini_base_url(base_url)
    } else {
        base_url
    };
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
        AgentProvider::Kilo => OpenAIConfig::with_key(&OPENAI, api_key)
            .with_auth(Auth::OptionalBearer)
            .with_base_url(base_url)
            .connect(http)
            .chat(model)
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

fn gemini_base_url(value: &str) -> &str {
    let root = value.trim_end_matches('/');
    root.strip_suffix("/v1beta").unwrap_or(root)
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
        let openai = AgentSettings::from_config("openai", "", None, None).unwrap();
        assert!(build_model(&openai, "sk-test").is_ok());

        let anthropic = AgentSettings::from_config("anthropic", "", None, None).unwrap();
        assert!(build_model(&anthropic, "sk-test").is_ok());

        let gemini = AgentSettings::from_config("gemini", "", None, None).unwrap();
        assert!(build_model(&gemini, "sk-test").is_ok());

        let kilo = AgentSettings::from_config("kilo", "", None, None).unwrap();
        assert!(build_model(&kilo, "").is_ok());

        let custom = AgentSettings::from_config("custom", "model", None, None).unwrap();
        assert!(build_model(&custom, "sk-test").is_err());
    }

    #[test]
    fn builds_a_proxy_client() {
        let settings =
            AgentSettings::from_config("openai", "", None, Some("socks5://127.0.0.1:1080"))
                .unwrap();
        assert!(build_model(&settings, "sk-test").is_ok());
    }

    async fn capture_request(
        provider: &str,
        model_id: &str,
        base_path: &str,
        api_key: &str,
    ) -> String {
        use rig::completion::CompletionRequest;
        use tokio::{io::AsyncReadExt, net::TcpListener, sync::oneshot};

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let base_url = format!("http://{address}{base_path}");
        let settings = AgentSettings::from_config(provider, model_id, Some(&base_url), None)
            .expect("valid test provider");
        let model = build_model(&settings, api_key).unwrap();
        let (sender, receiver) = oneshot::channel();
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut request = Vec::new();
            let mut buffer = [0; 4096];
            loop {
                let read = socket.read(&mut buffer).await.unwrap();
                if read == 0 {
                    break;
                }
                request.extend_from_slice(&buffer[..read]);
                if request.windows(4).any(|window| window == b"\r\n\r\n") {
                    break;
                }
            }
            let _ = sender.send(String::from_utf8_lossy(&request).into_owned());
        });

        let call = model.call(CompletionRequest::new("hello"));
        let request = tokio::time::timeout(Duration::from_secs(5), async {
            let (_, request) = tokio::join!(call, receiver);
            request.unwrap()
        })
        .await
        .unwrap();
        server.await.unwrap();
        request
    }

    #[tokio::test]
    async fn kilo_empty_key_omits_the_authorization_header() {
        let request = capture_request("kilo", "kilo-auto/free", "/v1", "").await;
        assert!(request.starts_with("POST /v1/chat/completions "));
        assert!(!request.to_ascii_lowercase().contains("\r\nauthorization:"));
    }

    #[tokio::test]
    async fn gemini_requests_have_one_version_prefix() {
        for base_path in ["", "/v1beta", "/v1beta/"] {
            let request = capture_request("gemini", "gemini-test", base_path, "test-key").await;
            assert!(
                request
                    .starts_with("POST /v1beta/models/gemini-test:generateContent?key=test-key "),
                "unexpected request for base path {base_path}: {request}"
            );
        }
        assert_eq!(
            AgentProvider::Gemini.default_base_url(),
            Some("https://generativelanguage.googleapis.com")
        );
    }
}
