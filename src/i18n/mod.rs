use std::collections::HashMap;
use std::sync::{Arc, OnceLock, RwLock};

static DICT: OnceLock<RwLock<Arc<Dict>>> = OnceLock::new();

#[derive(Debug)]
pub struct Dict {
    pub lang: Lang,
    entries: HashMap<String, String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lang {
    ZhCN,
    EnUS,
}

pub fn init(cfg_lang: &str) {
    set_dict(resolve_lang(cfg_lang));
}

pub fn resolve_lang(cfg_lang: &str) -> Lang {
    let env_lang = std::env::var("LANG").ok();
    resolve_lang_with_env(cfg_lang, env_lang.as_deref())
}

fn resolve_lang_with_env(cfg_lang: &str, env_lang: Option<&str>) -> Lang {
    match cfg_lang {
        "zh-CN" => Lang::ZhCN,
        "en-US" => Lang::EnUS,
        "auto" => {
            if env_lang
                .unwrap_or_default()
                .to_ascii_lowercase()
                .starts_with("zh")
            {
                Lang::ZhCN
            } else {
                Lang::EnUS
            }
        }
        _ => Lang::EnUS,
    }
}

pub fn tr(key: &str, vars: &[(&str, String)]) -> String {
    let dict = DICT
        .get()
        .map(|lock| lock.read().expect("i18n dict lock poisoned").clone())
        .unwrap_or_else(|| Arc::new(load_dict(Lang::EnUS)));
    let template = dict
        .entries
        .get(key)
        .cloned()
        .unwrap_or_else(|| key.to_string());
    vars.iter().fold(template, |acc, (name, value)| {
        acc.replace(&format!("{{{name}}}"), value)
    })
}

#[macro_export]
macro_rules! t {
    ($key:expr) => {
        $crate::i18n::tr($key, &[])
    };
    ($key:expr, $($name:ident = $value:expr),+ $(,)?) => {
        $crate::i18n::tr($key, &[$((stringify!($name), $value.to_string())),+])
    };
}

fn set_dict(lang: Lang) {
    let dict = Arc::new(load_dict(lang));
    let lock = DICT.get_or_init(|| RwLock::new(dict.clone()));
    *lock.write().expect("i18n dict lock poisoned") = dict;
}

fn load_dict(lang: Lang) -> Dict {
    let body = match lang {
        Lang::ZhCN => include_str!("../../assets/i18n/zh-CN.toml"),
        Lang::EnUS => include_str!("../../assets/i18n/en-US.toml"),
    };
    Dict {
        lang,
        entries: flatten_toml(body),
    }
}

fn flatten_toml(body: &str) -> HashMap<String, String> {
    let value = body
        .parse::<toml::Value>()
        .expect("embedded i18n TOML must parse");
    let mut out = HashMap::new();
    flatten_value(None, &value, &mut out);
    out
}

fn flatten_value(prefix: Option<&str>, value: &toml::Value, out: &mut HashMap<String, String>) {
    let Some(table) = value.as_table() else {
        return;
    };
    for (key, value) in table {
        let full_key = match prefix {
            Some(prefix) => format!("{prefix}.{key}"),
            None => key.to_string(),
        };
        if let Some(text) = value.as_str() {
            out.insert(full_key, text.to_string());
        } else {
            flatten_value(Some(&full_key), value, out);
        }
    }
}

#[cfg(test)]
fn init_for_tests(lang: Lang) {
    set_dict(lang);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_auto_language_from_lang_env() {
        assert_eq!(resolve_lang("zh-CN"), Lang::ZhCN);
        assert_eq!(resolve_lang("en-US"), Lang::EnUS);
        assert_eq!(
            resolve_lang_with_env("auto", Some("zh_CN.UTF-8")),
            Lang::ZhCN
        );
        assert_eq!(resolve_lang_with_env("auto", Some("C")), Lang::EnUS);
    }

    #[test]
    fn translates_keys_and_replaces_vars() {
        init_for_tests(Lang::EnUS);
        assert_eq!(tr("overlay.state_recording", &[]), "Recording");
        assert_eq!(
            tr("notice.step_failed", &[("name", "filler".to_string())]),
            "filler failed, skipped"
        );
    }
}
