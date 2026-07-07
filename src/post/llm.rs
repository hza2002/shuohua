use super::{AppContext, PipelineText, PostError, PostFailureReason, PostProcessor};
use async_trait::async_trait;
use reqwest::StatusCode;
use serde_json::{json, Map, Value};
use std::collections::HashSet;
use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProviderFormat {
    OpenAi,
    Anthropic,
}

#[derive(Debug, Clone)]
pub struct LlmCleanupConfig {
    pub name: String,
    pub format: ProviderFormat,
    pub provider_name: String,
    pub base_url: String,
    pub api_key: String,
    pub model: String,
    pub extra_body: Map<String, Value>,
    pub system_prompt: Option<String>,
    pub prompt: String,
}

pub struct LlmCleanup {
    cfg: LlmCleanupConfig,
    client: reqwest::Client,
}

impl LlmCleanup {
    pub fn new(cfg: LlmCleanupConfig) -> Self {
        Self {
            cfg,
            client: reqwest::Client::new(),
        }
    }

    pub async fn check_runtime(&self) -> Result<(), PostError> {
        let ctx = AppContext {
            bundle_id: Some("doctor.runtime".to_string()),
            app_name: Some("shuo doctor".to_string()),
        };
        let prompt = render_prompt(&self.cfg.prompt, &ctx, "doctor runtime check");
        match self.cfg.format {
            ProviderFormat::OpenAi => self.call_openai(&prompt).await.map(|_| ()),
            ProviderFormat::Anthropic => self.call_anthropic(&prompt).await.map(|_| ()),
        }
    }

    pub async fn list_models(&self) -> Result<Vec<String>, PostError> {
        match self.cfg.format {
            ProviderFormat::OpenAi => self.list_openai_models().await,
            ProviderFormat::Anthropic => self.list_anthropic_models().await,
        }
    }
}

#[async_trait]
impl PostProcessor for LlmCleanup {
    fn name(&self) -> &str {
        &self.cfg.name
    }

    async fn process(
        &self,
        input: PipelineText,
        ctx: &AppContext,
    ) -> Result<PipelineText, PostError> {
        let prompt = render_prompt(&self.cfg.prompt, ctx, &input.text);
        let text = match self.cfg.format {
            ProviderFormat::OpenAi => self.call_openai(&prompt).await?,
            ProviderFormat::Anthropic => self.call_anthropic(&prompt).await?,
        };
        Ok(PipelineText { text, ..input })
    }
}

impl LlmCleanup {
    async fn list_openai_models(&self) -> Result<Vec<String>, PostError> {
        let url = join_url(&self.cfg.base_url, "models");
        let response = self
            .client
            .get(url)
            .bearer_auth(&self.cfg.api_key)
            .send()
            .await
            .map_err(|e| {
                PostError::failed_with_reason(
                    PostFailureReason::Network,
                    format!(
                        "{} ({}) model list request failed: {e}",
                        self.cfg.name, self.cfg.provider_name
                    ),
                )
            })?;
        self.parse_model_list_http_response(response, "openai")
            .await
    }

    async fn list_anthropic_models(&self) -> Result<Vec<String>, PostError> {
        let url = join_url(&self.cfg.base_url, "v1/models");
        let response = self
            .client
            .get(url)
            .header("x-api-key", &self.cfg.api_key)
            .header("anthropic-version", "2023-06-01")
            .send()
            .await
            .map_err(|e| {
                PostError::failed_with_reason(
                    PostFailureReason::Network,
                    format!(
                        "{} ({}) model list request failed: {e}",
                        self.cfg.name, self.cfg.provider_name
                    ),
                )
            })?;
        self.parse_model_list_http_response(response, "anthropic")
            .await
    }

    async fn parse_model_list_http_response(
        &self,
        response: reqwest::Response,
        protocol: &str,
    ) -> Result<Vec<String>, PostError> {
        let status = response.status();
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            return Err(http_error(
                &self.cfg.name,
                &self.cfg.provider_name,
                protocol,
                status,
                &body,
            ));
        }
        let body: Value = response.json().await.map_err(|e| {
            PostError::failed_with_reason(
                PostFailureReason::InvalidResponse,
                format!("{} model list invalid json: {e}", self.cfg.name),
            )
        })?;
        parse_model_list_response(&body)
    }

    async fn call_openai(&self, prompt: &str) -> Result<String, PostError> {
        let url = join_url(&self.cfg.base_url, "chat/completions");
        let body = build_openai_request(&self.cfg, prompt);
        let response = self
            .client
            .post(url)
            .bearer_auth(&self.cfg.api_key)
            .json(&body)
            .send()
            .await
            .map_err(|e| {
                PostError::failed_with_reason(
                    PostFailureReason::Network,
                    format!(
                        "{} ({}) request failed: {e}",
                        self.cfg.name, self.cfg.provider_name
                    ),
                )
            })?;
        let status = response.status();
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            return Err(http_error(
                &self.cfg.name,
                &self.cfg.provider_name,
                "openai",
                status,
                &body,
            ));
        }
        let body: Value = response.json().await.map_err(|e| {
            PostError::failed_with_reason(
                PostFailureReason::InvalidResponse,
                format!("{} invalid json: {e}", self.cfg.name),
            )
        })?;
        parse_openai_response(&body)
    }

    async fn call_anthropic(&self, prompt: &str) -> Result<String, PostError> {
        let url = join_url(&self.cfg.base_url, "v1/messages");
        let body = build_anthropic_request(&self.cfg, prompt);
        let response = self
            .client
            .post(url)
            .header("x-api-key", &self.cfg.api_key)
            .header("anthropic-version", "2023-06-01")
            .json(&body)
            .send()
            .await
            .map_err(|e| {
                PostError::failed_with_reason(
                    PostFailureReason::Network,
                    format!(
                        "{} ({}) request failed: {e}",
                        self.cfg.name, self.cfg.provider_name
                    ),
                )
            })?;
        let status = response.status();
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            return Err(http_error(
                &self.cfg.name,
                &self.cfg.provider_name,
                "anthropic",
                status,
                &body,
            ));
        }
        let body: Value = response.json().await.map_err(|e| {
            PostError::failed_with_reason(
                PostFailureReason::InvalidResponse,
                format!("{} invalid json: {e}", self.cfg.name),
            )
        })?;
        parse_anthropic_response(&body)
    }
}

pub fn render_prompt(template: &str, ctx: &AppContext, text: &str) -> String {
    template
        .replace("{{app_name}}", ctx.app_name.as_deref().unwrap_or(""))
        .replace("{{bundle_id}}", ctx.bundle_id.as_deref().unwrap_or(""))
        .replace("{{text}}", text)
}

fn build_openai_request(cfg: &LlmCleanupConfig, prompt: &str) -> Value {
    let mut messages = Vec::new();
    if let Some(system) = cfg
        .system_prompt
        .as_deref()
        .filter(|s| !s.trim().is_empty())
    {
        messages.push(json!({ "role": "system", "content": system }));
    }
    messages.push(json!({ "role": "user", "content": prompt }));
    let mut body = json!({
        "model": cfg.model,
        "messages": messages,
    });
    for (key, value) in &cfg.extra_body {
        body[key] = value.clone();
    }
    body
}

fn build_anthropic_request(cfg: &LlmCleanupConfig, prompt: &str) -> Value {
    let mut body = json!({
        "model": cfg.model,
        "max_tokens": 2048,
        "messages": [{ "role": "user", "content": prompt }],
    });
    if let Some(system) = cfg
        .system_prompt
        .as_deref()
        .filter(|s| !s.trim().is_empty())
    {
        body["system"] = json!(system);
    }
    for (key, value) in &cfg.extra_body {
        body[key] = value.clone();
    }
    body
}

fn parse_openai_response(body: &Value) -> Result<String, PostError> {
    body.pointer("/choices/0/message/content")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(ToOwned::to_owned)
        .ok_or_else(|| {
            PostError::failed_with_reason(
                PostFailureReason::InvalidResponse,
                "openai response missing choices[0].message.content",
            )
        })
}

fn parse_anthropic_response(body: &Value) -> Result<String, PostError> {
    let parts = body
        .get("content")
        .and_then(Value::as_array)
        .ok_or_else(|| {
            PostError::failed_with_reason(
                PostFailureReason::InvalidResponse,
                "anthropic response missing content",
            )
        })?
        .iter()
        .filter_map(|part| {
            let is_text = part.get("type").and_then(Value::as_str) == Some("text");
            is_text
                .then(|| part.get("text").and_then(Value::as_str))
                .flatten()
        })
        .collect::<String>();
    let text = parts.trim();
    if text.is_empty() {
        return Err(PostError::failed_with_reason(
            PostFailureReason::InvalidResponse,
            "anthropic response has no text content",
        ));
    }
    Ok(text.to_string())
}

fn parse_model_list_response(body: &Value) -> Result<Vec<String>, PostError> {
    let data = body.get("data").and_then(Value::as_array).ok_or_else(|| {
        PostError::failed_with_reason(
            PostFailureReason::InvalidResponse,
            "model list response missing data array",
        )
    })?;
    let mut seen = HashSet::new();
    let models = data
        .iter()
        .filter_map(|item| item.get("id").and_then(Value::as_str))
        .filter(|id| !id.trim().is_empty())
        .filter(|id| seen.insert((*id).to_string()))
        .map(ToOwned::to_owned)
        .collect::<Vec<_>>();
    if models.is_empty() {
        return Err(PostError::failed_with_reason(
            PostFailureReason::InvalidResponse,
            "model list response has no model ids",
        ));
    }
    Ok(models)
}

fn join_url(base: &str, path: &str) -> String {
    format!(
        "{}/{}",
        base.trim_end_matches('/'),
        path.trim_start_matches('/')
    )
}

fn http_error_message(
    component: &str,
    provider: &str,
    protocol: &str,
    status: impl fmt::Display,
    body: &str,
) -> String {
    let prefix = format!("{component} ({provider}, {protocol}) http error {status}");
    match http_error_details(body) {
        Some(details) => format!("{prefix}; {details}"),
        None => prefix,
    }
}

fn http_error(
    component: &str,
    provider: &str,
    protocol: &str,
    status: StatusCode,
    body: &str,
) -> PostError {
    let fields = parse_error_fields(body);
    let reason = classify_http_error(status, fields.as_ref());
    PostError::failed_with_reason(
        reason,
        http_error_message(component, provider, protocol, status, body),
    )
}

fn http_error_details(body: &str) -> Option<String> {
    let trimmed = body.trim();
    if trimmed.is_empty() {
        return None;
    }

    if let Ok(value) = serde_json::from_str::<Value>(trimmed) {
        if let Some(details) = structured_error_details(&value) {
            return Some(details);
        }
    }

    Some("unstructured error body omitted".to_string())
}

fn structured_error_details(body: &Value) -> Option<String> {
    let error_fields = error_fields(body)?;
    let mut parts = Vec::new();
    push_error_text_field(&mut parts, "type", error_fields.type_.as_deref());
    push_error_text_field(&mut parts, "code", error_fields.code.as_deref());
    push_error_text_field(&mut parts, "message", error_fields.message.as_deref());
    (!parts.is_empty()).then(|| format!("error {}", parts.join(" ")))
}

#[derive(Debug, Clone, Default)]
struct ErrorFields {
    type_: Option<String>,
    code: Option<String>,
    message: Option<String>,
}

fn parse_error_fields(body: &str) -> Option<ErrorFields> {
    let value = serde_json::from_str::<Value>(body.trim()).ok()?;
    error_fields(&value)
}

fn error_fields(body: &Value) -> Option<ErrorFields> {
    let error = body.get("error").and_then(Value::as_object)?;
    Some(ErrorFields {
        type_: sanitize_error_value(error.get("type")),
        code: sanitize_error_value(error.get("code")),
        message: sanitize_error_value(error.get("message")),
    })
}

fn sanitize_error_value(value: Option<&Value>) -> Option<String> {
    let value = value?;
    let text = match value {
        Value::String(s) => sanitize_remote_string(s, 512),
        Value::Number(_) | Value::Bool(_) => sanitize_remote_string(&value.to_string(), 512),
        _ => return None,
    };
    (!text.is_empty()).then_some(text)
}

fn push_error_text_field(fields: &mut Vec<String>, name: &str, value: Option<&str>) {
    if let Some(text) = value.filter(|text| !text.is_empty()) {
        fields.push(format!("{name}={text}"));
    }
}

fn classify_http_error(status: StatusCode, fields: Option<&ErrorFields>) -> PostFailureReason {
    if status == StatusCode::UNAUTHORIZED {
        return PostFailureReason::AuthFailed;
    }
    if status == StatusCode::PAYMENT_REQUIRED {
        return PostFailureReason::QuotaOrBilling;
    }
    if status == StatusCode::FORBIDDEN {
        return PostFailureReason::PermissionDenied;
    }
    if status == StatusCode::NOT_FOUND {
        return classify_error_fields(fields).unwrap_or(PostFailureReason::ProviderError);
    }
    if status == StatusCode::REQUEST_TIMEOUT || status == StatusCode::GATEWAY_TIMEOUT {
        return PostFailureReason::Timeout;
    }
    if status == StatusCode::TOO_MANY_REQUESTS {
        return classify_error_fields(fields).unwrap_or(PostFailureReason::RateLimited);
    }
    if status.as_u16() == 529 {
        return PostFailureReason::ProviderOverloaded;
    }
    if status == StatusCode::BAD_REQUEST {
        return classify_error_fields(fields).unwrap_or(PostFailureReason::InvalidRequest);
    }
    if status.is_server_error() {
        return classify_error_fields(fields).unwrap_or(PostFailureReason::ProviderError);
    }
    classify_error_fields(fields).unwrap_or(PostFailureReason::ProviderError)
}

fn classify_error_fields(fields: Option<&ErrorFields>) -> Option<PostFailureReason> {
    let fields = fields?;
    let mut joined = String::new();
    for value in [&fields.type_, &fields.code, &fields.message]
        .into_iter()
        .flatten()
    {
        joined.push(' ');
        joined.push_str(&value.to_ascii_lowercase());
    }
    let text = joined.as_str();
    if text.contains("invalid_api_key")
        || text.contains("authentication")
        || text.contains("unauthorized")
        || text.contains("api key")
    {
        return Some(PostFailureReason::AuthFailed);
    }
    if text.contains("permission") || text.contains("forbidden") {
        return Some(PostFailureReason::PermissionDenied);
    }
    if text.contains("billing")
        || text.contains("insufficient_quota")
        || text.contains("quota")
        || text.contains("credit")
    {
        return Some(PostFailureReason::QuotaOrBilling);
    }
    if text.contains("rate_limit") || text.contains("rate limit") || text.contains("too many") {
        return Some(PostFailureReason::RateLimited);
    }
    if text.contains("model_not_found")
        || text.contains("model not found")
        || text.contains("model does not exist")
        || text.contains("supported api model names")
    {
        return Some(PostFailureReason::ModelNotFound);
    }
    if text.contains("timeout") || text.contains("timed out") {
        return Some(PostFailureReason::Timeout);
    }
    if text.contains("overload") || text.contains("overloaded") {
        return Some(PostFailureReason::ProviderOverloaded);
    }
    if text.contains("invalid_request") || text.contains("invalid request") {
        return Some(PostFailureReason::InvalidRequest);
    }
    None
}

fn sanitize_remote_string(text: &str, max_chars: usize) -> String {
    let normalized = normalize_control_chars(text);
    truncate_sanitized(&normalized, max_chars)
}

fn normalize_control_chars(text: &str) -> String {
    let mut out = String::new();
    let mut last_was_space = false;
    for ch in text.trim().chars() {
        let mapped = if ch.is_control() { ' ' } else { ch };
        if mapped.is_whitespace() {
            if !last_was_space {
                out.push(' ');
                last_was_space = true;
            }
        } else {
            out.push(mapped);
            last_was_space = false;
        }
    }
    out.trim().to_string()
}

fn truncate_sanitized(body: &str, max_chars: usize) -> String {
    let mut chars = body.chars();
    let mut out = chars.by_ref().take(max_chars).collect::<String>();
    if chars.next().is_some() {
        out.push_str("... [truncated]");
    }
    out
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn prompt_template_replaces_app_and_text_fields() {
        let ctx = AppContext {
            bundle_id: Some("com.apple.dt.Xcode".to_string()),
            app_name: Some("Xcode".to_string()),
        };

        let rendered = render_prompt(
            "app={{app_name}} bundle={{bundle_id}} text={{text}}",
            &ctx,
            "hello",
        );

        assert_eq!(rendered, "app=Xcode bundle=com.apple.dt.Xcode text=hello");
    }

    #[test]
    fn parses_openai_compatible_response_text() {
        let body = json!({
            "choices": [{
                "message": { "content": "cleaned text" }
            }]
        });

        assert_eq!(parse_openai_response(&body).unwrap(), "cleaned text");
    }

    #[test]
    fn parses_anthropic_response_text() {
        let body = json!({
            "content": [
                { "type": "text", "text": "cleaned" },
                { "type": "text", "text": " text" }
            ]
        });

        assert_eq!(parse_anthropic_response(&body).unwrap(), "cleaned text");
    }

    #[test]
    fn parses_provider_model_lists() {
        let openai = json!({
            "object": "list",
            "data": [
                { "id": "gpt-4.1-mini", "object": "model" },
                { "id": "whisper-1", "object": "model" },
                { "id": "gpt-4.1-mini", "object": "model" }
            ]
        });
        let anthropic = json!({
            "data": [
                {
                    "type": "model",
                    "id": "claude-haiku-4-5",
                    "display_name": "Claude Haiku 4.5"
                },
                {
                    "type": "model",
                    "id": "claude-sonnet-4-6",
                    "display_name": "Claude Sonnet 4.6"
                }
            ],
            "has_more": false
        });

        assert_eq!(
            parse_model_list_response(&openai).unwrap(),
            vec!["gpt-4.1-mini".to_string(), "whisper-1".to_string()]
        );
        assert_eq!(
            parse_model_list_response(&anthropic).unwrap(),
            vec![
                "claude-haiku-4-5".to_string(),
                "claude-sonnet-4-6".to_string()
            ]
        );
    }

    #[test]
    fn openai_request_uses_system_and_user_messages() {
        let cfg = LlmCleanupConfig {
            name: "clean".to_string(),
            format: ProviderFormat::OpenAi,
            provider_name: "openai".to_string(),
            base_url: "https://api.openai.com/v1".to_string(),
            api_key: "secret".to_string(),
            model: "gpt-4o-mini".to_string(),
            extra_body: Map::new(),
            system_prompt: Some("system".to_string()),
            prompt: "{{text}}".to_string(),
        };

        let body = build_openai_request(&cfg, "hello");

        assert_eq!(body["model"], "gpt-4o-mini");
        assert_eq!(body["messages"][0]["role"], "system");
        assert_eq!(body["messages"][1]["content"], "hello");
        assert!(body.get("api_key").is_none());
    }

    #[test]
    fn openai_request_includes_provider_extra_body() {
        let mut extra_body = Map::new();
        extra_body.insert("thinking".to_string(), json!({ "type": "disabled" }));
        let cfg = LlmCleanupConfig {
            name: "clean".to_string(),
            format: ProviderFormat::OpenAi,
            provider_name: "deepseek".to_string(),
            base_url: "https://api.deepseek.com".to_string(),
            api_key: "secret".to_string(),
            model: "deepseek-v4-flash".to_string(),
            extra_body,
            system_prompt: None,
            prompt: "{{text}}".to_string(),
        };

        let body = build_openai_request(&cfg, "hello");

        assert_eq!(body["thinking"], json!({ "type": "disabled" }));
        assert!(body.get("api_key").is_none());
    }

    #[test]
    fn anthropic_request_includes_provider_extra_body() {
        let mut extra_body = Map::new();
        extra_body.insert(
            "thinking".to_string(),
            json!({ "type": "enabled", "budget_tokens": 1024 }),
        );
        extra_body.insert("metadata".to_string(), json!({ "user_id": "shuohua" }));
        let cfg = LlmCleanupConfig {
            name: "clean".to_string(),
            format: ProviderFormat::Anthropic,
            provider_name: "anthropic".to_string(),
            base_url: "https://api.anthropic.com".to_string(),
            api_key: "secret".to_string(),
            model: "claude-sonnet-4-5".to_string(),
            extra_body,
            system_prompt: None,
            prompt: "{{text}}".to_string(),
        };

        let body = build_anthropic_request(&cfg, "hello");

        assert_eq!(
            body["thinking"],
            json!({ "type": "enabled", "budget_tokens": 1024 })
        );
        assert_eq!(body["metadata"], json!({ "user_id": "shuohua" }));
        assert!(body.get("api_key").is_none());
    }

    #[test]
    fn http_error_message_preserves_structured_openai_error_and_sanitizes_strings() {
        let body = json!({
            "error": {
                "type": "invalid_request_error\nforged",
                "code": "bad\tcode",
                "message": "first line\r\nsecond line",
            }
        })
        .to_string();

        let message = http_error_message("clean", "openai", "openai", 400, &body);

        assert_eq!(
            message,
            "clean (openai, openai) http error 400; error type=invalid_request_error forged code=bad code message=first line second line"
        );
        assert!(!message.contains('\n'));
        assert!(!message.contains('\r'));
        assert!(!message.contains('\t'));
    }

    #[test]
    fn openai_compatible_http_error_classifies_model_not_found() {
        let body = json!({
            "error": {
                "type": "invalid_request_error",
                "code": "model_not_found",
                "message": "model does not exist",
            }
        })
        .to_string();

        let error = http_error("clean", "openai", "openai", StatusCode::NOT_FOUND, &body);

        assert_eq!(
            error.reason(),
            crate::post::PostFailureReason::ModelNotFound
        );
        assert!(error.to_string().contains("model_not_found"));
    }

    #[test]
    fn generic_not_found_http_error_does_not_claim_model_not_found() {
        let body = json!({
            "error": {
                "type": "not_found_error",
                "code": "not_found",
                "message": "route not found",
            }
        })
        .to_string();

        let error = http_error("clean", "custom", "openai", StatusCode::NOT_FOUND, &body);

        assert_eq!(
            error.reason(),
            crate::post::PostFailureReason::ProviderError
        );
        assert!(error.to_string().contains("route not found"));
    }

    #[test]
    fn openai_compatible_http_error_classifies_deepseek_supported_model_message() {
        let body = json!({
            "error": {
                "type": "invalid_request_error",
                "code": "invalid_request_error",
                "message": "The supported API model names are deepseek-v4-pro or deepseek-v4-flash, but you passed shuo-test-model-not-found.",
            }
        })
        .to_string();

        let error = http_error(
            "clean",
            "deepseek",
            "openai",
            StatusCode::BAD_REQUEST,
            &body,
        );

        assert_eq!(
            error.reason(),
            crate::post::PostFailureReason::ModelNotFound
        );
    }

    #[test]
    fn http_error_message_preserves_structured_anthropic_error() {
        let body = json!({
            "type": "error",
            "error": {
                "type": "rate_limit_error",
                "message": "too many requests",
            }
        })
        .to_string();

        let message = http_error_message("clean", "anthropic", "anthropic", 429, &body);

        assert_eq!(
            message,
            "clean (anthropic, anthropic) http error 429; error type=rate_limit_error message=too many requests"
        );
    }

    #[test]
    fn anthropic_http_error_classifies_rate_limit_from_error_type() {
        let body = json!({
            "type": "error",
            "error": {
                "type": "rate_limit_error",
                "message": "too many requests",
            }
        })
        .to_string();

        let error = http_error(
            "clean",
            "anthropic",
            "anthropic",
            StatusCode::TOO_MANY_REQUESTS,
            &body,
        );

        assert_eq!(error.reason(), crate::post::PostFailureReason::RateLimited);
        assert!(error.to_string().contains("rate_limit_error"));
    }

    #[test]
    fn http_error_message_omits_unknown_body() {
        let body = format!("{}\nPROMPT: secret tail", "x".repeat(3000));

        let message = http_error_message("clean", "custom", "openai", 500, &body);

        assert!(message.contains("clean (custom, openai) http error 500"));
        assert!(message.contains("unstructured error body omitted"));
        assert!(!message.contains("PROMPT"));
        assert!(!message.contains(&"x".repeat(128)));
        assert!(!message.contains('\n'));
    }
}
