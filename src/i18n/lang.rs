#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Lang {
    ZhCN,
    ZhHant,
    ZhTW,
    ZhHK,
    EnUS,
    Pseudo,
}

pub(crate) fn resolve_lang_with_env(cfg_lang: &str, env_lang: Option<&str>) -> Lang {
    let cfg = cfg_lang.trim();
    if normalized_tag(cfg) == "auto" {
        return env_lang.map(resolve_tag).unwrap_or(Lang::EnUS);
    }
    resolve_tag(cfg)
}

fn resolve_tag(tag: &str) -> Lang {
    let tag = normalized_tag(tag);
    if tag == "pseudo" {
        return Lang::Pseudo;
    }
    if tag == "zh" || tag.starts_with("zh-") {
        return if tag.contains("-tw") {
            Lang::ZhTW
        } else if tag.contains("-hk") || tag.contains("-mo") {
            Lang::ZhHK
        } else if tag.contains("hant") {
            Lang::ZhHant
        } else {
            Lang::ZhCN
        };
    }
    Lang::EnUS
}

fn normalized_tag(tag: &str) -> String {
    let tag = tag
        .split_once('.')
        .map(|(lang, _)| lang)
        .unwrap_or(tag)
        .split_once('@')
        .map(|(lang, _)| lang)
        .unwrap_or(tag);
    tag.trim().replace('_', "-").to_ascii_lowercase()
}
