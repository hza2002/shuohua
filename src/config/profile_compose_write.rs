use std::path::Path;

use anyhow::{Context, Result};
use toml_edit::{value, Array, Item, Table};

use crate::config::field_write::{atomic_write, read_document};
use crate::config::schema::{self, SchemaId};
use crate::config::spec::{validate_value, Severity};

/// Which override table a field belongs to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OverrideSection {
    /// `[asr]` flattened provider overrides.
    Asr,
    /// `[post.overrides.<member_id>]`.
    Overrides(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MoveDir {
    Up,
    Down,
}

fn write_validated(path: &Path, doc: &toml_edit::DocumentMut) -> Result<()> {
    let rendered = doc.to_string();
    let parsed: toml::Value = toml::from_str(&rendered).context("re-parse profile after edit")?;
    let errors: Vec<_> = validate_value(&schema::spec_for(SchemaId::Profile), &parsed)
        .into_iter()
        .filter(|d| d.severity == Severity::Error)
        .collect();
    anyhow::ensure!(errors.is_empty(), "profile invalid after edit: {errors:?}");
    atomic_write(path, &rendered)
}

/// Navigate to the toml_edit table for a section, creating parents as needed.
fn section_table<'a>(
    doc: &'a mut toml_edit::DocumentMut,
    section: &OverrideSection,
) -> &'a mut Table {
    match section {
        OverrideSection::Asr => ensure_table(doc.as_table_mut(), "asr"),
        OverrideSection::Overrides(id) => {
            let post = ensure_table(doc.as_table_mut(), "post");
            let overrides = ensure_table(post, "overrides");
            ensure_table(overrides, id)
        }
    }
}

fn ensure_table<'a>(parent: &'a mut Table, key: &str) -> &'a mut Table {
    // Promote a user-authored inline table (`key = { ... }`) to a regular table
    // so we edit it in place instead of clobbering its existing keys.
    if parent.get(key).is_some_and(Item::is_inline_table) {
        if let Some(inline) = parent[key].as_inline_table().cloned() {
            parent[key] = Item::Table(inline.into_table());
        }
    }
    if !parent.contains_key(key) || !parent[key].is_table() {
        parent[key] = Item::Table(Table::new());
    }
    parent[key].as_table_mut().expect("created table")
}

pub fn set_override(path: &Path, section: &OverrideSection, field: &str, v: Item) -> Result<()> {
    let mut doc = read_document(path)?;
    section_table(&mut doc, section)[field] = v;
    write_validated(path, &doc)
}

pub fn unset_override(path: &Path, section: &OverrideSection, field: &str) -> Result<()> {
    let mut doc = read_document(path)?;
    section_table(&mut doc, section).remove(field);
    // If an [post.overrides.<id>] table is now empty, drop it entirely.
    if let OverrideSection::Overrides(id) = section {
        if let Some(post) = doc.get_mut("post").and_then(Item::as_table_mut) {
            if let Some(ov) = post.get_mut("overrides").and_then(Item::as_table_mut) {
                if ov
                    .get(id)
                    .and_then(Item::as_table)
                    .is_some_and(Table::is_empty)
                {
                    ov.remove(id);
                }
                if ov.is_empty() {
                    post.remove("overrides");
                }
            }
        }
    }
    write_validated(path, &doc)
}

fn chain_array(doc: &mut toml_edit::DocumentMut) -> Result<&mut Array> {
    let post = ensure_table(doc.as_table_mut(), "post");
    if post.get("chain").and_then(Item::as_array).is_none() {
        post["chain"] = value(Array::new());
    }
    post["chain"]
        .as_array_mut()
        .context("[post].chain is not an array")
}

pub fn add_chain_member(path: &Path, id: &str) -> Result<()> {
    let mut doc = read_document(path)?;
    chain_array(&mut doc)?.push(id);
    write_validated(path, &doc)
}

pub fn remove_chain_member(path: &Path, id: &str) -> Result<()> {
    let mut doc = read_document(path)?;
    let arr = chain_array(&mut doc)?;
    // remove first matching occurrence
    let idx = arr.iter().position(|x| x.as_str() == Some(id));
    if let Some(idx) = idx {
        arr.remove(idx);
    }
    // drop its override table if present
    let _ = unset_override_table(&mut doc, id);
    write_validated(path, &doc)
}

fn unset_override_table(doc: &mut toml_edit::DocumentMut, id: &str) -> Option<()> {
    let post = doc.get_mut("post")?.as_table_mut()?;
    let ov = post.get_mut("overrides")?.as_table_mut()?;
    ov.remove(id);
    if ov.is_empty() {
        post.remove("overrides");
    }
    Some(())
}

pub fn move_chain_member(path: &Path, idx: usize, dir: MoveDir) -> Result<()> {
    let mut doc = read_document(path)?;
    let arr = chain_array(&mut doc)?;
    let target = match dir {
        MoveDir::Up if idx > 0 => idx - 1,
        MoveDir::Down if idx + 1 < arr.len() => idx + 1,
        _ => return Ok(()),
    };
    // toml_edit Array has no swap; rebuild from the string values. Error rather
    // than silently drop a non-string element (which would truncate the chain).
    let mut items: Vec<String> = arr
        .iter()
        .map(|x| {
            x.as_str()
                .map(str::to_string)
                .context("[post].chain contains a non-string element")
        })
        .collect::<Result<Vec<_>>>()?;
    items.swap(idx, target);
    let mut rebuilt = Array::new();
    for it in items {
        rebuilt.push(it);
    }
    *arr = rebuilt;
    write_validated(path, &doc)
}

pub fn drop_invalid_overrides(path: &Path, invalid: &[(OverrideSection, String)]) -> Result<()> {
    let mut doc = read_document(path)?;
    for (section, field) in invalid {
        section_table(&mut doc, section).remove(field);
    }
    // collapse now-empty override tables
    if let Some(post) = doc.get_mut("post").and_then(Item::as_table_mut) {
        if let Some(ov) = post.get_mut("overrides").and_then(Item::as_table_mut) {
            let empties: Vec<String> = ov
                .iter()
                .filter(|(_, v)| v.as_table().is_some_and(Table::is_empty))
                .map(|(k, _)| k.to_string())
                .collect();
            for k in empties {
                ov.remove(&k);
            }
            if ov.is_empty() {
                post.remove("overrides");
            }
        }
    }
    write_validated(path, &doc)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn temp_profile(body: &str) -> std::path::PathBuf {
        let path = std::env::temp_dir().join(format!("shuohua-compose-{}.toml", ulid::Ulid::new()));
        fs::write(&path, body).unwrap();
        path
    }

    const BASE: &str = "name = \"Default\"\n[asr]\ninstance = \"doubao\"\n[post]\nchain = [\"zh_filter\", \"deepseek\"]\n";

    #[test]
    fn add_and_remove_chain_member_roundtrips() {
        let p = temp_profile(BASE);
        add_chain_member(&p, "openai").unwrap();
        let doc = fs::read_to_string(&p).unwrap();
        assert!(doc.contains("\"openai\""), "{doc}");
        // removing drops the id and any of its overrides
        set_override(
            &p,
            &OverrideSection::Overrides("openai".into()),
            "model",
            value("x"),
        )
        .unwrap();
        remove_chain_member(&p, "openai").unwrap();
        let doc = fs::read_to_string(&p).unwrap();
        assert!(
            !doc.contains("openai"),
            "member and its overrides gone: {doc}"
        );
        // still parses as a Profile
        crate::config::profile::parse(&doc).unwrap();
        let _ = fs::remove_file(p);
    }

    #[test]
    fn set_and_unset_override_targets_correct_table() {
        let p = temp_profile(BASE);
        set_override(
            &p,
            &OverrideSection::Overrides("deepseek".into()),
            "model",
            value("m2"),
        )
        .unwrap();
        set_override(&p, &OverrideSection::Asr, "language", value("zh-CN")).unwrap();
        let doc = fs::read_to_string(&p).unwrap();
        assert!(
            doc.contains("[post.overrides.deepseek]") && doc.contains("m2"),
            "{doc}"
        );
        assert!(doc.contains("language = \"zh-CN\""), "{doc}");
        unset_override(&p, &OverrideSection::Overrides("deepseek".into()), "model").unwrap();
        let doc = fs::read_to_string(&p).unwrap();
        assert!(!doc.contains("m2"), "{doc}");
        let _ = fs::remove_file(p);
    }

    #[test]
    fn move_chain_member_swaps_order() {
        let p = temp_profile(BASE);
        move_chain_member(&p, 1, MoveDir::Up).unwrap();
        let prof = crate::config::profile::parse(&fs::read_to_string(&p).unwrap()).unwrap();
        assert_eq!(prof.post.chain, vec!["deepseek", "zh_filter"]);
        let _ = fs::remove_file(p);
    }

    #[test]
    fn set_override_preserves_inline_asr_table() {
        // A hand-authored `asr = { instance = "doubao" }` inline table must not
        // be clobbered when we add a flattened override key.
        let p = temp_profile("name=\"D\"\nasr = { instance = \"doubao\" }\n[post]\nchain=[]\n");
        set_override(&p, &OverrideSection::Asr, "language", value("zh-CN")).unwrap();
        let prof = crate::config::profile::parse(&fs::read_to_string(&p).unwrap()).unwrap();
        assert_eq!(prof.asr.instance, "doubao");
        assert!(fs::read_to_string(&p).unwrap().contains("zh-CN"));
        let _ = fs::remove_file(p);
    }

    #[test]
    fn drop_invalid_overrides_removes_named_keys() {
        let p = temp_profile("name=\"D\"\n[asr]\ninstance=\"apple\"\napp_key=\"x\"\nlanguage=\"zh\"\n[post]\nchain=[]\n");
        drop_invalid_overrides(&p, &[(OverrideSection::Asr, "app_key".into())]).unwrap();
        let doc = fs::read_to_string(&p).unwrap();
        assert!(
            !doc.contains("app_key") && doc.contains("language"),
            "{doc}"
        );
        let _ = fs::remove_file(p);
    }
}
