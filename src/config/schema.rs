use crate::config::spec::{ConfigSpec, FieldSpec};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SchemaId {
    Main,
    AsrApple,
    AsrDoubao,
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
        "asr/apple.toml" => Some(spec_for(SchemaId::AsrApple)),
        "asr/doubao.toml" => Some(spec_for(SchemaId::AsrDoubao)),
        _ if path.starts_with("post/rule/") && path.ends_with(".toml") => {
            Some(spec_for(SchemaId::PostRule))
        }
        _ if path.starts_with("post/llm/") && path.ends_with(".toml") => {
            Some(spec_for(SchemaId::PostLlm))
        }
        _ if path.starts_with("theme/") && path.ends_with(".toml") => {
            Some(spec_for(SchemaId::Theme))
        }
        _ => None,
    }
}

fn field(kind: fn(&'static str) -> FieldSpec, name: &'static str) -> FieldSpec {
    kind(name).description_key(description_key(name))
}

fn description_key(name: &str) -> &'static str {
    match name {
        "hotkey" => "config.field.hotkey.description",
        "hotkey.trigger" => "config.field.hotkey.trigger.description",
        "hotkey.cancel" => "config.field.hotkey.cancel.description",
        "voice" => "config.field.voice.description",
        "voice.stop_delay_ms" => "config.field.voice.stop_delay_ms.description",
        "voice.record_audio" => "config.field.voice.record_audio.description",
        "voice.auto_paste" => "config.field.voice.auto_paste.description",
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
        "overlay.max_text_lines" => "config.field.overlay.max_text_lines.description",
        "overlay.thinking_delay_ms" => "config.field.overlay.thinking_delay_ms.description",
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
        "overlay.glass" => "config.field.theme.overlay.glass.description",
        "overlay.glass.variant" => "config.field.theme.overlay.glass.variant.description",
        "overlay.glass.style" => "config.field.theme.overlay.glass.style.description",
        "overlay.glass.subdued" => "config.field.theme.overlay.glass.subdued.description",
        "overlay.surface" => "config.field.theme.overlay.surface.description",
        "overlay.surface.background" => "config.field.theme.overlay.surface.background.description",
        "overlay.surface.background_alpha" => {
            "config.field.theme.overlay.surface.background_alpha.description"
        }
        "overlay.surface.background_blur_radius" => {
            "config.field.theme.overlay.surface.background_blur_radius.description"
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
        "idle_pause" => "config.field.idle_pause.description",
        "finalize_timeout_ms" => "config.field.finalize_timeout_ms.description",
        "app_key" => "config.field.app_key.description",
        "access_key" => "config.field.access_key.description",
        "resource_id" => "config.field.resource_id.description",
        "enable_itn" => "config.field.enable_itn.description",
        "enable_punc" => "config.field.enable_punc.description",
        "enable_ddc" => "config.field.enable_ddc.description",
        "stream_mode" => "config.field.stream_mode.description",
        "ai_vad" => "config.field.ai_vad.description",
        "name" => "config.field.name.description",
        "asr" => "config.field.asr.description",
        "asr.provider" => "config.field.asr.provider.description",
        "asr.hotwords" => "config.field.asr.hotwords.description",
        "post.chain" => "config.field.post.chain.description",
        "post.llm" => "config.field.post.llm.description",
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
        .field(field(FieldSpec::string, "hotkey.trigger").required())
        .field(field(FieldSpec::string, "hotkey.cancel").optional())
        .field(field(FieldSpec::table, "voice").optional())
        .field(field(FieldSpec::integer, "voice.stop_delay_ms").optional())
        .field(
            field(FieldSpec::string, "voice.record_audio")
                .optional()
                .allowed_values(["off", "lossless", "compact"]),
        )
        .field(field(FieldSpec::bool, "voice.auto_paste").optional())
        .field(field(FieldSpec::table, "voice.vad").optional())
        .field(
            field(FieldSpec::string, "voice.vad.backend")
                .optional()
                .allowed_values(["off", "silero"]),
        )
        .field(field(FieldSpec::float, "voice.vad.threshold").optional())
        .field(field(FieldSpec::integer, "voice.vad.pause_silence_ms").optional())
        .field(field(FieldSpec::integer, "voice.vad.pre_roll_ms").optional())
        .field(field(FieldSpec::integer, "voice.vad.max_overlap_ms").optional())
        .field(field(FieldSpec::integer, "voice.vad.min_start_voiced_frames").optional())
        .field(field(FieldSpec::table, "dev").optional())
        .field(field(FieldSpec::bool, "dev.vad_trace").optional())
        .field(field(FieldSpec::table, "post").optional())
        .field(field(FieldSpec::integer, "post.timeout_ms").optional())
        .field(field(FieldSpec::table, "profile").optional().free_table())
        .field(field(FieldSpec::table, "ui").optional())
        .field(field(FieldSpec::string, "ui.language").optional())
        .field(field(FieldSpec::string, "ui.theme").optional())
        .field(field(FieldSpec::string, "ui.theme_tui").optional())
        .field(field(FieldSpec::string, "ui.theme_overlay").optional())
        .field(field(FieldSpec::table, "overlay").optional())
        .field(
            field(FieldSpec::string, "overlay.position")
                .optional()
                .allowed_values(["top", "middle", "bottom"]),
        )
        .field(field(FieldSpec::integer, "overlay.max_text_lines").optional())
        .field(field(FieldSpec::integer, "overlay.thinking_delay_ms").optional())
}

pub fn asr_apple_spec() -> ConfigSpec {
    ConfigSpec::new("asr.apple")
        .field(field(FieldSpec::string, "language").optional())
        .field(field(FieldSpec::bool, "install_assets").optional())
        .field(field(FieldSpec::bool, "idle_pause").optional())
        .field(field(FieldSpec::integer, "finalize_timeout_ms").optional())
}

pub fn asr_doubao_spec() -> ConfigSpec {
    ConfigSpec::new("asr.doubao")
        .field(field(FieldSpec::string, "app_key").required().secret())
        .field(field(FieldSpec::string, "access_key").required().secret())
        .field(field(FieldSpec::string, "resource_id").optional())
        .field(field(FieldSpec::string, "language").optional())
        .field(field(FieldSpec::bool, "enable_itn").optional())
        .field(field(FieldSpec::bool, "enable_punc").optional())
        .field(field(FieldSpec::bool, "enable_ddc").optional())
        .field(field(FieldSpec::integer, "stream_mode").optional())
        .field(field(FieldSpec::bool, "ai_vad").optional())
        .field(field(FieldSpec::bool, "idle_pause").optional())
        .field(field(FieldSpec::integer, "finalize_timeout_ms").optional())
}

pub fn profile_spec() -> ConfigSpec {
    ConfigSpec::new("profile")
        .field(field(FieldSpec::string, "name").required())
        .field(field(FieldSpec::table, "asr").required())
        .field(field(FieldSpec::string, "asr.provider").required())
        .field(field(FieldSpec::array, "asr.hotwords").optional())
        .field(field(FieldSpec::table, "post").optional())
        .field(field(FieldSpec::array, "post.chain").optional())
        .field(field(FieldSpec::table, "post.llm").optional().free_table())
}

pub fn post_rule_spec() -> ConfigSpec {
    ConfigSpec::new("post.rule")
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
        .field(field(FieldSpec::string, "name").required())
        .field(field(FieldSpec::string, "base_url").optional())
        .field(field(FieldSpec::string, "api_key").required().secret())
        .field(field(FieldSpec::string, "model").required())
        .field(field(FieldSpec::string, "system_prompt").optional())
        .field(field(FieldSpec::string, "prompt").required())
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
        .field(field(FieldSpec::table, "overlay.glass").optional())
        .field(field(FieldSpec::integer, "overlay.glass.variant").optional())
        .field(
            field(FieldSpec::string, "overlay.glass.style")
                .optional()
                .allowed_values(["clear", "blur"]),
        )
        .field(field(FieldSpec::integer, "overlay.glass.subdued").optional())
        .field(field(FieldSpec::table, "overlay.surface").optional())
        .field(field(FieldSpec::color, "overlay.surface.background").optional())
        .field(field(FieldSpec::float, "overlay.surface.background_alpha").optional())
        .field(field(FieldSpec::integer, "overlay.surface.background_blur_radius").optional())
        .field(field(FieldSpec::float, "overlay.surface.corner_radius").optional())
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

    #[test]
    fn registry_resolves_known_paths() {
        assert!(spec_for_path("config.toml")
            .unwrap()
            .field_for_path("hotkey.trigger")
            .is_some());
        assert!(spec_for_path("profile/default.toml")
            .unwrap()
            .field_for_path("asr.provider")
            .is_some());
        assert!(spec_for_path("post/llm/openai.toml")
            .unwrap()
            .field_for_path("api_key")
            .unwrap()
            .is_secret());
    }

    #[test]
    fn generated_template_fields_have_description_keys() {
        for id in [
            SchemaId::Main,
            SchemaId::AsrApple,
            SchemaId::AsrDoubao,
            SchemaId::Profile,
            SchemaId::PostRule,
            SchemaId::PostLlm,
            SchemaId::Theme,
        ] {
            for field in spec_for(id).fields() {
                assert!(
                    field.description_key_value().is_some(),
                    "{id:?} {} missing description key",
                    field.name()
                );
            }
        }
    }
}
