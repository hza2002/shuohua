use super::{AppContext, PipelineText, PostError, PostProcessor};
use async_trait::async_trait;
use serde_json::{json, Map, Value};
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
            ..AppContext::default()
        };
        let prompt = render_prompt(&self.cfg.prompt, &ctx, "doctor runtime check");
        match self.cfg.format {
            ProviderFormat::OpenAi => self.call_openai(&prompt).await.map(|_| ()),
            ProviderFormat::Anthropic => self.call_anthropic(&prompt).await.map(|_| ()),
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
                PostError::Failed(format!(
                    "{} ({}) request failed: {e}",
                    self.cfg.name, self.cfg.provider_name
                ))
            })?;
        let status = response.status();
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            return Err(PostError::Failed(http_error_message(
                &self.cfg.name,
                &self.cfg.provider_name,
                "openai",
                status,
                &body,
            )));
        }
        let body: Value = response
            .json()
            .await
            .map_err(|e| PostError::Failed(format!("{} invalid json: {e}", self.cfg.name)))?;
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
                PostError::Failed(format!(
                    "{} ({}) request failed: {e}",
                    self.cfg.name, self.cfg.provider_name
                ))
            })?;
        let status = response.status();
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            return Err(PostError::Failed(http_error_message(
                &self.cfg.name,
                &self.cfg.provider_name,
                "anthropic",
                status,
                &body,
            )));
        }
        let body: Value = response
            .json()
            .await
            .map_err(|e| PostError::Failed(format!("{} invalid json: {e}", self.cfg.name)))?;
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
            PostError::Failed("openai response missing choices[0].message.content".into())
        })
}

fn parse_anthropic_response(body: &Value) -> Result<String, PostError> {
    let parts = body
        .get("content")
        .and_then(Value::as_array)
        .ok_or_else(|| PostError::Failed("anthropic response missing content".into()))?
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
        return Err(PostError::Failed(
            "anthropic response has no text content".into(),
        ));
    }
    Ok(text.to_string())
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

    let excerpt = sanitize_remote_string(trimmed, 512);
    (!excerpt.is_empty()).then(|| format!("body excerpt: {excerpt}"))
}

fn structured_error_details(body: &Value) -> Option<String> {
    let error = body.get("error").and_then(Value::as_object)?;
    let mut fields = Vec::new();
    push_error_field(&mut fields, "type", error.get("type"));
    push_error_field(&mut fields, "code", error.get("code"));
    push_error_field(&mut fields, "message", error.get("message"));
    (!fields.is_empty()).then(|| format!("error {}", fields.join(" ")))
}

fn push_error_field(fields: &mut Vec<String>, name: &str, value: Option<&Value>) {
    let Some(value) = value else {
        return;
    };
    let text = match value {
        Value::String(s) => sanitize_remote_string(s, 512),
        Value::Number(_) | Value::Bool(_) => sanitize_remote_string(&value.to_string(), 512),
        _ => return,
    };
    if !text.is_empty() {
        fields.push(format!("{name}={text}"));
    }
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
            ..AppContext::default()
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
    fn http_error_message_uses_short_sanitized_excerpt_for_unknown_body() {
        let body = format!("{}\nPROMPT: secret tail", "x".repeat(3000));

        let message = http_error_message("clean", "custom", "openai", 500, &body);

        assert!(message.contains("clean (custom, openai) http error 500"));
        assert!(message.contains("body excerpt: "));
        assert!(message.contains("truncated"));
        assert!(!message.contains("PROMPT"));
        assert!(!message.contains('\n'));
    }
}
