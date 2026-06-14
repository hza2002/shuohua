use super::{AppContext, PipelineText, PostError, PostProcessor};
use async_trait::async_trait;
use serde_json::{json, Map, Value};

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
            })?
            .error_for_status()
            .map_err(|e| {
                PostError::Failed(format!(
                    "{} ({}) http error: {e}",
                    self.cfg.name, self.cfg.provider_name
                ))
            })?;
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
            })?
            .error_for_status()
            .map_err(|e| {
                PostError::Failed(format!(
                    "{} ({}) http error: {e}",
                    self.cfg.name, self.cfg.provider_name
                ))
            })?;
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
}
