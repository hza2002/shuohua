use std::path::PathBuf;

use serde::Deserialize;
use toml::value::Table;

#[derive(Debug, Clone, Deserialize)]
pub struct DoubaoConfig {
    pub app_key: String,
    pub access_key: String,
    #[serde(default = "default_resource_id")]
    pub resource_id: String,
    /// 留空 = bigmodel_async 自动中英混合识别（默认推荐，中英混杂技术词汇友好）。
    /// 设置 `"zh-CN"` / `"en-US"` 等强制单语，换更高单语 confidence。
    /// 优先级：本字段 > `SessionCtx.language`（voice 层目前固定 Multilingual）。
    #[serde(default)]
    pub language: Option<String>,
    #[serde(default = "default_true")]
    pub enable_itn: bool,
    #[serde(default = "default_true")]
    pub enable_punc: bool,
    /// 服务端去口语词。我们本地 PostProcessor 也做一遍，双重保险。
    #[serde(default = "default_true")]
    pub enable_ddc: bool,
    /// 实验：StreamMode。0=流式 I/O，1=流式输入一次性输出，2=双向流式优化（火山推荐）。
    /// `None` = 不发字段走服务端默认。直连 WS 是否支持未文档化，实测中。
    #[serde(default)]
    pub stream_mode: Option<u8>,
    /// 实验：启用服务端 AI VAD（语义级句末检测）。理论上减少"半句被切成 definite"。
    /// `None` / `false` = 不发字段。字段名按 RTC 文档结构映射 `vad_config.ai_vad`，
    /// 直连 WS 不接受会触发 server protocol error，到时换名重试。
    #[serde(default)]
    pub ai_vad: Option<bool>,
    /// 允许 voice 层用本地 VAD 切分本 provider 的 session。默认关。
    #[serde(default)]
    pub idle_pause: bool,
    /// voice 发出 `is_last=true` 后最多等多久 provider finalize（毫秒）。
    #[serde(default = "default_finalize_timeout_ms")]
    pub finalize_timeout_ms: u64,
}

pub(crate) fn default_resource_id() -> String {
    "volc.bigasr.sauc.duration".into()
}

fn default_true() -> bool {
    true
}

pub(crate) fn default_finalize_timeout_ms() -> u64 {
    // 12s 取自 openless 实测经验值；正常情况下 < 1s 就能拿到 Done，这是给罕见
    // server 长尾留的 budget。改协议（带 sequence）之后应该几乎不会触发。
    12_000
}

pub fn config_path() -> PathBuf {
    if let Ok(xdg) = std::env::var("XDG_CONFIG_HOME") {
        return PathBuf::from(xdg).join("shuohua/asr/doubao.toml");
    }
    let home = std::env::var("HOME").unwrap_or_default();
    PathBuf::from(home).join(".config/shuohua/asr/doubao.toml")
}

pub fn load_config_with_overrides(overrides: Option<&Table>) -> anyhow::Result<DoubaoConfig> {
    let path = config_path();
    let body = std::fs::read_to_string(&path).map_err(|e| {
        anyhow::anyhow!(
            "doubao config not found at {}: {e}\n\
             hint: create {} and fill in app_key/access_key",
            path.display(),
            path.display(),
        )
    })?;
    let mut value: toml::Value =
        toml::from_str(&body).map_err(|e| anyhow::anyhow!("parse {}: {e}", path.display()))?;
    if let Some(overrides) = overrides {
        let table = value.as_table_mut().ok_or_else(|| {
            anyhow::anyhow!("parse {}: expected top-level TOML table", path.display())
        })?;
        for (key, value) in overrides {
            table.insert(key.clone(), value.clone());
        }
    }
    let mut cfg: DoubaoConfig = value
        .try_into()
        .map_err(|e| anyhow::anyhow!("parse {}: {e}", path.display()))?;
    // 控制台复制粘贴常带首尾空格，进协议帧前裁掉，避免 401。
    cfg.app_key = cfg.app_key.trim().to_string();
    cfg.access_key = cfg.access_key.trim().to_string();
    if cfg.app_key.is_empty() || cfg.access_key.is_empty() {
        anyhow::bail!(
            "{}: app_key / access_key 为空。从 console.volcengine.com/speech 拿一对填进去",
            path.display()
        );
    }
    Ok(cfg)
}
