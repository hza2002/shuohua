#[cfg(test)]
use std::cell::RefCell;
use std::sync::Arc;
#[cfg(not(test))]
use std::sync::{OnceLock, RwLock};

mod catalog;
pub mod diagnostics;
mod format;
mod lang;

pub use lang::Lang;

use catalog::Dict;

#[cfg(not(test))]
static DICT: OnceLock<RwLock<Arc<Dict>>> = OnceLock::new();
#[cfg(test)]
thread_local! {
    static TEST_DICT: RefCell<Arc<Dict>> = RefCell::new(Arc::new(catalog::load_dict(Lang::EnUS)));
}

pub fn init(cfg_lang: &str) {
    set_dict(resolve_lang(cfg_lang));
}

pub fn resolve_lang(cfg_lang: &str) -> Lang {
    let env_lang = std::env::var("LANG").ok();
    lang::resolve_lang_with_env(cfg_lang, env_lang.as_deref())
}

pub fn tr(key: &str, vars: &[(&str, String)]) -> String {
    #[cfg(test)]
    let dict = TEST_DICT.with(|dict| dict.borrow().clone());
    #[cfg(not(test))]
    let dict = DICT
        .get()
        .map(|lock| lock.read().expect("i18n dict lock poisoned").clone())
        .unwrap_or_else(|| Arc::new(catalog::load_dict(Lang::EnUS)));
    tr_from_dict(&dict, key, vars)
}

pub fn tr_lang(lang: Lang, key: &str, vars: &[(&str, String)]) -> String {
    let dict = catalog::load_dict(lang);
    tr_from_dict(&dict, key, vars)
}

fn tr_from_dict(dict: &Dict, key: &str, vars: &[(&str, String)]) -> String {
    let template = dict.get(key).unwrap_or(key);
    format::replace_placeholders(template, vars)
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
    let dict = Arc::new(catalog::load_dict(lang));
    #[cfg(test)]
    TEST_DICT.with(|current| *current.borrow_mut() = dict);
    #[cfg(not(test))]
    {
        let lock = DICT.get_or_init(|| RwLock::new(dict.clone()));
        *lock.write().expect("i18n dict lock poisoned") = dict;
    }
}

#[cfg(test)]
fn resolve_lang_with_env(cfg_lang: &str, env_lang: Option<&str>) -> Lang {
    lang::resolve_lang_with_env(cfg_lang, env_lang)
}

#[cfg(test)]
fn load_dict(lang: Lang) -> Dict {
    catalog::load_dict(lang)
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
    fn normalizes_common_language_tags() {
        assert_eq!(resolve_lang_with_env("zh_CN.UTF-8", None), Lang::ZhCN);
        assert_eq!(resolve_lang_with_env("zh-Hans-CN", None), Lang::ZhCN);
        assert_eq!(resolve_lang_with_env("zh-Hant", None), Lang::ZhHant);
        assert_eq!(resolve_lang_with_env("zh_TW.UTF-8", None), Lang::ZhTW);
        assert_eq!(resolve_lang_with_env("zh-HK", None), Lang::ZhHK);
        assert_eq!(resolve_lang_with_env("zh_MO.UTF-8", None), Lang::ZhHK);
        assert_eq!(resolve_lang_with_env("en", None), Lang::EnUS);
        assert_eq!(resolve_lang_with_env("en_US.UTF-8", None), Lang::EnUS);
        assert_eq!(
            resolve_lang_with_env("auto", Some("zh_Hant.UTF-8")),
            Lang::ZhHant
        );
        assert_eq!(
            resolve_lang_with_env("auto", Some("zh_TW.UTF-8")),
            Lang::ZhTW
        );
        assert_eq!(
            resolve_lang_with_env("auto", Some("zh_HK.UTF-8")),
            Lang::ZhHK
        );
        assert_eq!(resolve_lang_with_env("pseudo", None), Lang::Pseudo);
        assert_eq!(resolve_lang_with_env("fr-FR", None), Lang::EnUS);
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

    #[test]
    fn missing_key_falls_back_to_key() {
        init_for_tests(Lang::EnUS);
        assert_eq!(tr("missing.i18n.key", &[]), "missing.i18n.key");
    }

    #[test]
    fn extracts_placeholders() {
        assert_eq!(
            format::extract_placeholders("Open {path}: {error} ({path})"),
            ["error".to_string(), "path".to_string()].into()
        );
        assert!(format::extract_placeholders("literal {{path}} and {not-closed").is_empty());
    }

    #[test]
    fn replaces_placeholders() {
        assert_eq!(
            format::replace_placeholders(
                "Open {path}: {error}",
                &[
                    ("path", "/tmp/a".to_string()),
                    ("error", "denied".to_string())
                ]
            ),
            "Open /tmp/a: denied"
        );
    }

    #[test]
    fn pseudo_locale_expands_text_and_preserves_placeholders() {
        let pseudo = format::pseudo_text("Open {path}: {error}");

        assert!(pseudo.len() > "Open {path}: {error}".len());
        assert!(pseudo.contains("{path}"));
        assert!(pseudo.contains("{error}"));
        assert!(!format::replace_placeholders(
            &pseudo,
            &[
                ("path", "/tmp/a".to_string()),
                ("error", "denied".to_string())
            ]
        )
        .contains("{path}"));
    }

    #[test]
    fn pseudo_locale_is_available_for_translation() {
        assert_ne!(
            tr_lang(
                Lang::Pseudo,
                "notice.step_failed",
                &[("name", "filler".to_string())]
            ),
            tr_lang(
                Lang::EnUS,
                "notice.step_failed",
                &[("name", "filler".to_string())]
            )
        );
        assert!(tr_lang(
            Lang::Pseudo,
            "notice.step_failed",
            &[("name", "filler".to_string())]
        )
        .contains("filler"));
    }

    #[test]
    fn generated_chinese_variants_translate_from_zh_cn_source() {
        assert_eq!(tr_lang(Lang::ZhHant, "tui.tab_history", &[]), "2 歷史");
        assert_eq!(tr_lang(Lang::ZhTW, "tui.tab_history", &[]), "2 歷史");
        assert_eq!(tr_lang(Lang::ZhHK, "tui.tab_history", &[]), "2 歷史");
        assert_ne!(
            tr_lang(Lang::ZhTW, "notice.history_save_failed", &[]),
            tr_lang(Lang::ZhCN, "notice.history_save_failed", &[])
        );
    }

    #[test]
    fn generated_chinese_variants_preserve_placeholders() {
        let tw = tr_lang(
            Lang::ZhTW,
            "tui.configure.opening",
            &[("path", "/tmp/a".to_string())],
        );
        let hk = tr_lang(
            Lang::ZhHK,
            "tui.configure.opening",
            &[("path", "/tmp/a".to_string())],
        );

        assert!(tw.contains("編輯器"), "{tw}");
        assert!(tw.contains("/tmp/a"), "{tw}");
        assert!(!tw.contains("{path}"), "{tw}");
        assert!(hk.contains("/tmp/a"), "{hk}");
        assert!(!hk.contains("{path}"), "{hk}");
    }

    #[test]
    fn zh_cn_and_en_us_keys_match() {
        let zh = load_dict(Lang::ZhCN);
        let en = load_dict(Lang::EnUS);

        let mut zh_only = zh
            .entries()
            .keys()
            .filter(|key| !en.entries().contains_key(*key))
            .cloned()
            .collect::<Vec<_>>();
        let mut en_only = en
            .entries()
            .keys()
            .filter(|key| !zh.entries().contains_key(*key))
            .cloned()
            .collect::<Vec<_>>();
        zh_only.sort();
        en_only.sort();

        assert!(zh_only.is_empty(), "zh-CN only keys: {zh_only:?}");
        assert!(en_only.is_empty(), "en-US only keys: {en_only:?}");
    }

    #[test]
    fn embedded_locale_diagnostics_are_clean() {
        let diagnostics = diagnostics::diagnose_embedded();

        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }
}
