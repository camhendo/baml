use std::collections::{HashMap, HashSet};

use base64::write;
use colored::*;
pub mod llm_provider;
pub mod orchestrator;
pub mod primitive;

pub mod retry_policy;
mod strategy;
pub mod traits;

use anyhow::Result;

use internal_baml_core::ir::ClientWalker;
use internal_baml_jinja::{ChatMessagePart, RenderedChatMessage, RenderedPrompt};
use serde::{Deserialize, Serialize};
use serde_json::Map;
use std::error::Error;

use reqwest::StatusCode;

#[cfg(target_arch = "wasm32")]
use wasm_bindgen::JsValue;

#[derive(Clone, Copy, PartialEq)]
pub enum ResolveMediaUrls {
    // there are 5 input formats:
    // - file
    // - url_with_mime
    // - url_no_mime
    // - b64_with_mime
    // - b64_no_mime

    // there are 5 possible output formats:
    // - url_with_mime: vertex
    // - url_no_mime: openai
    // - b64_with_mime: everyone (aws, anthropic, google, openai, vertex)
    // - b64_no_mime: no one

    // aws: supports b64 w mime
    // anthropic: supports b64 w mime
    // google: supports b64 w mime
    // openai: supports URLs w/o mime (b64 data URLs also work here)
    // vertex: supports URLs w/ mime, b64 w/ mime
    Always,
    EnsureMime,
    Never,
}

#[derive(Clone)]
pub struct ModelFeatures {
    pub completion: bool,
    pub chat: bool,
    pub anthropic_system_constraints: bool,
    pub resolve_media_urls: ResolveMediaUrls,
    pub allowed_metadata: AllowedMetadata,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum AllowedMetadata {
    #[serde(rename = "all")]
    All,
    #[serde(rename = "none")]
    None,
    Only(HashSet<String>),
}

impl AllowedMetadata {
    pub fn is_allowed(&self, key: &str) -> bool {
        match self {
            Self::All => true,
            Self::None => false,
            Self::Only(allowed) => allowed.contains(&key.to_string()),
        }
    }
}

#[derive(Debug)]
pub struct RetryLLMResponse {
    pub client: Option<String>,
    pub passed: Option<Box<LLMResponse>>,
    pub failed: Vec<LLMResponse>,
}

#[derive(Debug, Clone)]
pub enum LLMResponse {
    Success(LLMCompleteResponse),
    LLMFailure(LLMErrorResponse),
    OtherFailure(String),
}

impl Error for LLMResponse {}

impl std::fmt::Display for LLMResponse {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Success(response) => write!(f, "{}", response),
            Self::LLMFailure(failure) => write!(f, "LLM call failed: {failure:?}"),
            Self::OtherFailure(message) => write!(f, "LLM call failed: {message}"),
        }
    }
}

impl LLMResponse {
    pub fn content(&self) -> Result<&str> {
        match self {
            Self::Success(response) => Ok(&response.content),
            Self::LLMFailure(failure) => Err(anyhow::anyhow!("LLM call failed: {failure:?}")),
            Self::OtherFailure(message) => Err(anyhow::anyhow!("LLM failed to call: {message}")),
        }
    }
}

#[derive(Debug, Clone)]
pub struct LLMErrorResponse {
    pub client: String,
    pub model: Option<String>,
    pub prompt: RenderedPrompt,
    pub request_options: HashMap<String, serde_json::Value>,
    pub start_time: web_time::SystemTime,
    pub latency: web_time::Duration,

    // Short error message
    pub message: String,
    pub code: ErrorCode,
}

#[derive(Debug, Clone)]
pub enum ErrorCode {
    InvalidAuthentication, // 401
    NotSupported,          // 403
    RateLimited,           // 429
    ServerError,           // 500
    ServiceUnavailable,    // 503

    // We failed to parse the response
    UnsupportedResponse(u16),

    // Any other error
    Other(u16),
}

impl ErrorCode {
    pub fn to_string(&self) -> String {
        match self {
            ErrorCode::InvalidAuthentication => "InvalidAuthentication (401)".into(),
            ErrorCode::NotSupported => "NotSupported (403)".into(),
            ErrorCode::RateLimited => "RateLimited (429)".into(),
            ErrorCode::ServerError => "ServerError (500)".into(),
            ErrorCode::ServiceUnavailable => "ServiceUnavailable (503)".into(),
            ErrorCode::UnsupportedResponse(code) => format!("BadResponse {}", code),
            ErrorCode::Other(code) => format!("Unspecified error code: {}", code),
        }
    }

    pub fn from_status(status: StatusCode) -> Self {
        match status.as_u16() {
            401 => ErrorCode::InvalidAuthentication,
            403 => ErrorCode::NotSupported,
            429 => ErrorCode::RateLimited,
            500 => ErrorCode::ServerError,
            503 => ErrorCode::ServiceUnavailable,
            code => ErrorCode::Other(code),
        }
    }

    pub fn from_u16(code: u16) -> Self {
        match code {
            401 => ErrorCode::InvalidAuthentication,
            403 => ErrorCode::NotSupported,
            429 => ErrorCode::RateLimited,
            500 => ErrorCode::ServerError,
            503 => ErrorCode::ServiceUnavailable,
            code => ErrorCode::Other(code),
        }
    }
}

#[derive(Clone, Debug)]
pub struct LLMCompleteResponse {
    pub client: String,
    pub model: String,
    pub prompt: RenderedPrompt,
    pub request_options: HashMap<String, serde_json::Value>,
    pub content: String,
    pub start_time: web_time::SystemTime,
    pub latency: web_time::Duration,
    pub metadata: LLMCompleteResponseMetadata,
}

#[derive(Clone, Debug, Serialize)]
pub struct LLMCompleteResponseMetadata {
    pub baml_is_complete: bool,
    pub finish_reason: Option<String>,
    pub prompt_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
    pub total_tokens: Option<u64>,
}

impl std::fmt::Display for LLMCompleteResponse {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        writeln!(
            f,
            "{}",
            format!(
                "Client: {} ({}) - {}ms. StopReason: {}",
                self.client,
                self.model,
                self.latency.as_millis(),
                self.metadata.finish_reason.as_deref().unwrap_or("unknown")
            )
            .yellow()
        )?;
        writeln!(f, "{}", "---PROMPT---".blue())?;
        writeln!(f, "{}", self.prompt.to_string().dimmed())?;
        writeln!(f, "{}", "---LLM REPLY---".blue())?;
        write!(f, "{}", self.content.dimmed())
    }
}

// For parsing args
fn resolve_properties_walker(
    client: &ClientWalker,
    ctx: &crate::RuntimeContext,
) -> Result<std::collections::HashMap<String, serde_json::Value>> {
    use anyhow::Context;
    (&client.item.elem.options)
        .iter()
        .map(|(k, v)| {
            Ok((
                k.into(),
                ctx.resolve_expression::<serde_json::Value>(v)
                    .context(format!(
                        "client {} could not resolve options.{}",
                        client.name(),
                        k
                    ))?,
            ))
        })
        .collect::<Result<std::collections::HashMap<_, _>>>()
}
