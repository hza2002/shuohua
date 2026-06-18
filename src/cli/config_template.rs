use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use clap::Args;

#[derive(Debug, Args)]
pub struct ConfigTemplateArgs {
    /// Directory to write generated templates into.
    #[arg(long)]
    pub out: Option<PathBuf>,
    /// Comment language for generated templates: auto, en-US, or zh-CN.
    #[arg(long, default_value = "auto")]
    pub lang: String,
}

pub fn run(args: ConfigTemplateArgs) -> Result<()> {
    let out = args.out.unwrap_or_else(default_template_dir);
    let lang = template_lang(&args.lang);
    write_templates(&out, lang)?;
    println!("config templates written to {}", out.display());
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
    for template in crate::config::template::registry() {
        write_new_file(
            &out.join(template.path),
            crate::config::template::render_with_lang(template, lang),
        )?;
    }
    Ok(())
}

fn write_new_file(path: &Path, body: String) -> Result<()> {
    if path.exists() {
        anyhow::bail!("refusing to overwrite {}", path.display());
    }
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
