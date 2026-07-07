use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

use super::{registry, Template, TemplateKind, TemplateValue};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AsrAppleDraft {
    pub file_id: String,
    pub name: String,
    pub language: String,
    pub install_assets: bool,
    pub local_vad: String,
    pub open_timeout_ms: i64,
    pub finalize_timeout_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AsrDoubaoDraft {
    pub file_id: String,
    pub name: String,
    pub app_key: String,
    pub access_key: String,
    pub resource_id: String,
    pub language: String,
    pub enable_itn: bool,
    pub enable_punc: bool,
    pub enable_ddc: bool,
    pub stream_mode: i64,
    pub ai_vad: bool,
    pub local_vad: String,
    pub open_timeout_ms: i64,
    pub finalize_timeout_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AsrTencentDraft {
    pub file_id: String,
    pub name: String,
    pub app_id: String,
    pub secret_id: String,
    pub secret_key: String,
    pub engine_model_type: String,
    pub need_vad: bool,
    pub filter_dirty: i64,
    pub filter_modal: i64,
    pub filter_punc: bool,
    pub convert_num_mode: i64,
    pub vad_silence_time: i64,
    pub max_speak_time: i64,
    pub noise_threshold: String,
    pub hotword_weight: i64,
    pub hotword_id: String,
    pub customization_id: String,
    pub replace_text_id: String,
    pub sentence_strategy: i64,
    pub local_vad: String,
    pub open_timeout_ms: i64,
    pub finalize_timeout_ms: i64,
}

pub fn asr_templates() -> impl Iterator<Item = &'static Template> {
    registry()
        .iter()
        .filter(|template| template.kind == TemplateKind::Asr)
}

pub fn asr_apple_from_template(template_id: &str) -> Option<AsrAppleDraft> {
    let template = asr_templates().find(|template| template.id == template_id)?;
    let values = template.values;
    let file_id = template_id
        .strip_prefix("asr/")
        .unwrap_or(template_id)
        .to_string();
    Some(AsrAppleDraft {
        file_id,
        name: string_value(values, "name")
            .unwrap_or("Apple Local ASR")
            .to_string(),
        language: string_value(values, "language")
            .unwrap_or("zh-CN")
            .to_string(),
        install_assets: bool_value(values, "install_assets").unwrap_or(true),
        local_vad: string_value(values, "local_vad")
            .unwrap_or("off")
            .to_string(),
        open_timeout_ms: integer_value(values, "open_timeout_ms").unwrap_or(5000),
        finalize_timeout_ms: integer_value(values, "finalize_timeout_ms").unwrap_or(5000),
    })
}

pub fn asr_doubao_from_template(template_id: &str) -> Option<AsrDoubaoDraft> {
    let template = asr_templates().find(|template| template.id == template_id)?;
    let values = template.values;
    let file_id = template_id
        .strip_prefix("asr/")
        .unwrap_or(template_id)
        .to_string();
    Some(AsrDoubaoDraft {
        file_id,
        name: string_value(values, "name")
            .unwrap_or("Doubao ASR")
            .to_string(),
        app_key: string_value(values, "app_key")
            .unwrap_or_default()
            .to_string(),
        access_key: string_value(values, "access_key")
            .unwrap_or_default()
            .to_string(),
        resource_id: string_value(values, "resource_id")
            .unwrap_or("volc.bigasr.sauc.duration")
            .to_string(),
        language: string_value(values, "language")
            .unwrap_or("auto")
            .to_string(),
        enable_itn: bool_value(values, "enable_itn").unwrap_or(true),
        enable_punc: bool_value(values, "enable_punc").unwrap_or(true),
        enable_ddc: bool_value(values, "enable_ddc").unwrap_or(true),
        stream_mode: integer_value(values, "stream_mode").unwrap_or(2),
        ai_vad: bool_value(values, "ai_vad").unwrap_or(false),
        local_vad: string_value(values, "local_vad")
            .unwrap_or("auto")
            .to_string(),
        open_timeout_ms: integer_value(values, "open_timeout_ms").unwrap_or(12_000),
        finalize_timeout_ms: integer_value(values, "finalize_timeout_ms").unwrap_or(12_000),
    })
}

pub fn asr_tencent_from_template(template_id: &str) -> Option<AsrTencentDraft> {
    let template = asr_templates().find(|template| template.id == template_id)?;
    let values = template.values;
    let file_id = template_id
        .strip_prefix("asr/")
        .unwrap_or(template_id)
        .to_string();
    Some(AsrTencentDraft {
        file_id,
        name: string_value(values, "name")
            .unwrap_or("Tencent ASR")
            .to_string(),
        app_id: string_value(values, "app_id")
            .unwrap_or_default()
            .to_string(),
        secret_id: string_value(values, "secret_id")
            .unwrap_or_default()
            .to_string(),
        secret_key: string_value(values, "secret_key")
            .unwrap_or_default()
            .to_string(),
        engine_model_type: string_value(values, "engine_model_type")
            .unwrap_or("16k_zh")
            .to_string(),
        need_vad: bool_value(values, "need_vad").unwrap_or(false),
        filter_dirty: integer_value(values, "filter_dirty").unwrap_or(0),
        filter_modal: integer_value(values, "filter_modal").unwrap_or(1),
        filter_punc: bool_value(values, "filter_punc").unwrap_or(false),
        convert_num_mode: integer_value(values, "convert_num_mode").unwrap_or(1),
        vad_silence_time: integer_value(values, "vad_silence_time").unwrap_or(1000),
        max_speak_time: integer_value(values, "max_speak_time").unwrap_or(60_000),
        noise_threshold: number_string_value(values, "noise_threshold")
            .unwrap_or_else(|| "0".to_string()),
        hotword_weight: integer_value(values, "hotword_weight").unwrap_or(10),
        hotword_id: string_value(values, "hotword_id")
            .unwrap_or_default()
            .to_string(),
        customization_id: string_value(values, "customization_id")
            .unwrap_or_default()
            .to_string(),
        replace_text_id: string_value(values, "replace_text_id")
            .unwrap_or_default()
            .to_string(),
        sentence_strategy: integer_value(values, "sentence_strategy").unwrap_or(0),
        local_vad: string_value(values, "local_vad")
            .unwrap_or("auto")
            .to_string(),
        open_timeout_ms: integer_value(values, "open_timeout_ms").unwrap_or(12_000),
        finalize_timeout_ms: integer_value(values, "finalize_timeout_ms").unwrap_or(12_000),
    })
}

pub fn render_asr_apple(draft: &AsrAppleDraft) -> Result<String> {
    anyhow::ensure!(!draft.file_id.trim().is_empty(), "file name is required");
    ensure_local_vad(&draft.local_vad)?;
    anyhow::ensure!(
        (1000..=120_000).contains(&draft.open_timeout_ms),
        "open_timeout_ms must be between 1000 and 120000"
    );
    anyhow::ensure!(
        (1000..=60_000).contains(&draft.finalize_timeout_ms),
        "finalize_timeout_ms must be between 1000 and 60000"
    );
    let mut body = String::new();
    body.push_str("type = \"apple\"\n");
    if !draft.name.trim().is_empty() {
        body.push_str(&format!("name = {:?}\n", draft.name));
    }
    if !draft.language.trim().is_empty() {
        body.push_str(&format!("language = {:?}\n", draft.language));
    }
    body.push_str(&format!("install_assets = {}\n", draft.install_assets));
    body.push_str(&format!("local_vad = {:?}\n", draft.local_vad));
    body.push_str(&format!("open_timeout_ms = {}\n", draft.open_timeout_ms));
    body.push_str(&format!(
        "finalize_timeout_ms = {}\n",
        draft.finalize_timeout_ms
    ));
    toml::from_str::<toml::Value>(&body).context("rendered Apple ASR template is invalid TOML")?;
    Ok(body)
}

pub fn render_asr_doubao(draft: &AsrDoubaoDraft) -> Result<String> {
    anyhow::ensure!(!draft.file_id.trim().is_empty(), "file name is required");
    ensure_local_vad(&draft.local_vad)?;
    anyhow::ensure!(!draft.app_key.trim().is_empty(), "app_key is required");
    anyhow::ensure!(
        !draft.access_key.trim().is_empty(),
        "access_key is required"
    );
    anyhow::ensure!(
        !draft.resource_id.trim().is_empty(),
        "resource_id is required"
    );
    anyhow::ensure!(
        (0..=2).contains(&draft.stream_mode),
        "stream_mode must be between 0 and 2"
    );
    anyhow::ensure!(
        (1000..=120_000).contains(&draft.open_timeout_ms),
        "open_timeout_ms must be between 1000 and 120000"
    );
    anyhow::ensure!(
        (1000..=60_000).contains(&draft.finalize_timeout_ms),
        "finalize_timeout_ms must be between 1000 and 60000"
    );

    let mut body = String::new();
    body.push_str("type = \"doubao\"\n");
    if !draft.name.trim().is_empty() {
        body.push_str(&format!("name = {:?}\n", draft.name));
    }
    body.push_str(&format!("app_key = {:?}\n", draft.app_key));
    body.push_str(&format!("access_key = {:?}\n", draft.access_key));
    body.push_str(&format!("resource_id = {:?}\n", draft.resource_id));
    body.push_str(&format!("language = {:?}\n", draft.language));
    body.push_str(&format!("enable_itn = {}\n", draft.enable_itn));
    body.push_str(&format!("enable_punc = {}\n", draft.enable_punc));
    body.push_str(&format!("enable_ddc = {}\n", draft.enable_ddc));
    body.push_str(&format!("stream_mode = {}\n", draft.stream_mode));
    body.push_str(&format!("ai_vad = {}\n", draft.ai_vad));
    body.push_str(&format!("local_vad = {:?}\n", draft.local_vad));
    body.push_str(&format!("open_timeout_ms = {}\n", draft.open_timeout_ms));
    body.push_str(&format!(
        "finalize_timeout_ms = {}\n",
        draft.finalize_timeout_ms
    ));
    toml::from_str::<toml::Value>(&body).context("rendered Doubao ASR template is invalid TOML")?;
    Ok(body)
}

pub fn render_asr_tencent(draft: &AsrTencentDraft) -> Result<String> {
    anyhow::ensure!(!draft.file_id.trim().is_empty(), "file name is required");
    ensure_local_vad(&draft.local_vad)?;
    anyhow::ensure!(!draft.app_id.trim().is_empty(), "app_id is required");
    anyhow::ensure!(!draft.secret_id.trim().is_empty(), "secret_id is required");
    anyhow::ensure!(
        !draft.secret_key.trim().is_empty(),
        "secret_key is required"
    );
    anyhow::ensure!(
        !draft.engine_model_type.trim().is_empty(),
        "engine_model_type is required"
    );
    anyhow::ensure!(
        matches!(draft.convert_num_mode, 0 | 1 | 3),
        "convert_num_mode must be 0, 1, or 3"
    );
    anyhow::ensure!(
        matches!(draft.hotword_weight, 1..=11 | 100),
        "hotword_weight must be between 1 and 11, or 100"
    );
    anyhow::ensure!(
        (0..=2).contains(&draft.filter_dirty),
        "filter_dirty must be between 0 and 2"
    );
    anyhow::ensure!(
        (0..=2).contains(&draft.filter_modal),
        "filter_modal must be between 0 and 2"
    );
    anyhow::ensure!(
        (500..=2000).contains(&draft.vad_silence_time),
        "vad_silence_time must be between 500 and 2000"
    );
    anyhow::ensure!(
        (5000..=90_000).contains(&draft.max_speak_time),
        "max_speak_time must be between 5000 and 90000"
    );
    let noise_threshold: f64 = draft
        .noise_threshold
        .trim()
        .parse()
        .context("noise_threshold must be a number")?;
    anyhow::ensure!(
        (-2.0..=2.0).contains(&noise_threshold),
        "noise_threshold must be between -2 and 2"
    );
    anyhow::ensure!(
        (0..=1).contains(&draft.sentence_strategy),
        "sentence_strategy must be 0 or 1"
    );
    anyhow::ensure!(
        (1000..=120_000).contains(&draft.open_timeout_ms),
        "open_timeout_ms must be between 1000 and 120000"
    );
    anyhow::ensure!(
        (1000..=60_000).contains(&draft.finalize_timeout_ms),
        "finalize_timeout_ms must be between 1000 and 60000"
    );

    let mut body = String::new();
    body.push_str("type = \"tencent\"\n");
    if !draft.name.trim().is_empty() {
        body.push_str(&format!("name = {:?}\n", draft.name));
    }
    body.push_str(&format!("app_id = {:?}\n", draft.app_id));
    body.push_str(&format!("secret_id = {:?}\n", draft.secret_id));
    body.push_str(&format!("secret_key = {:?}\n", draft.secret_key));
    body.push_str(&format!(
        "engine_model_type = {:?}\n",
        draft.engine_model_type
    ));
    body.push_str(&format!("convert_num_mode = {}\n", draft.convert_num_mode));
    body.push_str(&format!("filter_modal = {}\n", draft.filter_modal));
    body.push_str(&format!("filter_punc = {}\n", draft.filter_punc));
    body.push_str(&format!("filter_dirty = {}\n", draft.filter_dirty));
    body.push_str(&format!("need_vad = {}\n", draft.need_vad));
    body.push_str(&format!("vad_silence_time = {}\n", draft.vad_silence_time));
    body.push_str(&format!("max_speak_time = {}\n", draft.max_speak_time));
    body.push_str(&format!(
        "sentence_strategy = {}\n",
        draft.sentence_strategy
    ));
    body.push_str(&format!("noise_threshold = {}\n", noise_threshold));
    body.push_str(&format!("hotword_weight = {}\n", draft.hotword_weight));
    if !draft.hotword_id.trim().is_empty() {
        body.push_str(&format!("hotword_id = {:?}\n", draft.hotword_id));
    }
    if !draft.customization_id.trim().is_empty() {
        body.push_str(&format!(
            "customization_id = {:?}\n",
            draft.customization_id
        ));
    }
    if !draft.replace_text_id.trim().is_empty() {
        body.push_str(&format!("replace_text_id = {:?}\n", draft.replace_text_id));
    }
    body.push_str(&format!("local_vad = {:?}\n", draft.local_vad));
    body.push_str(&format!("open_timeout_ms = {}\n", draft.open_timeout_ms));
    body.push_str(&format!(
        "finalize_timeout_ms = {}\n",
        draft.finalize_timeout_ms
    ));
    toml::from_str::<toml::Value>(&body)
        .context("rendered Tencent ASR template is invalid TOML")?;
    Ok(body)
}

fn ensure_local_vad(value: &str) -> Result<()> {
    anyhow::ensure!(
        matches!(value, "auto" | "on" | "off"),
        "local_vad must be auto, on, or off"
    );
    Ok(())
}

pub fn create_asr_apple(asr_dir: &Path, draft: &AsrAppleDraft) -> Result<PathBuf> {
    write_asr_instance(asr_dir, &draft.file_id, render_asr_apple(draft)?)
}

pub fn create_asr_doubao(asr_dir: &Path, draft: &AsrDoubaoDraft) -> Result<PathBuf> {
    write_asr_instance(asr_dir, &draft.file_id, render_asr_doubao(draft)?)
}

pub fn create_asr_tencent(asr_dir: &Path, draft: &AsrTencentDraft) -> Result<PathBuf> {
    write_asr_instance(asr_dir, &draft.file_id, render_asr_tencent(draft)?)
}

fn write_asr_instance(asr_dir: &Path, file_id: &str, body: String) -> Result<PathBuf> {
    crate::config::inventory::validate_config_file_id(file_id)
        .map_err(anyhow::Error::msg)
        .context("invalid file name")?;
    let path = asr_dir.join(format!("{file_id}.toml"));
    anyhow::ensure!(
        !path.exists(),
        "ASR instance {file_id:?} already exists; pick a different file name"
    );
    std::fs::create_dir_all(asr_dir).with_context(|| format!("create {}", asr_dir.display()))?;
    std::fs::write(&path, body).with_context(|| format!("write {}", path.display()))?;
    Ok(path)
}

fn string_value<'a>(values: &'a [(&str, TemplateValue)], key: &str) -> Option<&'a str> {
    values.iter().find_map(|(name, value)| match (name, value) {
        (name, TemplateValue::String(value)) if *name == key => Some(*value),
        (name, TemplateValue::MultilineString(value)) if *name == key => Some(*value),
        _ => None,
    })
}

fn bool_value(values: &[(&str, TemplateValue)], key: &str) -> Option<bool> {
    values.iter().find_map(|(name, value)| match (name, value) {
        (name, TemplateValue::Bool(value)) if *name == key => Some(*value),
        _ => None,
    })
}

fn integer_value(values: &[(&str, TemplateValue)], key: &str) -> Option<i64> {
    values.iter().find_map(|(name, value)| match (name, value) {
        (name, TemplateValue::Integer(value)) if *name == key => Some(*value),
        _ => None,
    })
}

fn number_string_value(values: &[(&str, TemplateValue)], key: &str) -> Option<String> {
    values.iter().find_map(|(name, value)| match (name, value) {
        (name, TemplateValue::Integer(value)) if *name == key => Some(value.to_string()),
        (name, TemplateValue::Float(value)) if *name == key => Some(value.to_string()),
        (name, TemplateValue::String(value)) if *name == key => Some((*value).to_string()),
        _ => None,
    })
}
