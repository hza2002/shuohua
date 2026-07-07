use crate::config::spec::{ConfigSpec, FieldSpec};

use crate::config::asr::options::{
    values, APPLE_LANGUAGE_VALUES, DOUBAO_LANGUAGE_VALUES, LOCAL_VAD_VALUES,
    TENCENT_ENGINE_MODEL_TYPE_VALUES,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SchemaId {
    Main,
    AsrApple,
    AsrDoubao,
    AsrTencent,
    Profile,
    PostRule,
    PostLlm,
    Theme,
}

pub fn spec_for(id: SchemaId) -> ConfigSpec {
    match id {
        SchemaId::Main => main_spec(),
        SchemaId::AsrApple => asr_apple_spec(),
        SchemaId::AsrDoubao => asr_doubao_spec(),
        SchemaId::AsrTencent => asr_tencent_spec(),
        SchemaId::Profile => profile_spec(),
        SchemaId::PostRule => post_rule_spec(),
        SchemaId::PostLlm => post_llm_spec(),
        SchemaId::Theme => theme_spec(),
    }
}

pub fn spec_for_path(path: &str) -> Option<ConfigSpec> {
    if path == "config.toml" {
        return Some(spec_for(SchemaId::Main));
    }
    if path.starts_with("profile/") && path.ends_with(".toml") {
        return Some(spec_for(SchemaId::Profile));
    }
    match path {
        _ if path.starts_with("theme/") && path.ends_with(".toml") => {
            Some(spec_for(SchemaId::Theme))
        }
        _ => None,
    }
}

pub fn asr_spec_for_value(
    id: &str,
    path: &std::path::Path,
    value: &toml::Value,
) -> anyhow::Result<ConfigSpec> {
    let kind = crate::config::asr::instance::kind_from_value(id, path, value)?;
    Ok(spec_for(kind.schema_id()))
}

pub fn post_spec_for_value(
    id: &str,
    path: &std::path::Path,
    value: &toml::Value,
) -> anyhow::Result<ConfigSpec> {
    let kind = crate::config::post::kind_from_value(id, path, value)?;
    Ok(spec_for(kind.schema_id()))
}

/// Value-aware spec for a real config file.
///
/// - `source` is the real filesystem path.  It is used to detect whether the
///   file lives inside an `asr/` or `post/` directory (case-sensitive match on
///   the parent component) and to read and parse the file contents when
///   selecting the provider/type-specific spec.
/// - `rel` is the logical config-relative path (e.g. `"asr/doubao.toml"`).  It
///   is only used for the non-value-aware `spec_for_path` fallback; asr/post
///   resolution ignores it.
///
/// Returns:
/// - `None` → not a spec-managed file
/// - `Some(Err(e))` → IS an asr/post file but its `type` field is missing or
///   invalid (`e` carries a user-facing message from `kind_from_value`)
/// - `Some(Ok(spec))` → resolved spec
///
/// Note: the `asr/`/`post/` directory match is case-sensitive.
pub fn spec_for_config_file(
    source: &std::path::Path,
    rel: &str,
) -> Option<anyhow::Result<ConfigSpec>> {
    let is_toml = source.extension().and_then(|e| e.to_str()) == Some("toml");
    let parent = source
        .parent()
        .and_then(|p| p.file_name())
        .and_then(|n| n.to_str());
    if is_toml && matches!(parent, Some("asr") | Some("post")) {
        let stem = source.file_stem().and_then(|s| s.to_str())?;
        let value = match std::fs::read_to_string(source)
            .map_err(|e| anyhow::anyhow!("read {}: {e}", source.display()))
            .and_then(|body| {
                body.parse::<toml::Value>()
                    .map_err(|e| anyhow::anyhow!("parse {}: {e}", source.display()))
            }) {
            Ok(v) => v,
            Err(e) => return Some(Err(e)),
        };
        return Some(match parent {
            Some("asr") => asr_spec_for_value(stem, source, &value),
            _ => post_spec_for_value(stem, source, &value),
        });
    }
    spec_for_path(rel).map(Ok)
}

fn field(kind: fn(&'static str) -> FieldSpec, name: &'static str) -> FieldSpec {
    kind(name).description_key(description_key(name))
}

fn description_key(name: &str) -> &'static str {
    match name {
        "hotkey" => "config.field.hotkey.description",
        "hotkey.trigger" => "config.field.hotkey.trigger.description",
        "hotkey.cancel" => "config.field.hotkey.cancel.description",
        "hotkey.resume" => "config.field.hotkey.resume.description",
        "voice" => "config.field.voice.description",
        "voice.stop_delay_ms" => "config.field.voice.stop_delay_ms.description",
        "voice.record_audio" => "config.field.voice.record_audio.description",
        "voice.auto_paste" => "config.field.voice.auto_paste.description",
        "voice.preprocess" => "config.field.voice.preprocess.description",
        "voice.preprocess.backend" => "config.field.voice.preprocess.backend.description",
        "voice.vad" => "config.field.voice.vad.description",
        "voice.vad.backend" => "config.field.voice.vad.backend.description",
        "voice.vad.threshold" => "config.field.voice.vad.threshold.description",
        "voice.vad.pause_silence_ms" => "config.field.voice.vad.pause_silence_ms.description",
        "voice.vad.pre_roll_ms" => "config.field.voice.vad.pre_roll_ms.description",
        "voice.vad.max_overlap_ms" => "config.field.voice.vad.max_overlap_ms.description",
        "voice.vad.min_start_voiced_frames" => {
            "config.field.voice.vad.min_start_voiced_frames.description"
        }
        "dev" => "config.field.dev.description",
        "dev.vad_trace" => "config.field.dev.vad_trace.description",
        "dev.apple_backend_trace" => "config.field.dev.apple_backend_trace.description",
        "post" => "config.field.post.description",
        "post.timeout_ms" => "config.field.post.timeout_ms.description",
        "profile" => "config.field.profile.description",
        "ui" => "config.field.ui.description",
        "ui.language" => "config.field.ui.language.description",
        "ui.theme" => "config.field.ui.theme.description",
        "ui.theme_tui" => "config.field.ui.theme_tui.description",
        "ui.theme_overlay" => "config.field.ui.theme_overlay.description",
        "overlay" => "config.field.overlay.description",
        "overlay.position" => "config.field.overlay.position.description",
        "overlay.width" => "config.field.overlay.width.description",
        "overlay.max_text_lines" => "config.field.overlay.max_text_lines.description",
        "palette" => "config.field.theme.palette.description",
        "foreground" => "config.field.theme.foreground.description",
        "muted" => "config.field.theme.muted.description",
        "accent" => "config.field.theme.accent.description",
        "success" => "config.field.theme.success.description",
        "warning" => "config.field.theme.warning.description",
        "error" => "config.field.theme.error.description",
        "info" => "config.field.theme.info.description",
        "highlight" => "config.field.theme.highlight.description",
        "border" => "config.field.theme.border.description",
        "border_focus" => "config.field.theme.border_focus.description",
        "segment" => "config.field.theme.segment.description",
        "overlay.macos" => "config.field.theme.overlay.macos.description",
        "overlay.macos.glass_variant" => {
            "config.field.theme.overlay.macos.glass_variant.description"
        }
        "overlay.macos.glass_style" => "config.field.theme.overlay.macos.glass_style.description",
        "overlay.macos.subdued" => "config.field.theme.overlay.macos.subdued.description",
        "overlay.macos.background_blur_radius" => {
            "config.field.theme.overlay.macos.background_blur_radius.description"
        }
        "overlay.surface" => "config.field.theme.overlay.surface.description",
        "overlay.surface.background" => "config.field.theme.overlay.surface.background.description",
        "overlay.surface.background_alpha" => {
            "config.field.theme.overlay.surface.background_alpha.description"
        }
        "overlay.surface.corner_radius" => {
            "config.field.theme.overlay.surface.corner_radius.description"
        }
        "overlay.text" => "config.field.theme.overlay.text.description",
        "overlay.text.primary" => "config.field.theme.overlay.text.primary.description",
        "overlay.text.secondary" => "config.field.theme.overlay.text.secondary.description",
        "overlay.text.tertiary" => "config.field.theme.overlay.text.tertiary.description",
        "overlay.text.segment" => "config.field.theme.overlay.text.segment.description",
        "overlay.text.notice" => "config.field.theme.overlay.text.notice.description",
        "overlay.text.error" => "config.field.theme.overlay.text.error.description",
        "overlay.state" => "config.field.theme.overlay.state.description",
        "overlay.state.idle" => "config.field.theme.overlay.state.idle.description",
        "overlay.state.connecting" => "config.field.theme.overlay.state.connecting.description",
        "overlay.state.recording" => "config.field.theme.overlay.state.recording.description",
        "overlay.state.thinking" => "config.field.theme.overlay.state.thinking.description",
        "overlay.state.stopping" => "config.field.theme.overlay.state.stopping.description",
        "overlay.state.error" => "config.field.theme.overlay.state.error.description",
        "language" => "config.field.language.description",
        "install_assets" => "config.field.install_assets.description",
        "local_vad" => "config.field.local_vad.description",
        "open_timeout_ms" => "config.field.open_timeout_ms.description",
        "finalize_timeout_ms" => "config.field.finalize_timeout_ms.description",
        "app_key" => "config.field.app_key.description",
        "access_key" => "config.field.access_key.description",
        "app_id" => "config.field.app_id.description",
        "secret_id" => "config.field.secret_id.description",
        "secret_key" => "config.field.secret_key.description",
        "engine_model_type" => "config.field.engine_model_type.description",
        "need_vad" => "config.field.need_vad.description",
        "filter_dirty" => "config.field.filter_dirty.description",
        "filter_modal" => "config.field.filter_modal.description",
        "filter_punc" => "config.field.filter_punc.description",
        "convert_num_mode" => "config.field.convert_num_mode.description",
        "vad_silence_time" => "config.field.vad_silence_time.description",
        "max_speak_time" => "config.field.max_speak_time.description",
        "noise_threshold" => "config.field.noise_threshold.description",
        "hotword_weight" => "config.field.hotword_weight.description",
        "hotword_id" => "config.field.hotword_id.description",
        "customization_id" => "config.field.customization_id.description",
        "replace_text_id" => "config.field.replace_text_id.description",
        "sentence_strategy" => "config.field.sentence_strategy.description",
        "resource_id" => "config.field.resource_id.description",
        "enable_itn" => "config.field.enable_itn.description",
        "enable_punc" => "config.field.enable_punc.description",
        "enable_ddc" => "config.field.enable_ddc.description",
        "stream_mode" => "config.field.stream_mode.description",
        "ai_vad" => "config.field.ai_vad.description",
        "name" => "config.field.name.description",
        "asr" => "config.field.asr.description",
        "asr.instance" => "config.field.asr.instance.description",
        "asr.hotwords" => "config.field.asr.hotwords.description",
        "post.chain" => "config.field.post.chain.description",
        "post.overrides" => "config.field.post.overrides.description",
        "type" => "config.field.type.description",
        "patterns" => "config.field.patterns.description",
        "format" => "config.field.format.description",
        "base_url" => "config.field.base_url.description",
        "api_key" => "config.field.api_key.description",
        "model" => "config.field.model.description",
        "system_prompt" => "config.field.system_prompt.description",
        "prompt" => "config.field.prompt.description",
        "extra_body" => "config.field.extra_body.description",
        _ => "config.field.unknown.description",
    }
}

pub fn main_spec() -> ConfigSpec {
    ConfigSpec::new("config")
        .field(field(FieldSpec::table, "hotkey").required())
        .field(
            field(FieldSpec::string, "hotkey.trigger")
                .required()
                .keycapture(),
        )
        .field(
            field(FieldSpec::string, "hotkey.cancel")
                .optional()
                .default("escape")
                .keycapture(),
        )
        .field(
            field(FieldSpec::string, "hotkey.resume")
                .optional()
                .default("shift+right_option:double")
                .keycapture(),
        )
        .field(field(FieldSpec::table, "voice").optional())
        .field(
            field(FieldSpec::integer, "voice.stop_delay_ms")
                .optional()
                .range(0.0, 5000.0)
                .default("800"),
        )
        .field(
            field(FieldSpec::string, "voice.record_audio")
                .optional()
                .allowed_values(["off", "lossless", "compact"])
                .default("off"),
        )
        .field(
            field(FieldSpec::bool, "voice.auto_paste")
                .optional()
                .default("true"),
        )
        .field(field(FieldSpec::table, "voice.preprocess").optional())
        .field(
            field(FieldSpec::string, "voice.preprocess.backend")
                .optional()
                .allowed_values(["off", "apple", "webrtc"])
                .default("webrtc"),
        )
        .field(field(FieldSpec::table, "voice.vad").optional())
        .field(
            field(FieldSpec::string, "voice.vad.backend")
                .optional()
                .allowed_values(["off", "silero"])
                .default("silero"),
        )
        .field(
            field(FieldSpec::float, "voice.vad.threshold")
                .optional()
                .range(0.0, 1.0)
                .default("0.5"),
        )
        .field(
            field(FieldSpec::integer, "voice.vad.pause_silence_ms")
                .optional()
                .range(200.0, 10_000.0)
                .default("1500"),
        )
        .field(
            field(FieldSpec::integer, "voice.vad.pre_roll_ms")
                .optional()
                .range(0.0, 2000.0)
                .default("300"),
        )
        .field(
            field(FieldSpec::integer, "voice.vad.max_overlap_ms")
                .optional()
                .range(0.0, 2000.0)
                .default("200"),
        )
        .field(
            field(FieldSpec::integer, "voice.vad.min_start_voiced_frames")
                .optional()
                .range(1.0, 20.0)
                .default("2"),
        )
        .field(field(FieldSpec::table, "dev").optional())
        .field(
            field(FieldSpec::bool, "dev.vad_trace")
                .optional()
                .default("false"),
        )
        .field(
            field(FieldSpec::bool, "dev.apple_backend_trace")
                .optional()
                .default("false"),
        )
        .field(field(FieldSpec::table, "post").optional())
        .field(
            field(FieldSpec::integer, "post.timeout_ms")
                .optional()
                .range(100.0, 60_000.0)
                .default("30000"),
        )
        .field(field(FieldSpec::table, "profile").optional().free_table())
        .field(field(FieldSpec::table, "ui").optional())
        .field(
            field(FieldSpec::string, "ui.language")
                .optional()
                .default("auto"),
        )
        .field(
            field(FieldSpec::string, "ui.theme")
                .optional()
                .default(crate::config::theme::DEFAULT_THEME_NAME),
        )
        .field(
            field(FieldSpec::string, "ui.theme_tui")
                .optional()
                .default(""),
        )
        .field(
            field(FieldSpec::string, "ui.theme_overlay")
                .optional()
                .default(""),
        )
        .field(field(FieldSpec::table, "overlay").optional())
        .field(
            field(FieldSpec::string, "overlay.position")
                .optional()
                .allowed_values(["top", "middle", "bottom"])
                .default("bottom"),
        )
        .field(
            field(FieldSpec::integer, "overlay.width")
                .optional()
                .range(480.0, 900.0)
                .default(crate::overlay::layout::constants::DEFAULT_WIDTH_PX.to_string()),
        )
        .field(
            field(FieldSpec::integer, "overlay.max_text_lines")
                .optional()
                .min(1.0)
                .default("5"),
        )
}

pub fn asr_apple_spec() -> ConfigSpec {
    ConfigSpec::new("asr.apple")
        .field(
            field(FieldSpec::string, "type")
                .required()
                .allowed_values(["apple"]),
        )
        .field(field(FieldSpec::string, "name").optional())
        .field(
            field(FieldSpec::string, "language")
                .optional()
                .allowed_values(values(APPLE_LANGUAGE_VALUES)),
        )
        .field(
            field(FieldSpec::bool, "install_assets")
                .optional()
                .default("true"),
        )
        .field(
            field(FieldSpec::string, "local_vad")
                .optional()
                .allowed_values(values(LOCAL_VAD_VALUES))
                .default("off"),
        )
        .field(
            field(FieldSpec::integer, "open_timeout_ms")
                .optional()
                .range(1000.0, 120_000.0)
                .default("5000"),
        )
        .field(
            field(FieldSpec::integer, "finalize_timeout_ms")
                .optional()
                .range(1000.0, 60_000.0)
                .default("5000"),
        )
}

pub fn asr_doubao_spec() -> ConfigSpec {
    ConfigSpec::new("asr.doubao")
        .field(
            field(FieldSpec::string, "type")
                .required()
                .allowed_values(["doubao"]),
        )
        .field(field(FieldSpec::string, "name").optional())
        .field(field(FieldSpec::string, "app_key").required().secret())
        .field(field(FieldSpec::string, "access_key").required().secret())
        .field(
            field(FieldSpec::string, "resource_id")
                .optional()
                .default("volc.bigasr.sauc.duration"),
        )
        .field(
            field(FieldSpec::string, "language")
                .optional()
                .allowed_values(values(DOUBAO_LANGUAGE_VALUES))
                .default("auto"),
        )
        .field(
            field(FieldSpec::bool, "enable_itn")
                .optional()
                .default("true"),
        )
        .field(
            field(FieldSpec::bool, "enable_punc")
                .optional()
                .default("true"),
        )
        .field(
            field(FieldSpec::bool, "enable_ddc")
                .optional()
                .default("true"),
        )
        .field(
            field(FieldSpec::integer, "stream_mode")
                .optional()
                .range(0.0, 2.0)
                .default("2"),
        )
        .field(field(FieldSpec::bool, "ai_vad").optional().default("false"))
        .field(
            field(FieldSpec::string, "local_vad")
                .optional()
                .allowed_values(values(LOCAL_VAD_VALUES))
                .default("auto"),
        )
        .field(
            field(FieldSpec::integer, "open_timeout_ms")
                .optional()
                .range(1000.0, 120_000.0)
                .default("12000"),
        )
        .field(
            field(FieldSpec::integer, "finalize_timeout_ms")
                .optional()
                .range(1000.0, 60_000.0)
                .default("12000"),
        )
}

pub fn asr_tencent_spec() -> ConfigSpec {
    ConfigSpec::new("asr.tencent")
        .field(
            field(FieldSpec::string, "type")
                .required()
                .allowed_values(["tencent"]),
        )
        .field(field(FieldSpec::string, "name").optional())
        .field(field(FieldSpec::string, "app_id").required())
        .field(field(FieldSpec::string, "secret_id").required().secret())
        .field(field(FieldSpec::string, "secret_key").required().secret())
        .field(
            field(FieldSpec::string, "engine_model_type")
                .optional()
                .allowed_values(values(TENCENT_ENGINE_MODEL_TYPE_VALUES))
                .default("16k_zh"),
        )
        .field(
            field(FieldSpec::integer, "convert_num_mode")
                .optional()
                .range(0.0, 3.0)
                .default("1"),
        )
        .field(
            field(FieldSpec::integer, "filter_modal")
                .optional()
                .range(0.0, 2.0)
                .default("1"),
        )
        .field(
            field(FieldSpec::bool, "filter_punc")
                .optional()
                .default("false"),
        )
        .field(
            field(FieldSpec::integer, "filter_dirty")
                .optional()
                .range(0.0, 2.0)
                .default("0"),
        )
        .field(
            field(FieldSpec::bool, "need_vad")
                .optional()
                .default("false"),
        )
        .field(
            field(FieldSpec::integer, "vad_silence_time")
                .optional()
                .range(500.0, 2000.0)
                .default("1000"),
        )
        .field(
            field(FieldSpec::integer, "max_speak_time")
                .optional()
                .range(5000.0, 90_000.0)
                .default("60000"),
        )
        .field(
            field(FieldSpec::integer, "sentence_strategy")
                .optional()
                .range(0.0, 1.0)
                .default("0"),
        )
        .field(
            field(FieldSpec::float, "noise_threshold")
                .optional()
                .range(-2.0, 2.0)
                .default("0"),
        )
        .field(
            field(FieldSpec::integer, "hotword_weight")
                .optional()
                .range(1.0, 100.0)
                .default("10"),
        )
        .field(field(FieldSpec::string, "hotword_id").optional())
        .field(field(FieldSpec::string, "customization_id").optional())
        .field(field(FieldSpec::string, "replace_text_id").optional())
        .field(
            field(FieldSpec::string, "local_vad")
                .optional()
                .allowed_values(values(LOCAL_VAD_VALUES))
                .default("auto"),
        )
        .field(
            field(FieldSpec::integer, "open_timeout_ms")
                .optional()
                .range(1000.0, 120_000.0)
                .default("12000"),
        )
        .field(
            field(FieldSpec::integer, "finalize_timeout_ms")
                .optional()
                .range(1000.0, 60_000.0)
                .default("12000"),
        )
}

pub fn profile_spec() -> ConfigSpec {
    ConfigSpec::new("profile")
        .field(field(FieldSpec::string, "name").optional())
        .field(field(FieldSpec::table, "asr").required().free_table())
        .field(field(FieldSpec::string, "asr.instance").required())
        .field(field(FieldSpec::array, "asr.hotwords").optional())
        .field(field(FieldSpec::table, "post").optional())
        .field(field(FieldSpec::array, "post.chain").optional())
        .field(
            field(FieldSpec::table, "post.overrides")
                .optional()
                .free_table(),
        )
}

pub fn post_rule_spec() -> ConfigSpec {
    ConfigSpec::new("post.rule")
        .field(field(FieldSpec::string, "name").optional())
        .field(
            field(FieldSpec::string, "type")
                .required()
                .allowed_values(["rule"]),
        )
        .field(field(FieldSpec::array, "patterns").required())
}

pub fn post_llm_spec() -> ConfigSpec {
    ConfigSpec::new("post.llm")
        .field(
            field(FieldSpec::string, "type")
                .required()
                .allowed_values(["llm"]),
        )
        .field(
            field(FieldSpec::string, "format")
                .default("openai")
                .allowed_values(["openai", "anthropic"]),
        )
        .field(field(FieldSpec::string, "name").optional())
        .field(field(FieldSpec::string, "base_url").required())
        .field(field(FieldSpec::string, "api_key").required().secret())
        .field(field(FieldSpec::string, "model").required())
        .field(
            field(FieldSpec::string, "system_prompt")
                .optional()
                .multiline(),
        )
        .field(field(FieldSpec::string, "prompt").required().multiline())
        .field(
            field(FieldSpec::table, "extra_body")
                .optional()
                .free_table(),
        )
}

pub fn theme_spec() -> ConfigSpec {
    ConfigSpec::new("theme")
        .field(field(FieldSpec::string, "name").optional())
        .field(field(FieldSpec::table, "palette").optional().free_table())
        .field(field(FieldSpec::table, "tui").optional())
        .field(field(FieldSpec::color, "tui.foreground").optional())
        .field(field(FieldSpec::color, "tui.muted").optional())
        .field(field(FieldSpec::color, "tui.accent").optional())
        .field(field(FieldSpec::color, "tui.success").optional())
        .field(field(FieldSpec::color, "tui.warning").optional())
        .field(field(FieldSpec::color, "tui.error").optional())
        .field(field(FieldSpec::color, "tui.info").optional())
        .field(field(FieldSpec::color, "tui.highlight").optional())
        .field(field(FieldSpec::color, "tui.border").optional())
        .field(field(FieldSpec::color, "tui.border_focus").optional())
        .field(field(FieldSpec::color, "tui.segment").optional())
        .field(field(FieldSpec::table, "overlay").optional())
        .field(field(FieldSpec::table, "overlay.macos").optional())
        .field(field(FieldSpec::integer, "overlay.macos.glass_variant").optional())
        .field(
            field(FieldSpec::string, "overlay.macos.glass_style")
                .optional()
                .allowed_values(["clear", "blur"]),
        )
        .field(field(FieldSpec::integer, "overlay.macos.subdued").optional())
        .field(field(FieldSpec::integer, "overlay.macos.background_blur_radius").optional())
        .field(field(FieldSpec::table, "overlay.surface").optional())
        .field(field(FieldSpec::color, "overlay.surface.background").optional())
        .field(
            field(FieldSpec::float, "overlay.surface.background_alpha")
                .optional()
                .range(0.0, 1.0),
        )
        .field(
            field(FieldSpec::float, "overlay.surface.corner_radius")
                .optional()
                .range(0.0, 40.0),
        )
        .field(field(FieldSpec::table, "overlay.text").optional())
        .field(field(FieldSpec::color, "overlay.text.primary").optional())
        .field(field(FieldSpec::color, "overlay.text.secondary").optional())
        .field(field(FieldSpec::color, "overlay.text.tertiary").optional())
        .field(field(FieldSpec::color, "overlay.text.segment").optional())
        .field(field(FieldSpec::color, "overlay.text.notice").optional())
        .field(field(FieldSpec::color, "overlay.text.error").optional())
        .field(field(FieldSpec::table, "overlay.state").optional())
        .field(field(FieldSpec::color, "overlay.state.idle").optional())
        .field(field(FieldSpec::color, "overlay.state.connecting").optional())
        .field(field(FieldSpec::color, "overlay.state.recording").optional())
        .field(field(FieldSpec::color, "overlay.state.thinking").optional())
        .field(field(FieldSpec::color, "overlay.state.stopping").optional())
        .field(field(FieldSpec::color, "overlay.state.error").optional())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::spec::validate_value;

    const ALL_SCHEMA_IDS: &[SchemaId] = &[
        SchemaId::Main,
        SchemaId::AsrApple,
        SchemaId::AsrDoubao,
        SchemaId::AsrTencent,
        SchemaId::Profile,
        SchemaId::PostRule,
        SchemaId::PostLlm,
        SchemaId::Theme,
    ];

    #[test]
    fn registry_resolves_known_paths() {
        assert!(spec_for_path("config.toml")
            .unwrap()
            .field_for_path("hotkey.trigger")
            .is_some());
        assert!(spec_for_path("profile/default.toml")
            .unwrap()
            .field_for_path("asr.instance")
            .is_some());
        // Flat post files no longer resolve by path; they need value-aware resolution.
        assert!(spec_for_path("post/openai.toml").is_none());
    }

    #[test]
    fn post_llm_requires_base_url_and_allows_optional_name() {
        let spec = spec_for(SchemaId::PostLlm);
        let missing_base: toml::Value =
            "type=\"llm\"\nname=\"x\"\napi_key=\"k\"\nmodel=\"m\"\nprompt=\"{{text}}\"\n"
                .parse()
                .unwrap();
        assert!(validate_value(&spec, &missing_base)
            .iter()
            .any(|d| d.path == "base_url"));
        let no_name: toml::Value = "type=\"llm\"\nbase_url=\"https://a\"\napi_key=\"k\"\nmodel=\"m\"\nprompt=\"{{text}}\"\n".parse().unwrap();
        assert!(!validate_value(&spec, &no_name)
            .iter()
            .any(|d| d.path == "name"));
    }

    #[test]
    fn optional_display_name_fields_declared_for_all_instance_types() {
        for id in [
            SchemaId::AsrApple,
            SchemaId::AsrDoubao,
            SchemaId::AsrTencent,
            SchemaId::PostRule,
            SchemaId::PostLlm,
            SchemaId::Profile,
        ] {
            let spec = spec_for(id);
            let field = spec
                .field_for_path("name")
                .unwrap_or_else(|| panic!("{id:?} should declare optional name"));
            assert!(!field.required_without_default(), "{id:?} name is optional");
        }
    }

    #[test]
    fn generated_template_fields_have_description_keys() {
        for &id in ALL_SCHEMA_IDS {
            for field in spec_for(id).fields() {
                assert!(
                    field.description_key_value().is_some(),
                    "{id:?} {} missing description key",
                    field.name()
                );
            }
        }
    }

    #[test]
    fn schema_description_keys_exist_in_base_locales() {
        for &id in ALL_SCHEMA_IDS {
            for field in spec_for(id).fields() {
                let key = field
                    .description_key_value()
                    .unwrap_or_else(|| panic!("{id:?} {} missing description key", field.name()));
                assert_ne!(
                    crate::i18n::tr_lang(crate::i18n::Lang::EnUS, key, &[]),
                    key,
                    "{id:?} {} missing en-US key {key}",
                    field.name()
                );
                assert_ne!(
                    crate::i18n::tr_lang(crate::i18n::Lang::ZhCN, key, &[]),
                    key,
                    "{id:?} {} missing zh-CN key {key}",
                    field.name()
                );
            }
        }
    }

    #[test]
    fn official_template_values_are_declared_in_schema() {
        for template in crate::config::template::registry() {
            let spec = template.spec();
            let value: toml::Value = toml::from_str(&crate::config::template::render(template))
                .unwrap_or_else(|error| panic!("{} rendered invalid TOML: {error}", template.id));
            let diagnostics = validate_value(&spec, &value)
                .into_iter()
                .filter(|diagnostic| {
                    !spec.field_for_path(&diagnostic.path).is_some_and(|field| {
                        field.is_secret()
                            && diagnostic.message.contains("secret field cannot be empty")
                    })
                })
                .collect::<Vec<_>>();
            assert!(
                diagnostics.is_empty(),
                "{} template does not match schema: {diagnostics:?}",
                template.id
            );
        }
    }

    #[test]
    fn main_schema_defaults_match_serde_effective_values() {
        use crate::config::main;

        let cfg = main::parse("[hotkey]\ntrigger = \"f16\"\n").expect("minimal config parses");

        let effective: &[(&str, String)] = &[
            ("hotkey.cancel", cfg.hotkey.cancel.clone()),
            ("hotkey.resume", cfg.hotkey.resume.clone()),
            ("voice.stop_delay_ms", cfg.voice.stop_delay_ms.to_string()),
            ("voice.record_audio", cfg.voice.record_audio.to_string()),
            ("voice.auto_paste", cfg.voice.auto_paste.to_string()),
            (
                "voice.preprocess.backend",
                format!("{:?}", cfg.voice.preprocess.backend).to_lowercase(),
            ),
            (
                "voice.vad.backend",
                format!("{:?}", cfg.voice.vad.backend).to_lowercase(),
            ),
            ("voice.vad.threshold", cfg.voice.vad.threshold.to_string()),
            (
                "voice.vad.pause_silence_ms",
                cfg.voice.vad.pause_silence_ms.to_string(),
            ),
            (
                "voice.vad.pre_roll_ms",
                cfg.voice.vad.pre_roll_ms.to_string(),
            ),
            (
                "voice.vad.max_overlap_ms",
                cfg.voice.vad.max_overlap_ms.to_string(),
            ),
            (
                "voice.vad.min_start_voiced_frames",
                cfg.voice.vad.min_start_voiced_frames.to_string(),
            ),
            ("dev.vad_trace", cfg.dev.vad_trace.to_string()),
            (
                "dev.apple_backend_trace",
                cfg.dev.apple_backend_trace.to_string(),
            ),
            ("post.timeout_ms", cfg.post.timeout_ms.to_string()),
            ("ui.language", cfg.ui.language.clone()),
            ("ui.theme", cfg.ui.theme.clone()),
            ("ui.theme_tui", cfg.ui.theme_tui.clone()),
            ("ui.theme_overlay", cfg.ui.theme_overlay.clone()),
            (
                "overlay.position",
                format!("{:?}", cfg.overlay.position).to_lowercase(),
            ),
            ("overlay.width", cfg.overlay.width.to_string()),
            (
                "overlay.max_text_lines",
                cfg.overlay.max_text_lines.to_string(),
            ),
        ];

        let spec = spec_for(SchemaId::Main);
        for (path, value) in effective {
            let declared = spec
                .field_for_path(path)
                .unwrap_or_else(|| panic!("{path} declared in schema"))
                .default_value()
                .unwrap_or_else(|| panic!("{path} has a schema default"));
            assert_eq!(declared, value, "schema default drift at {path}");
        }
    }

    #[test]
    fn main_schema_allows_webrtc_preprocess_backend() {
        let spec = spec_for(SchemaId::Main);
        let value = toml::Value::Table(toml::toml! {
            hotkey = { trigger = "f16" }
            voice = { preprocess = { backend = "webrtc" } }
        });

        assert!(validate_value(&spec, &value).is_empty());
    }

    #[test]
    fn main_schema_allows_large_overlay_text_viewport() {
        let spec = spec_for(SchemaId::Main);
        let value = toml::Value::Table(toml::toml! {
            hotkey = { trigger = "f16" }
            overlay = { max_text_lines = 200 }
        });

        assert!(validate_value(&spec, &value).is_empty());
    }

    #[test]
    fn main_hotkey_fields_are_keycapture() {
        let spec = spec_for(SchemaId::Main);

        for path in ["hotkey.trigger", "hotkey.cancel", "hotkey.resume"] {
            let field = spec
                .field_for_path(path)
                .unwrap_or_else(|| panic!("{path} declared in schema"));
            assert!(field.is_keycapture(), "{path} should use keycapture UI");
        }
    }

    #[test]
    fn asr_schema_defaults_match_serde_effective_values() {
        use crate::config::asr::apple::AppleConfig;

        let apple = AppleConfig::default();
        let apple_expected: &[(&str, String)] = &[
            ("install_assets", apple.install_assets.to_string()),
            ("local_vad", "off".to_string()),
            ("open_timeout_ms", apple.open_timeout_ms.to_string()),
            ("finalize_timeout_ms", apple.finalize_timeout_ms.to_string()),
        ];
        let apple_spec = spec_for(SchemaId::AsrApple);
        for (path, value) in apple_expected {
            let declared = apple_spec
                .field_for_path(path)
                .unwrap_or_else(|| panic!("{path} declared in apple schema"))
                .default_value()
                .unwrap_or_else(|| panic!("{path} has a schema default"));
            assert_eq!(declared, value, "apple schema default drift at {path}");
        }

        use crate::config::asr::doubao::{
            default_finalize_timeout_ms, default_open_timeout_ms, default_resource_id,
        };
        // enable_itn/enable_punc/enable_ddc have no pub accessor; they are trivial constants.
        let doubao_expected: &[(&str, String)] = &[
            ("resource_id", default_resource_id()),
            ("language", "auto".to_string()),
            ("enable_itn", "true".to_string()),
            ("enable_punc", "true".to_string()),
            ("enable_ddc", "true".to_string()),
            ("stream_mode", "2".to_string()),
            ("ai_vad", "false".to_string()),
            ("local_vad", "auto".to_string()),
            ("open_timeout_ms", default_open_timeout_ms().to_string()),
            (
                "finalize_timeout_ms",
                default_finalize_timeout_ms().to_string(),
            ),
        ];
        let doubao_spec = spec_for(SchemaId::AsrDoubao);
        for (path, value) in doubao_expected {
            let declared = doubao_spec
                .field_for_path(path)
                .unwrap_or_else(|| panic!("{path} declared in doubao schema"))
                .default_value()
                .unwrap_or_else(|| panic!("{path} has a schema default"));
            assert_eq!(
                declared,
                value.as_str(),
                "doubao schema default drift at {path}"
            );
        }

        use crate::config::asr::tencent::{
            default_engine_model_type, default_finalize_timeout_ms as tencent_finalize_timeout_ms,
            default_open_timeout_ms as tencent_open_timeout_ms,
        };
        let tencent_expected: &[(&str, String)] = &[
            ("engine_model_type", default_engine_model_type()),
            ("need_vad", "false".to_string()),
            ("filter_modal", "1".to_string()),
            ("filter_punc", "false".to_string()),
            ("convert_num_mode", "1".to_string()),
            ("hotword_weight", "10".to_string()),
            ("local_vad", "auto".to_string()),
            ("open_timeout_ms", tencent_open_timeout_ms().to_string()),
            (
                "finalize_timeout_ms",
                tencent_finalize_timeout_ms().to_string(),
            ),
        ];
        let tencent_spec = spec_for(SchemaId::AsrTencent);
        for (path, value) in tencent_expected {
            let declared = tencent_spec
                .field_for_path(path)
                .unwrap_or_else(|| panic!("{path} declared in tencent schema"))
                .default_value()
                .unwrap_or_else(|| panic!("{path} has a schema default"));
            assert_eq!(
                declared,
                value.as_str(),
                "tencent schema default drift at {path}"
            );
        }
    }

    #[test]
    fn runtime_parsers_reject_unknown_fields_across_config_kinds() {
        assert!(crate::config::main::parse(
            "[hotkey]\ntrigger = \"f16\"\n[voice]\nstop_delay_mss = 1\n"
        )
        .unwrap_err()
        .to_string()
        .contains("voice.stop_delay_mss"));

        let apple = toml::toml! {
            idle_paus = true
        }
        .into();
        assert!(validate_value(&spec_for(SchemaId::AsrApple), &apple)
            .iter()
            .any(|diagnostic| diagnostic.path == "idle_paus"));

        let post = toml::toml! {
            type = "rule"
            patterns = []
            typo = true
        }
        .into();
        assert!(validate_value(&spec_for(SchemaId::PostRule), &post)
            .iter()
            .any(|diagnostic| diagnostic.path == "typo"));

        let theme = toml::toml! {
            [overlay.surface]
            background_alfa = 0.5
        }
        .into();
        assert!(validate_value(&spec_for(SchemaId::Theme), &theme)
            .iter()
            .any(|diagnostic| diagnostic.path == "overlay.surface.background_alfa"));
    }

    #[test]
    fn asr_specs_require_type_with_matching_allowed_value() {
        let apple = asr_apple_spec();
        let doubao = asr_doubao_spec();
        let tencent = asr_tencent_spec();
        // Missing type is an error
        assert!(
            validate_value(&apple, &"name = \"x\"\n".parse::<toml::Value>().unwrap())
                .iter()
                .any(|d| d.path == "type"),
            "apple spec should require type"
        );
        assert!(
            validate_value(
                &doubao,
                &"app_key=\"a\"\naccess_key=\"b\"\n"
                    .parse::<toml::Value>()
                    .unwrap()
            )
            .iter()
            .any(|d| d.path == "type"),
            "doubao spec should require type"
        );
        assert!(
            validate_value(
                &tencent,
                &"app_id=\"1\"\nsecret_id=\"sid\"\nsecret_key=\"key\"\n"
                    .parse::<toml::Value>()
                    .unwrap()
            )
            .iter()
            .any(|d| d.path == "type"),
            "tencent spec should require type"
        );
        // Wrong type value is rejected by apple spec
        assert!(
            validate_value(
                &apple,
                &"type = \"doubao\"\n".parse::<toml::Value>().unwrap()
            )
            .iter()
            .any(|d| d.path == "type"),
            "apple spec should reject type=doubao"
        );
    }

    #[test]
    fn spec_for_config_file_selects_asr_spec_by_type() {
        let root = std::env::temp_dir().join(format!("shuohua-spec-{}", ulid::Ulid::new()));
        std::fs::create_dir_all(root.join("asr")).unwrap();
        let path = root.join("asr/team.toml");
        std::fs::write(
            &path,
            "type = \"doubao\"\napp_key=\"a\"\naccess_key=\"b\"\n",
        )
        .unwrap();
        let spec = spec_for_config_file(&path, "asr/team.toml")
            .unwrap()
            .unwrap();
        // doubao-only field present, apple-only field absent => doubao spec selected
        assert!(
            spec.field_for_path("app_key").is_some(),
            "doubao spec should have app_key"
        );
        assert!(
            spec.field_for_path("install_assets").is_none(),
            "doubao spec should not have install_assets"
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn spec_for_config_file_selects_post_spec_by_type() {
        let root = std::env::temp_dir().join(format!("shuohua-spec-{}", ulid::Ulid::new()));
        std::fs::create_dir_all(root.join("post")).unwrap();
        let path = root.join("post/team.toml");
        std::fs::write(
            &path,
            "type = \"llm\"\nname = \"team\"\napi_key = \"k\"\nmodel = \"m\"\nprompt = \"{{text}}\"\n",
        )
        .unwrap();
        let spec = spec_for_config_file(&path, "post/team.toml")
            .unwrap()
            .unwrap();
        // llm-only field present, rule-only field absent => llm spec selected
        assert!(
            spec.field_for_path("api_key").is_some(),
            "llm spec should have api_key"
        );
        assert!(
            spec.field_for_path("patterns").is_none(),
            "llm spec should not have patterns"
        );

        let rule_path = root.join("post/zh_filter.toml");
        std::fs::write(&rule_path, "type = \"rule\"\npatterns = []\n").unwrap();
        let rule_spec = spec_for_config_file(&rule_path, "post/zh_filter.toml")
            .unwrap()
            .unwrap();
        assert!(
            rule_spec.field_for_path("patterns").is_some(),
            "rule spec should have patterns"
        );
        assert!(
            rule_spec.field_for_path("api_key").is_none(),
            "rule spec should not have api_key"
        );
        let _ = std::fs::remove_dir_all(root);
    }
}
