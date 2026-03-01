// LLM provider abstraction for compact --llm
//
// Supports:
//   - Anthropic (Messages API)
//   - OpenAI (Chat Completions API)
//   - Custom (OpenAI-compatible endpoint)
//
// Used by the compact command for CLI-driven compaction.
// MCP/serve never calls LLMs — the agent IS the LLM in that flow.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

// --- Provider trait ---

pub trait LlmProvider: Send + Sync {
    fn complete(&self, prompt: &str) -> Result<String>;
}

// --- Provider config ---

#[derive(Debug, Clone)]
pub struct LlmConfig {
    pub provider: ProviderKind,
    pub model: String,
    pub api_key: String,
    pub base_url: Option<String>,
    pub concurrency: usize,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ProviderKind {
    Anthropic,
    OpenAi,
    Custom,
}

impl std::str::FromStr for ProviderKind {
    type Err = anyhow::Error;
    fn from_str(s: &str) -> Result<Self> {
        match s.to_lowercase().as_str() {
            "anthropic" => Ok(Self::Anthropic),
            "openai" => Ok(Self::OpenAi),
            "custom" => Ok(Self::Custom),
            other => anyhow::bail!("unknown provider: {}. Use 'anthropic', 'openai', or 'custom'.", other),
        }
    }
}

impl std::fmt::Display for ProviderKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Anthropic => write!(f, "anthropic"),
            Self::OpenAi => write!(f, "openai"),
            Self::Custom => write!(f, "custom"),
        }
    }
}

impl LlmConfig {
    pub fn default_model(provider: &ProviderKind) -> &'static str {
        match provider {
            ProviderKind::Anthropic => "claude-sonnet-4-20250514",
            ProviderKind::OpenAi => "gpt-4o",
            ProviderKind::Custom => "gpt-4o",
        }
    }

    pub fn env_key_name(provider: &ProviderKind) -> &'static str {
        match provider {
            ProviderKind::Anthropic => "ANTHROPIC_API_KEY",
            ProviderKind::OpenAi => "OPENAI_API_KEY",
            ProviderKind::Custom => "AGIT_API_KEY",
        }
    }

    /// Create a provider instance from this config.
    pub fn create_provider(&self) -> Result<Box<dyn LlmProvider>> {
        match self.provider {
            ProviderKind::Anthropic => Ok(Box::new(AnthropicProvider {
                api_key: self.api_key.clone(),
                model: self.model.clone(),
            })),
            ProviderKind::OpenAi | ProviderKind::Custom => {
                let base_url = self.base_url.clone().unwrap_or_else(|| {
                    "https://api.openai.com/v1".to_string()
                });
                Ok(Box::new(OpenAiProvider {
                    api_key: self.api_key.clone(),
                    model: self.model.clone(),
                    base_url,
                }))
            }
        }
    }
}

// --- Anthropic provider ---

struct AnthropicProvider {
    api_key: String,
    model: String,
}

#[derive(Serialize)]
struct AnthropicRequest {
    model: String,
    max_tokens: u32,
    messages: Vec<AnthropicMessage>,
}

#[derive(Serialize)]
struct AnthropicMessage {
    role: String,
    content: String,
}

#[derive(Deserialize)]
struct AnthropicResponse {
    content: Vec<AnthropicContent>,
}

#[derive(Deserialize)]
struct AnthropicContent {
    text: String,
}

impl LlmProvider for AnthropicProvider {
    fn complete(&self, prompt: &str) -> Result<String> {
        let rt = tokio::runtime::Handle::try_current()
            .unwrap_or_else(|_| {
                // If no runtime exists, we'll need to create one in the caller
                panic!("LLM provider must be called within a tokio runtime");
            });

        let client = reqwest::Client::new();
        let request = AnthropicRequest {
            model: self.model.clone(),
            max_tokens: 4096,
            messages: vec![AnthropicMessage {
                role: "user".into(),
                content: prompt.to_string(),
            }],
        };

        let api_key = self.api_key.clone();
        let response = rt.block_on(async {
            client
                .post("https://api.anthropic.com/v1/messages")
                .header("x-api-key", &api_key)
                .header("anthropic-version", "2023-06-01")
                .header("content-type", "application/json")
                .json(&request)
                .send()
                .await
                .context("sending request to Anthropic API")?
                .error_for_status()
                .context("Anthropic API returned an error")?
                .json::<AnthropicResponse>()
                .await
                .context("parsing Anthropic API response")
        })?;

        response
            .content
            .first()
            .map(|c| c.text.clone())
            .context("empty response from Anthropic API")
    }
}

// --- OpenAI-compatible provider ---

struct OpenAiProvider {
    api_key: String,
    model: String,
    base_url: String,
}

#[derive(Serialize)]
struct OpenAiRequest {
    model: String,
    messages: Vec<OpenAiMessage>,
    max_tokens: u32,
}

#[derive(Serialize)]
struct OpenAiMessage {
    role: String,
    content: String,
}

#[derive(Deserialize)]
struct OpenAiResponse {
    choices: Vec<OpenAiChoice>,
}

#[derive(Deserialize)]
struct OpenAiChoice {
    message: OpenAiResponseMessage,
}

#[derive(Deserialize)]
struct OpenAiResponseMessage {
    content: String,
}

impl LlmProvider for OpenAiProvider {
    fn complete(&self, prompt: &str) -> Result<String> {
        let rt = tokio::runtime::Handle::try_current()
            .unwrap_or_else(|_| {
                panic!("LLM provider must be called within a tokio runtime");
            });

        let client = reqwest::Client::new();
        let request = OpenAiRequest {
            model: self.model.clone(),
            messages: vec![OpenAiMessage {
                role: "user".into(),
                content: prompt.to_string(),
            }],
            max_tokens: 4096,
        };

        let url = format!("{}/chat/completions", self.base_url);
        let api_key = self.api_key.clone();
        let response = rt.block_on(async {
            client
                .post(&url)
                .header("Authorization", format!("Bearer {}", api_key))
                .header("content-type", "application/json")
                .json(&request)
                .send()
                .await
                .context("sending request to OpenAI API")?
                .error_for_status()
                .context("OpenAI API returned an error")?
                .json::<OpenAiResponse>()
                .await
                .context("parsing OpenAI API response")
        })?;

        response
            .choices
            .first()
            .map(|c| c.message.content.clone())
            .context("empty response from OpenAI API")
    }
}

