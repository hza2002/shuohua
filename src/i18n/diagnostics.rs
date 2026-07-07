use std::collections::BTreeSet;

use super::catalog::{self, Dict};
use super::format;
use super::lang::Lang;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Diagnostic {
    MissingKey {
        lang: Lang,
        key: String,
    },
    PlaceholderMismatch {
        lang: Lang,
        key: String,
        expected: BTreeSet<String>,
        actual: BTreeSet<String>,
    },
    EmptyValue {
        lang: Lang,
        key: String,
    },
}

pub fn diagnose_embedded() -> Vec<Diagnostic> {
    let en = catalog::load_base_dict(Lang::EnUS);
    let zh = catalog::load_base_dict(Lang::ZhCN);
    let zh_hant = catalog::load_dict(Lang::ZhHant);
    let zh_tw = catalog::load_dict(Lang::ZhTW);
    let zh_hk = catalog::load_dict(Lang::ZhHK);
    let pseudo = catalog::load_dict(Lang::Pseudo);

    let mut diagnostics = Vec::new();
    diagnostics.extend(diagnose_pair(Lang::EnUS, &en, Lang::ZhCN, &zh));
    diagnostics.extend(diagnose_pair(Lang::ZhCN, &zh, Lang::ZhHant, &zh_hant));
    diagnostics.extend(diagnose_pair(Lang::ZhCN, &zh, Lang::ZhTW, &zh_tw));
    diagnostics.extend(diagnose_pair(Lang::ZhCN, &zh, Lang::ZhHK, &zh_hk));
    diagnostics.extend(diagnose_pair(Lang::EnUS, &en, Lang::Pseudo, &pseudo));
    diagnostics
}

fn diagnose_pair(base_lang: Lang, base: &Dict, other_lang: Lang, other: &Dict) -> Vec<Diagnostic> {
    let mut diagnostics = Vec::new();
    let base_keys = base.entries().keys().collect::<BTreeSet<_>>();
    let other_keys = other.entries().keys().collect::<BTreeSet<_>>();

    for key in base_keys.difference(&other_keys) {
        diagnostics.push(Diagnostic::MissingKey {
            lang: other_lang,
            key: (*key).clone(),
        });
    }
    for key in other_keys.difference(&base_keys) {
        diagnostics.push(Diagnostic::MissingKey {
            lang: base_lang,
            key: (*key).clone(),
        });
    }

    for (key, base_value) in base.entries() {
        if base_value.is_empty() && !is_allowed_empty(key) {
            diagnostics.push(Diagnostic::EmptyValue {
                lang: base_lang,
                key: key.clone(),
            });
        }
        let Some(other_value) = other.entries().get(key) else {
            continue;
        };
        if other_value.is_empty() && !is_allowed_empty(key) {
            diagnostics.push(Diagnostic::EmptyValue {
                lang: other_lang,
                key: key.clone(),
            });
        }
        let expected = format::extract_placeholders(base_value);
        let actual = format::extract_placeholders(other_value);
        if expected != actual {
            diagnostics.push(Diagnostic::PlaceholderMismatch {
                lang: other_lang,
                key: key.clone(),
                expected,
                actual,
            });
        }
    }

    diagnostics
}

fn is_allowed_empty(_key: &str) -> bool {
    // No i18n value is intentionally empty.
    false
}
