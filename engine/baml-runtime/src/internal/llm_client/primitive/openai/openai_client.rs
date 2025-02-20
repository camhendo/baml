use std::collections::HashMap;

use crate::internal::llm_client::ResolveMediaUrls;
use anyhow::Result;
use baml_types::{BamlMedia, BamlMediaContent, BamlMediaType};
use internal_baml_core::ir::ClientWalker;
use internal_baml_jinja::{ChatMessagePart, RenderContext_Client, RenderedChatMessage};
use serde_json::json;

use crate::client_registry::ClientProperty;
use crate::internal::llm_client::primitive::request::{
    make_parsed_request, make_request, RequestBuilder,
};
use crate::internal::llm_client::traits::{
    SseResponseTrait, StreamResponse, ToProviderMessage, ToProviderMessageExt,
    WithClientProperties, WithStreamChat,
};
use crate::internal::llm_client::{
    traits::{WithChat, WithClient, WithNoCompletion, WithRetryPolicy},
    LLMResponse, ModelFeatures,
};

use crate::request::create_client;
use crate::RuntimeContext;
use eventsource_stream::Eventsource;
use futures::StreamExt;

pub struct OpenAIClient {
    pub name: String,
    provider: String,
    // client: ClientWalker<'ir>,
    retry_policy: Option<String>,
    context: RenderContext_Client,
    features: ModelFeatures,
    properties: PostRequestProperities,
    // clients
    client: reqwest::Client,
}

impl WithRetryPolicy for OpenAIClient {
    fn retry_policy_name(&self) -> Option<&str> {
        self.retry_policy.as_deref()
    }
}

impl WithClientProperties for OpenAIClient {
    fn client_properties(&self) -> &HashMap<String, serde_json::Value> {
        &self.properties.properties
    }
    fn allowed_metadata(&self) -> &crate::internal::llm_client::AllowedMetadata {
        &self.properties.allowed_metadata
    }
}

impl WithClient for OpenAIClient {
    fn context(&self) -> &RenderContext_Client {
        &self.context
    }

    fn model_features(&self) -> &ModelFeatures {
        &self.features
    }
}

impl WithNoCompletion for OpenAIClient {}
// TODO: Enable completion with support for completion streams
// impl WithCompletion for OpenAIClient {
//     fn completion_options(
//         &self,
//         ctx: &RuntimeContext,
//     ) -> Result<internal_baml_jinja::CompletionOptions> {
//         return Ok(internal_baml_jinja::CompletionOptions::new("\n".into()));
//     }

//     async fn completion(&self, ctx: &RuntimeContext, prompt: &String) -> LLMResponse {
//         let (response, system_start, instant_start) =
//             match make_parsed_request::<CompletionResponse>(
//                 self,
//                 either::Either::Left(prompt),
//                 false,
//             )
//             .await
//             {
//                 Ok(v) => v,
//                 Err(e) => return e,
//             };

//         if response.choices.len() != 1 {
//             return LLMResponse::LLMFailure(LLMErrorResponse {
//                 client: self.context.name.to_string(),
//                 model: None,
//                 prompt: internal_baml_jinja::RenderedPrompt::Completion(prompt.clone()),
//                 start_time: system_start,
//                 latency: instant_start.elapsed(),
//                 request_options: self.properties.properties.clone(),
//                 message: format!(
//                     "Expected exactly one choices block, got {}",
//                     response.choices.len()
//                 ),
//                 code: ErrorCode::Other(200),
//             });
//         }

//         let usage = response.usage.as_ref();

//         LLMResponse::Success(LLMCompleteResponse {
//             client: self.context.name.to_string(),
//             prompt: internal_baml_jinja::RenderedPrompt::Completion(prompt.clone()),
//             content: response.choices[0].text.clone(),
//             start_time: system_start,
//             latency: instant_start.elapsed(),
//             model: response.model,
//             request_options: self.properties.properties.clone(),
//             metadata: LLMCompleteResponseMetadata {
//                 baml_is_complete: match response.choices.get(0) {
//                     Some(c) => match c.finish_reason {
//                         Some(FinishReason::Stop) => true,
//                         _ => false,
//                     },
//                     None => false,
//                 },
//                 finish_reason: match response.choices.get(0) {
//                     Some(c) => match c.finish_reason {
//                         Some(FinishReason::Stop) => Some(FinishReason::Stop.to_string()),
//                         _ => None,
//                     },
//                     None => None,
//                 },
//                 prompt_tokens: usage.map(|u| u.prompt_tokens),
//                 output_tokens: usage.map(|u| u.completion_tokens),
//                 total_tokens: usage.map(|u| u.total_tokens),
//             },
//         })
//     }
// }

impl WithChat for OpenAIClient {
    fn chat_options(&self, _ctx: &RuntimeContext) -> Result<internal_baml_jinja::ChatOptions> {
        Ok(internal_baml_jinja::ChatOptions::new(
            self.properties.default_role.clone(),
            None,
        ))
    }

    async fn chat(&self, _ctx: &RuntimeContext, prompt: &Vec<RenderedChatMessage>) -> LLMResponse {
        let (response, system_start, instant_start) =
            match make_parsed_request::<ChatCompletionResponse>(
                self,
                either::Either::Right(prompt),
                false,
            )
            .await
            {
                Ok(v) => v,
                Err(e) => return e,
            };

        if response.choices.len() != 1 {
            return LLMResponse::LLMFailure(LLMErrorResponse {
                client: self.context.name.to_string(),
                model: None,
                prompt: internal_baml_jinja::RenderedPrompt::Chat(prompt.clone()),
                start_time: system_start,
                latency: instant_start.elapsed(),
                request_options: self.properties.properties.clone(),
                message: format!(
                    "Expected exactly one choices block, got {}",
                    response.choices.len()
                ),
                code: ErrorCode::Other(200),
            });
        }

        let usage = response.usage.as_ref();

        LLMResponse::Success(LLMCompleteResponse {
            client: self.context.name.to_string(),
            prompt: internal_baml_jinja::RenderedPrompt::Chat(prompt.clone()),
            content: response.choices[0]
                .message
                .content
                .as_ref()
                .map_or("", |s| s.as_str())
                .to_string(),
            start_time: system_start,
            latency: instant_start.elapsed(),
            model: response.model,
            request_options: self.properties.properties.clone(),
            metadata: LLMCompleteResponseMetadata {
                baml_is_complete: match response.choices.get(0) {
                    Some(c) => match c.finish_reason {
                        Some(FinishReason::Stop) => true,
                        _ => false,
                    },
                    None => false,
                },
                finish_reason: match response.choices.get(0) {
                    Some(c) => match c.finish_reason {
                        Some(FinishReason::Stop) => Some(FinishReason::Stop.to_string()),
                        _ => None,
                    },
                    None => None,
                },
                prompt_tokens: usage.map(|u| u.prompt_tokens),
                output_tokens: usage.map(|u| u.completion_tokens),
                total_tokens: usage.map(|u| u.total_tokens),
            },
        })
    }
}

use crate::internal::llm_client::{
    ErrorCode, LLMCompleteResponse, LLMCompleteResponseMetadata, LLMErrorResponse,
};

use super::properties::{
    resolve_azure_properties, resolve_ollama_properties, resolve_openai_properties,
    PostRequestProperities,
};
use super::types::{ChatCompletionResponse, ChatCompletionResponseDelta, FinishReason};

impl RequestBuilder for OpenAIClient {
    fn http_client(&self) -> &reqwest::Client {
        &self.client
    }

    async fn build_request(
        &self,
        prompt: either::Either<&String, &Vec<RenderedChatMessage>>,
        allow_proxy: bool,
        stream: bool,
    ) -> Result<reqwest::RequestBuilder> {
        // Never proxy requests to Ollama
        let allow_proxy = allow_proxy
            && self.properties.proxy_url.is_some()
            && !self.properties.base_url.starts_with("http://localhost");

        let destination_url = if allow_proxy {
            self.properties
                .proxy_url
                .as_ref()
                .unwrap_or(&self.properties.base_url)
        } else {
            &self.properties.base_url
        };

        let mut req = self.client.post(if prompt.is_left() {
            format!("{}/completions", destination_url)
        } else {
            format!("{}/chat/completions", destination_url)
        });

        if !self.properties.query_params.is_empty() {
            req = req.query(&self.properties.query_params);
        }

        for (key, value) in &self.properties.headers {
            req = req.header(key, value);
        }
        if let Some(key) = &self.properties.api_key {
            req = req.bearer_auth(key);
        }

        // Don't attach BAML creds to localhost requests, i.e. ollama
        if allow_proxy {
            req = req.header("baml-original-url", self.properties.base_url.as_str());
        }

        let mut body = json!(self.properties.properties);

        let body_obj = body.as_object_mut().unwrap();
        match prompt {
            either::Either::Left(prompt) => {
                body_obj.insert("prompt".into(), json!(prompt));
            }
            either::Either::Right(messages) => {
                body_obj.extend(self.chat_to_message(messages)?);
            }
        }

        if stream {
            body_obj.insert("stream".into(), json!(true));
            if self.provider == "openai" {
                body_obj.insert(
                    "stream_options".into(),
                    json!({
                        "include_usage": true,
                    }),
                );
            }
        }

        Ok(req.json(&body))
    }

    fn request_options(&self) -> &HashMap<String, serde_json::Value> {
        &self.properties.properties
    }
}

impl SseResponseTrait for OpenAIClient {
    fn response_stream(
        &self,
        resp: reqwest::Response,
        prompt: &Vec<RenderedChatMessage>,
        system_start: web_time::SystemTime,
        instant_start: web_time::Instant,
    ) -> StreamResponse {
        let prompt = prompt.clone();
        let client_name = self.context.name.clone();
        let params = self.properties.properties.clone();
        Ok(Box::pin(
            resp.bytes_stream()
                .eventsource()
                .take_while(|event| {
                    std::future::ready(event.as_ref().is_ok_and(|e| e.data != "[DONE]"))
                })
                .map(|event| -> Result<ChatCompletionResponseDelta> {
                    Ok(serde_json::from_str::<ChatCompletionResponseDelta>(
                        &event?.data,
                    )?)
                })
                .inspect(|event| log::trace!("{:#?}", event))
                .scan(
                    Ok(LLMCompleteResponse {
                        client: client_name.clone(),
                        prompt: internal_baml_jinja::RenderedPrompt::Chat(prompt.clone()),
                        content: "".to_string(),
                        start_time: system_start,
                        latency: instant_start.elapsed(),
                        model: "".to_string(),
                        request_options: params.clone(),
                        metadata: LLMCompleteResponseMetadata {
                            baml_is_complete: false,
                            finish_reason: None,
                            prompt_tokens: None,
                            output_tokens: None,
                            total_tokens: None,
                        },
                    }),
                    move |accumulated: &mut Result<LLMCompleteResponse>, event| {
                        let Ok(ref mut inner) = accumulated else {
                            // halt the stream: the last stream event failed to parse
                            return std::future::ready(None);
                        };
                        let event = match event {
                            Ok(event) => event,
                            Err(e) => {
                                return std::future::ready(Some(LLMResponse::LLMFailure(
                                    LLMErrorResponse {
                                        client: client_name.clone(),
                                        model: if inner.model == "" {
                                            None
                                        } else {
                                            Some(inner.model.clone())
                                        },
                                        prompt: internal_baml_jinja::RenderedPrompt::Chat(
                                            prompt.clone(),
                                        ),
                                        start_time: system_start,
                                        request_options: params.clone(),
                                        latency: instant_start.elapsed(),
                                        message: format!("Failed to parse event: {:#?}", e),
                                        code: ErrorCode::Other(2),
                                    },
                                )));
                            }
                        };
                        if let Some(choice) = event.choices.get(0) {
                            if let Some(content) = choice.delta.content.as_ref() {
                                inner.content += content.as_str();
                            }
                            inner.model = event.model;
                            match choice.finish_reason.as_ref() {
                                Some(FinishReason::Stop) => {
                                    inner.metadata.baml_is_complete = true;
                                    inner.metadata.finish_reason =
                                        Some(FinishReason::Stop.to_string());
                                }
                                finish_reason => {
                                    inner.metadata.baml_is_complete = false;
                                    inner.metadata.finish_reason =
                                        finish_reason.as_ref().map(|r| r.to_string());
                                }
                            }
                        }
                        inner.latency = instant_start.elapsed();
                        if let Some(usage) = event.usage.as_ref() {
                            inner.metadata.prompt_tokens = Some(usage.prompt_tokens);
                            inner.metadata.output_tokens = Some(usage.completion_tokens);
                            inner.metadata.total_tokens = Some(usage.total_tokens);
                        }

                        std::future::ready(Some(LLMResponse::Success(inner.clone())))
                    },
                ),
        ))
    }
}

impl WithStreamChat for OpenAIClient {
    async fn stream_chat(
        &self,
        _ctx: &RuntimeContext,
        prompt: &Vec<RenderedChatMessage>,
    ) -> StreamResponse {
        let (resp, system_start, instant_start) =
            match make_request(self, either::Either::Right(prompt), true).await {
                Ok(v) => v,
                Err(e) => return Err(e),
            };
        self.response_stream(resp, prompt, system_start, instant_start)
    }
}

macro_rules! make_openai_client {
    ($client:ident, $properties:ident, $provider:expr, dynamic) => {
        Ok(Self {
            name: $client.name.clone(),
            provider: $provider.into(),
            context: RenderContext_Client {
                name: $client.name.clone(),
                provider: $client.provider.clone(),
                default_role: $properties.default_role.clone(),
            },
            features: ModelFeatures {
                chat: true,
                completion: false,
                anthropic_system_constraints: false,
                resolve_media_urls: ResolveMediaUrls::Never,
                allowed_metadata: $properties.allowed_metadata.clone(),
            },
            properties: $properties,
            retry_policy: $client.retry_policy.clone(),
            client: create_client()?,
        })
    };
    ($client:ident, $properties:ident, $provider:expr) => {
        Ok(Self {
            name: $client.name().into(),
            provider: $provider.into(),
            context: RenderContext_Client {
                name: $client.name().into(),
                provider: $client.elem().provider.clone(),
                default_role: $properties.default_role.clone(),
            },
            features: ModelFeatures {
                chat: true,
                completion: false,
                anthropic_system_constraints: false,
                resolve_media_urls: ResolveMediaUrls::Never,
                allowed_metadata: $properties.allowed_metadata.clone(),
            },
            properties: $properties,
            retry_policy: $client
                .elem()
                .retry_policy_id
                .as_ref()
                .map(|s| s.to_string()),
            client: create_client()?,
        })
    };
}

impl OpenAIClient {
    pub fn new(client: &ClientWalker, ctx: &RuntimeContext) -> Result<OpenAIClient> {
        let properties = super::super::resolve_properties_walker(client, ctx)?;
        let properties = resolve_openai_properties(properties, ctx)?;
        make_openai_client!(client, properties, "openai")
    }

    pub fn new_ollama(client: &ClientWalker, ctx: &RuntimeContext) -> Result<OpenAIClient> {
        let properties = super::super::resolve_properties_walker(client, ctx)?;
        let properties = resolve_ollama_properties(properties, ctx)?;
        make_openai_client!(client, properties, "ollama")
    }

    pub fn new_azure(client: &ClientWalker, ctx: &RuntimeContext) -> Result<OpenAIClient> {
        let properties = super::super::resolve_properties_walker(client, ctx)?;
        let properties = resolve_azure_properties(properties, ctx)?;
        make_openai_client!(client, properties, "azure")
    }

    pub fn dynamic_new(client: &ClientProperty, ctx: &RuntimeContext) -> Result<OpenAIClient> {
        let properties = resolve_openai_properties(
            client
                .options
                .iter()
                .map(|(k, v)| Ok((k.clone(), json!(v))))
                .collect::<Result<HashMap<_, _>>>()?,
            &ctx,
        )?;
        make_openai_client!(client, properties, "openai", dynamic)
    }

    pub fn dynamic_new_ollama(
        client: &ClientProperty,
        ctx: &RuntimeContext,
    ) -> Result<OpenAIClient> {
        let properties = resolve_ollama_properties(
            client
                .options
                .iter()
                .map(|(k, v)| Ok((k.clone(), json!(v))))
                .collect::<Result<HashMap<_, _>>>()?,
            ctx,
        )?;
        make_openai_client!(client, properties, "ollama", dynamic)
    }

    pub fn dynamic_new_azure(
        client: &ClientProperty,
        ctx: &RuntimeContext,
    ) -> Result<OpenAIClient> {
        let properties = resolve_azure_properties(
            client
                .options
                .iter()
                .map(|(k, v)| Ok((k.clone(), json!(v))))
                .collect::<Result<HashMap<_, _>>>()?,
            ctx,
        )?;
        make_openai_client!(client, properties, "azure", dynamic)
    }
}

impl ToProviderMessage for OpenAIClient {
    fn to_chat_message(
        &self,
        mut content: serde_json::Map<String, serde_json::Value>,
        text: &str,
    ) -> Result<serde_json::Map<String, serde_json::Value>> {
        content.insert("type".into(), json!("text"));
        content.insert("text".into(), json!(text));
        Ok(content)
    }

    fn to_media_message(
        &self,
        mut content: serde_json::Map<String, serde_json::Value>,
        media: &baml_types::BamlMedia,
    ) -> Result<serde_json::Map<String, serde_json::Value>> {
        let media_type = match media.media_type {
            BamlMediaType::Image => "image",
            BamlMediaType::Audio => "audio",
        };
        let media_type = format!("{}_url", media_type);
        match &media.content {
            BamlMediaContent::Url(media) => {
                content.insert("type".into(), json!(media_type));
                content.insert(
                    media_type,
                    json!({
                        "url": media.url
                    }),
                );
            }
            BamlMediaContent::Base64(b64_media) => {
                content.insert("type".into(), json!(media_type));
                content.insert(
                    format!("{}_url", media_type),
                    json!({
                        "url": format!("data:{};base64,{}", media.mime_type_as_ok()?, b64_media.base64)
                    }),
                );
            }
            BamlMediaContent::File(_) => {
                anyhow::bail!(
                    "BAML internal error (openai): file should have been resolved to base64"
                )
            }
        }
        Ok(content)
    }

    fn role_to_message(
        &self,
        content: &RenderedChatMessage,
    ) -> Result<serde_json::Map<String, serde_json::Value>> {
        let mut message = serde_json::Map::new();
        message.insert("role".into(), json!(content.role));
        message.insert(
            "content".into(),
            json!(self.parts_to_message(&content.parts)?),
        );
        Ok(message)
    }
}

impl ToProviderMessageExt for OpenAIClient {
    fn chat_to_message(
        &self,
        chat: &Vec<RenderedChatMessage>,
    ) -> Result<serde_json::Map<String, serde_json::Value>> {
        // merge all adjacent roles of the same type
        let mut res = serde_json::Map::new();

        res.insert(
            "messages".into(),
            chat.iter()
                .map(|c| self.role_to_message(c))
                .collect::<Result<Vec<_>>>()?
                .into(),
        );

        Ok(res)
    }
}
