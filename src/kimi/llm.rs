use std::pin::Pin;

use async_trait::async_trait;
use futures::{Stream, StreamExt};
use serde::{Deserialize, Serialize};

use crate::kimi::error::{MekaiError, Result};
use async_openai::types::chat as oa;

pub type ChatStream = Pin<Box<dyn Stream<Item = Result<ChatStreamChunk>> + Send>>;

#[async_trait]
pub trait ChatProvider: Send + Sync {
    async fn chat(
        &self,
        messages: Vec<Message>,
        tools: Option<Vec<ToolDef>>,
    ) -> Result<ChatResponse>;
    async fn stream_chat(
        &self,
        messages: Vec<Message>,
        tools: Option<Vec<ToolDef>>,
    ) -> Result<ChatStream>;
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    pub role: String,
    pub content: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<Vec<ToolCall>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
}

impl Message {
    pub fn system(content: impl Into<String>) -> Self {
        Self {
            role: "system".into(),
            content: content.into(),
            tool_calls: None,
            tool_call_id: None,
        }
    }
    pub fn user(content: impl Into<String>) -> Self {
        Self {
            role: "user".into(),
            content: content.into(),
            tool_calls: None,
            tool_call_id: None,
        }
    }
    pub fn assistant(content: impl Into<String>) -> Self {
        Self {
            role: "assistant".into(),
            content: content.into(),
            tool_calls: None,
            tool_call_id: None,
        }
    }
    pub fn tool(content: impl Into<String>, call_id: impl Into<String>) -> Self {
        Self {
            role: "tool".into(),
            content: content.into(),
            tool_calls: None,
            tool_call_id: Some(call_id.into()),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ToolCall {
    pub id: String,
    #[serde(rename = "type")]
    pub tool_type: String,
    pub function: FunctionCall,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct FunctionCall {
    pub name: String,
    pub arguments: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolDef {
    pub name: String,
    pub description: String,
    pub parameters: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatResponse {
    pub message: Message,
    pub usage: Option<Usage>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatStreamChunk {
    pub delta: String,
    pub tool_calls: Option<Vec<ToolCall>>,
    pub finish_reason: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Usage {
    pub prompt_tokens: usize,
    pub completion_tokens: usize,
    pub total_tokens: usize,
}

use crate::kimi::config::{Config, LlmModel, LlmProvider};

pub async fn create_llm(config: &Config, model_name: &str) -> Result<Box<dyn ChatProvider>> {
    let model = config
        .models
        .get(model_name)
        .cloned()
        .unwrap_or_else(|| LlmModel {
            provider: model_name.to_string(),
            model: model_name.to_string(),
            max_context_size: None,
            temperature: None,
            capabilities: vec![],
        });

    if !model.capabilities.is_empty() {
        tracing::info!("Model {model_name} capabilities: {:?}", model.capabilities);
    }

    let mut provider = config
        .providers
        .get(&model.provider)
        .cloned()
        .unwrap_or_else(|| LlmProvider {
            r#type: model.provider.clone(),
            base_url: None,
            api_key: None,
        });

    // Set default base_url based on provider type if not configured
    if provider.base_url.is_none() {
        provider.base_url = Some(match provider.r#type.as_str() {
            "kimi" => "https://api.moonshot.cn/v1".to_string(),
            "openai" | "openai_legacy" | "openai_responses" => {
                "https://api.openai.com/v1".to_string()
            }
            _ => "https://api.openai.com/v1".to_string(),
        });
    }

    tracing::info!(
        "LLM created: model={}, base_url={}",
        model.model,
        provider.base_url.as_deref().unwrap_or("?"),
    );
    match provider.r#type.as_str() {
        "kimi" | "openai" | "openai_legacy" | "openai_responses" => {
            Ok(Box::new(OpenAiProvider::new(provider, model)?))
        }
        "anthropic" => Ok(Box::new(AnthropicProvider::new(provider, model)?)),
        "google_genai" | "gemini" => Ok(Box::new(GeminiProvider::new(provider, model)?)),
        "_echo" => Ok(Box::new(EchoProvider)),
        _ => Err(MekaiError::Llm(format!(
            "Unsupported provider type: {}",
            provider.r#type
        ))),
    }
}

pub struct OpenAiProvider {
    client: async_openai::Client<async_openai::config::OpenAIConfig>,
    model: String,
    temperature: Option<f32>,
}

impl OpenAiProvider {
    pub fn new(provider: LlmProvider, model: LlmModel) -> Result<Self> {
        let api_key = provider
            .api_key
            .ok_or_else(|| MekaiError::Llm("API key required for OpenAI-style provider".into()))?;
        let base_url = provider
            .base_url
            .unwrap_or_else(|| "https://api.openai.com/v1".to_string());
        let config = async_openai::config::OpenAIConfig::new()
            .with_api_key(api_key)
            .with_api_base(base_url);
        let http_client = reqwest::ClientBuilder::new()
            .user_agent("Mekai")
            .build()
            .map_err(|e| MekaiError::Llm(format!("Failed to build HTTP client: {e}")))?;
        let client = async_openai::Client::with_config(config)
            .with_http_client(http_client);
        Ok(Self {
            client,
            model: model.model,
            temperature: model.temperature,
        })
    }

    fn to_openai_messages(&self, messages: Vec<Message>) -> Vec<oa::ChatCompletionRequestMessage> {
        messages
            .into_iter()
            .map(|m| {
                let role = m.role.as_str();
                match role {
                    "system" => oa::ChatCompletionRequestSystemMessage {
                        content: oa::ChatCompletionRequestSystemMessageContent::Text(m.content),
                        name: None,
                    }
                    .into(),
                    "user" => oa::ChatCompletionRequestUserMessage {
                        content: oa::ChatCompletionRequestUserMessageContent::Text(m.content),
                        name: None,
                    }
                    .into(),
                    "assistant" => {
                        let mut msg = oa::ChatCompletionRequestAssistantMessage {
                            content: Some(oa::ChatCompletionRequestAssistantMessageContent::Text(
                                m.content,
                            )),
                            name: None,
                            refusal: None,
                            tool_calls: m.tool_calls.map(|calls| {
                                calls
                                    .into_iter()
                                    .map(|tc| {
                                        oa::ChatCompletionMessageToolCalls::Function(
                                            oa::ChatCompletionMessageToolCall {
                                                id: tc.id,
                                                function: oa::FunctionCall {
                                                    name: tc.function.name,
                                                    arguments: tc.function.arguments,
                                                },
                                            },
                                        )
                                    })
                                    .collect()
                            }),
                            audio: None,
                            #[allow(deprecated)]
                            function_call: None,
                        };
                        if msg
                            .content
                            .as_ref()
                            .map(|c| match c {
                                oa::ChatCompletionRequestAssistantMessageContent::Text(t) => {
                                    t.is_empty()
                                }
                                _ => false,
                            })
                            .unwrap_or(true)
                            && msg.tool_calls.is_some()
                        {
                            // Kimi-for-coding rejects empty content alongside tool calls
                            msg.content = None;
                        }
                        msg.into()
                    }
                    "tool" => oa::ChatCompletionRequestToolMessage {
                        content: oa::ChatCompletionRequestToolMessageContent::Text(m.content),
                        tool_call_id: m.tool_call_id.unwrap_or_default(),
                    }
                    .into(),
                    _ => oa::ChatCompletionRequestUserMessage {
                        content: oa::ChatCompletionRequestUserMessageContent::Text(m.content),
                        name: None,
                    }
                    .into(),
                }
            })
            .collect()
    }

    fn to_openai_tools(&self, tools: Vec<ToolDef>) -> Vec<oa::ChatCompletionTools> {
        tools
            .into_iter()
            .map(|t| {
                oa::ChatCompletionTools::Function(oa::ChatCompletionTool {
                    function: oa::FunctionObject {
                        name: t.name,
                        description: Some(t.description),
                        parameters: Some(t.parameters),
                        strict: None,
                    },
                })
            })
            .collect()
    }

    fn openai_message_to_message(msg: &oa::ChatCompletionResponseMessage) -> Message {
        let content = msg.content.clone().unwrap_or_default();
        let tool_calls = msg.tool_calls.as_ref().map(|calls| {
            calls
                .iter()
                .filter_map(|tc| match tc {
                    oa::ChatCompletionMessageToolCalls::Function(tc) => Some(ToolCall {
                        id: tc.id.clone(),
                        tool_type: "function".to_string(),
                        function: FunctionCall {
                            name: tc.function.name.clone(),
                            arguments: tc.function.arguments.clone(),
                        },
                    }),
                    _ => None,
                })
                .collect()
        });
        Message {
            role: format!("{:?}", msg.role).to_lowercase(),
            content,
            tool_calls,
            tool_call_id: None,
        }
    }
}

#[async_trait]
impl ChatProvider for OpenAiProvider {
    async fn chat(
        &self,
        messages: Vec<Message>,
        tools: Option<Vec<ToolDef>>,
    ) -> Result<ChatResponse> {
        let mut request = oa::CreateChatCompletionRequest {
            model: self.model.clone(),
            messages: self.to_openai_messages(messages),
            tools: tools
                .filter(|t| !t.is_empty())
                .map(|t| self.to_openai_tools(t)),
            temperature: self.temperature,
            ..Default::default()
        };
        if let Some(ref tools) = request.tools
            && !tools.is_empty() {
                request.tool_choice = Some(oa::ChatCompletionToolChoiceOption::Mode(
                    oa::ToolChoiceOptions::Auto,
                ));
            }
        let response = self
            .client
            .chat()
            .create(request)
            .await
            .map_err(|e| MekaiError::Llm(format!("LLM API error: {e}")))?;
        let choice = response
            .choices
            .into_iter()
            .next()
            .ok_or_else(|| MekaiError::Llm("No choices in LLM response".into()))?;
        let message = Self::openai_message_to_message(&choice.message);
        let usage = response.usage.map(|u| Usage {
            prompt_tokens: u.prompt_tokens as usize,
            completion_tokens: u.completion_tokens as usize,
            total_tokens: u.total_tokens as usize,
        });
        Ok(ChatResponse { message, usage })
    }

    async fn stream_chat(
        &self,
        messages: Vec<Message>,
        tools: Option<Vec<ToolDef>>,
    ) -> Result<ChatStream> {
        let mut request = oa::CreateChatCompletionRequest {
            model: self.model.clone(),
            messages: self.to_openai_messages(messages),
            tools: tools
                .filter(|t| !t.is_empty())
                .map(|t| self.to_openai_tools(t)),
            stream: Some(true),
            temperature: self.temperature,
            ..Default::default()
        };
        if let Some(ref tools) = request.tools
            && !tools.is_empty() {
                request.tool_choice = Some(oa::ChatCompletionToolChoiceOption::Mode(
                    oa::ToolChoiceOptions::Auto,
                ));
            }
        tracing::info!("Sending streaming chat request for model {}", self.model);
        let mut stream = self
            .client
            .chat()
            .create_stream(request)
            .await
            .map_err(|e| MekaiError::Llm(format!("LLM API error: {e}")))?;
        let model = self.model.clone();
        let stream = async_stream::try_stream! {
            while let Some(result) = stream.next().await {
                let chunk = result.map_err(|e| MekaiError::Llm(format!("Stream error: {e}")))?;
                if let Some(usage) = chunk.usage {
                    tracing::info!("Stream usage: prompt={}, completion={}", usage.prompt_tokens, usage.completion_tokens);
                }
                if chunk.choices.is_empty() {
                    continue;
                }
                let delta = &chunk.choices[0].delta;
                let content = delta.content.clone().unwrap_or_default();
                let tool_calls = delta.tool_calls.as_ref().and_then(|calls| {
                    if calls.is_empty() { return None; }
                    Some(calls.iter().filter_map(|tc| {
                        let function = tc.function.as_ref()?;
                        Some(ToolCall {
                            id: tc.id.clone().unwrap_or_else(|| uuid::Uuid::new_v4().to_string()),
                            tool_type: "function".to_string(),
                            function: FunctionCall {
                                name: function.name.clone().unwrap_or_default(),
                                arguments: function.arguments.clone().unwrap_or_default(),
                            },
                        })
                    }).collect::<Vec<_>>())
                });
                let finish_reason = chunk.choices[0].finish_reason.map(|r| format!("{:?}", r).to_lowercase());
                if !content.is_empty() || tool_calls.is_some() || finish_reason.is_some() {
                    yield ChatStreamChunk { delta: content, tool_calls, finish_reason };
                }
            }
            tracing::info!("Stream ended for model {}", model);
        };
        Ok(Box::pin(stream))
    }
}

pub struct AnthropicProvider {
    _client: reqwest::Client,
    _base_url: String,
    _api_key: String,
    _model: String,
}

impl AnthropicProvider {
    pub fn new(provider: LlmProvider, model: LlmModel) -> Result<Self> {
        let api_key = provider
            .api_key
            .ok_or_else(|| MekaiError::Llm("API key required for Anthropic provider".into()))?;
        Ok(Self {
            _client: reqwest::Client::new(),
            _base_url: provider
                .base_url
                .unwrap_or_else(|| "https://api.anthropic.com".to_string()),
            _api_key: api_key,
            _model: model.model,
        })
    }
}

#[async_trait]
impl ChatProvider for AnthropicProvider {
    async fn chat(
        &self,
        _messages: Vec<Message>,
        _tools: Option<Vec<ToolDef>>,
    ) -> Result<ChatResponse> {
        unimplemented!("Anthropic provider not yet implemented")
    }

    async fn stream_chat(
        &self,
        _messages: Vec<Message>,
        _tools: Option<Vec<ToolDef>>,
    ) -> Result<ChatStream> {
        unimplemented!("Anthropic provider not yet implemented")
    }
}

pub struct GeminiProvider {
    _client: reqwest::Client,
    _base_url: String,
    _api_key: String,
    _model: String,
}

impl GeminiProvider {
    pub fn new(provider: LlmProvider, model: LlmModel) -> Result<Self> {
        let api_key = provider
            .api_key
            .ok_or_else(|| MekaiError::Llm("API key required for Gemini provider".into()))?;
        Ok(Self {
            _client: reqwest::Client::new(),
            _base_url: provider
                .base_url
                .unwrap_or_else(|| "https://generativelanguage.googleapis.com/v1".to_string()),
            _api_key: api_key,
            _model: model.model,
        })
    }
}

#[async_trait]
impl ChatProvider for GeminiProvider {
    async fn chat(
        &self,
        _messages: Vec<Message>,
        _tools: Option<Vec<ToolDef>>,
    ) -> Result<ChatResponse> {
        unimplemented!("Gemini provider not yet implemented")
    }

    async fn stream_chat(
        &self,
        _messages: Vec<Message>,
        _tools: Option<Vec<ToolDef>>,
    ) -> Result<ChatStream> {
        unimplemented!("Gemini provider not yet implemented")
    }
}

pub struct EchoProvider;

#[async_trait]
impl ChatProvider for EchoProvider {
    async fn chat(
        &self,
        messages: Vec<Message>,
        _tools: Option<Vec<ToolDef>>,
    ) -> Result<ChatResponse> {
        let last = messages
            .last()
            .map(|m| m.content.clone())
            .unwrap_or_default();
        Ok(ChatResponse {
            message: Message {
                role: "assistant".to_string(),
                content: format!("Echo: {last}"),
                tool_calls: None,
                tool_call_id: None,
            },
            usage: None,
        })
    }

    async fn stream_chat(
        &self,
        messages: Vec<Message>,
        tools: Option<Vec<ToolDef>>,
    ) -> Result<ChatStream> {
        let response = self.chat(messages, tools).await?;
        let delta = response.message.content;
        let stream = async_stream::try_stream! {
            yield ChatStreamChunk { delta, tool_calls: None, finish_reason: Some("stop".to_string()) };
        };
        Ok(Box::pin(stream))
    }
}
