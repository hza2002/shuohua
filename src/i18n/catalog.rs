use std::collections::HashMap;

use super::format;
use super::lang::Lang;

#[derive(Debug, Clone)]
pub(crate) struct Dict {
    entries: HashMap<String, String>,
}

impl Dict {
    pub(crate) fn get(&self, key: &str) -> Option<&str> {
        self.entries.get(key).map(String::as_str)
    }

    pub(crate) fn entries(&self) -> &HashMap<String, String> {
        &self.entries
    }
}

pub(crate) fn load_dict(lang: Lang) -> Dict {
    match lang {
        Lang::ZhCN => load_base_dict(Lang::ZhCN),
        Lang::ZhHant => generated_dict(include!(concat!(env!("OUT_DIR"), "/i18n_zh_hant.rs"))),
        Lang::ZhTW => generated_dict(include!(concat!(env!("OUT_DIR"), "/i18n_zh_tw.rs"))),
        Lang::ZhHK => generated_dict(include!(concat!(env!("OUT_DIR"), "/i18n_zh_hk.rs"))),
        Lang::EnUS => load_base_dict(Lang::EnUS),
        Lang::Pseudo => pseudo_dict(load_base_dict(Lang::EnUS)),
    }
}

pub(crate) fn load_base_dict(lang: Lang) -> Dict {
    let body = match lang {
        Lang::ZhCN => include_str!("../../assets/i18n/zh-CN.toml"),
        Lang::EnUS => include_str!("../../assets/i18n/en-US.toml"),
        Lang::ZhHant | Lang::ZhTW | Lang::ZhHK | Lang::Pseudo => {
            panic!("derived locale {lang:?} does not have an embedded TOML asset")
        }
    };
    Dict {
        entries: flatten_toml(body),
    }
}

fn generated_dict(entries: &[(&str, &str)]) -> Dict {
    Dict {
        entries: entries
            .iter()
            .map(|(key, value)| ((*key).to_string(), (*value).to_string()))
            .collect(),
    }
}

fn pseudo_dict(dict: Dict) -> Dict {
    Dict {
        entries: dict
            .entries
            .into_iter()
            .map(|(key, value)| (key, format::pseudo_text(&value)))
            .collect(),
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
