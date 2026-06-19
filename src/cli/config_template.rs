use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use clap::Args;

#[derive(Debug, Args)]
pub struct ConfigTemplateArgs {
    /// Directory to write generated templates into.
    #[arg(long)]
    pub out: Option<PathBuf>,
    /// Comment language for generated templates: auto, en-US, zh-CN, zh-Hant, zh-TW, zh-HK, or pseudo.
    #[arg(long, default_value = "auto")]
    pub lang: String,
}

pub fn run(args: ConfigTemplateArgs) -> Result<()> {
    let out = args.out.unwrap_or_else(default_template_dir);
    let lang = template_lang(&args.lang);
    write_templates(&out, lang)?;
    println!(
        "{}",
        crate::i18n::tr(
            "cli.config_template.written",
            &[("path", out.display().to_string())]
        )
    );
    Ok(())
}

fn default_template_dir() -> PathBuf {
    crate::config::default_path()
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join("templates")
}

fn template_lang(value: &str) -> crate::i18n::Lang {
    let configured = if value == "auto" {
        crate::config::load_from(&crate::config::default_path())
            .map(|cfg| cfg.ui.language)
            .unwrap_or_else(|_| "auto".to_string())
    } else {
        value.to_string()
    };
    crate::i18n::resolve_lang(&configured)
}

fn write_templates(out: &Path, lang: crate::i18n::Lang) -> Result<()> {
    std::fs::create_dir_all(out).with_context(|| format!("create {}", out.display()))?;
    let templates = template_outputs(out, lang);
    let conflicts: Vec<_> = templates
        .iter()
        .filter(|(path, _)| path.exists())
        .map(|(path, _)| path.display().to_string())
        .collect();
    if !conflicts.is_empty() {
        anyhow::bail!(
            "{}\n{}",
            crate::i18n::tr("cli.config_template.refuse_overwrite", &[]),
            conflicts.join("\n")
        );
    }
    for (path, body) in templates {
        write_new_file(&path, body)?;
    }
    Ok(())
}

fn template_outputs(out: &Path, lang: crate::i18n::Lang) -> Vec<(PathBuf, String)> {
    crate::config::template::registry()
        .iter()
        .map(|template| {
            (
                out.join(template.path),
                crate::config::template::render_with_lang(template, lang),
            )
        })
        .chain(
            crate::config::template::theme_presets()
                .iter()
                .map(|preset| {
                    (
                        out.join(preset.path),
                        crate::config::template::render_theme_preset(preset),
                    )
                }),
        )
        .collect()
}

fn write_new_file(path: &Path, body: String) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).with_context(|| format!("create {}", parent.display()))?;
    }
    std::fs::write(path, body).with_context(|| format!("write {}", path.display()))
}

#[cfg(test)]
mod tests {
    use clap::Parser;

    use super::*;

    fn temp_dir() -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("shuohua-template-export-{}", ulid::Ulid::new()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn writes_templates_from_registry_without_assets() {
        let dir = temp_dir();

        write_templates(&dir, crate::i18n::Lang::EnUS).unwrap();

        assert!(dir.join("config.toml").exists());
        assert!(dir.join("profile/default.toml").exists());
        assert!(dir.join("asr/apple.toml").exists());
        assert!(dir.join("asr/doubao.toml").exists());
        assert!(dir.join("post/rule/zh_filter.toml").exists());
        assert!(dir.join("post/llm/openai.toml").exists());
        assert!(dir.join("theme/gruvbox-dark.toml").exists());
        assert!(dir.join("theme/catppuccin-latte.toml").exists());
        assert!(!dir.join("theme/light.toml").exists());
        assert!(!dir.join("theme/default.toml").exists());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn refuses_to_overwrite_existing_templates() {
        let dir = temp_dir();
        std::fs::write(dir.join("config.toml"), "existing").unwrap();

        let error = write_templates(&dir, crate::i18n::Lang::EnUS)
            .unwrap_err()
            .to_string();

        assert!(error.contains("refusing to overwrite"), "{error}");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn refuses_all_conflicts_before_writing_any_template() {
        let dir = temp_dir();
        std::fs::write(dir.join("config.toml"), "existing").unwrap();
        std::fs::create_dir_all(dir.join("asr")).unwrap();
        std::fs::write(dir.join("asr/apple.toml"), "existing").unwrap();

        let error = write_templates(&dir, crate::i18n::Lang::EnUS)
            .unwrap_err()
            .to_string();

        assert!(error.contains("config.toml"), "{error}");
        assert!(error.contains("asr/apple.toml"), "{error}");
        assert!(!dir.join("profile/default.toml").exists());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn parses_template_language_arg() {
        let cli = crate::cli::Cli::try_parse_from([
            "shuo",
            "config-template",
            "--out",
            "/tmp/shuohua",
            "--lang",
            "zh-CN",
        ])
        .unwrap();

        match cli.command {
            Some(crate::cli::Command::ConfigTemplate(args)) => {
                assert_eq!(args.lang, "zh-CN");
            }
            other => panic!("expected config-template command, got {other:?}"),
        }
    }
}
