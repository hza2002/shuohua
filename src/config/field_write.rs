use std::path::Path;

use anyhow::Context;
use toml_edit::{value, Array, DocumentMut, Item, Table, Value};

use crate::config::spec::{validate_value, ConfigSpec, Diagnostic, Severity};

#[derive(Debug, Clone, PartialEq)]
pub enum TypedInput {
    Bool(bool),
    Integer(i64),
    Float(f64),
    Str(String),
    StrArray(Vec<String>),
}

#[derive(Debug)]
#[allow(dead_code)]
pub enum WriteError {
    Validation(Vec<Diagnostic>),
    Semantic(String),
    Io(anyhow::Error),
}

pub fn set_field(
    path: &Path,
    field_path: &str,
    input: TypedInput,
    spec: &ConfigSpec,
) -> Result<(), WriteError> {
    let mut doc = read_document(path).map_err(WriteError::Io)?;
    apply_field(&mut doc, field_path, input, spec)?;
    atomic_write(path, &doc.to_string()).map_err(WriteError::Io)
}

/// 在内存 `DocumentMut` 上写入并校验一个字段（结构 + Error 级 schema 校验 +
/// 语义校验），不碰磁盘。File 写盘路径与内存 draft 编辑共用同一套校验，保证
/// 新建（未落盘的 draft）与编辑（已落盘文件）行为一致。
pub fn apply_field(
    doc: &mut DocumentMut,
    field_path: &str,
    input: TypedInput,
    spec: &ConfigSpec,
) -> Result<(), WriteError> {
    let item = typed_to_item(&input);
    set_dotted(doc, field_path, item);

    let rendered = doc.to_string();
    let parsed: toml::Value = toml::from_str(&rendered)
        .map_err(|e| WriteError::Io(anyhow::anyhow!("re-parse after edit: {e}")))?;
    let errors = blocking_errors(spec, &parsed);
    if !errors.is_empty() {
        return Err(WriteError::Validation(errors));
    }

    if let TypedInput::Str(raw) = &input {
        if let Err(reason) = semantic_validate(field_path, raw) {
            return Err(WriteError::Semantic(reason));
        }
    }
    Ok(())
}

/// 阻断保存的错误：结构性 Error（类型/范围/未知字段/枚举），但**排除 secret 字段的
/// 「未填」类错误**——填密钥是 readiness，不该挡住逐字段保存半成品。这样新建 draft 与
/// 编辑已落盘文件用同一套规则，都能渐进填写；加载/运行仍由 `reject_schema_diagnostics`
/// （连 Warning 一起 bail）拦住空密钥，doctor 也照常按引用关系升级严重度。
pub fn blocking_errors(spec: &ConfigSpec, parsed: &toml::Value) -> Vec<Diagnostic> {
    validate_value(spec, parsed)
        .into_iter()
        .filter(|d| d.severity == Severity::Error)
        .filter(|d| {
            !spec
                .field_for_path(&d.path)
                .is_some_and(|field| field.is_secret())
        })
        .collect()
}

pub(crate) fn read_document(path: &Path) -> anyhow::Result<DocumentMut> {
    match std::fs::read_to_string(path) {
        Ok(body) => body
            .parse::<DocumentMut>()
            .with_context(|| format!("parse {}", path.display())),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(DocumentMut::new()),
        Err(e) => Err(anyhow::Error::new(e)).with_context(|| format!("read {}", path.display())),
    }
}

pub(crate) fn typed_to_item(input: &TypedInput) -> Item {
    match input {
        TypedInput::Bool(b) => value(*b),
        TypedInput::Integer(i) => value(*i),
        TypedInput::Float(f) => value(*f),
        TypedInput::Str(s) => value(s.as_str()),
        TypedInput::StrArray(items) => {
            let mut array = Array::default();
            for item in items {
                array.push(item.as_str());
            }
            Item::Value(Value::Array(array))
        }
    }
}

/// 沿点路径下钻，缺失中间表用普通 Table 创建，最后写入叶子。
fn set_dotted(doc: &mut DocumentMut, field_path: &str, leaf: Item) {
    if field_path.is_empty() {
        return;
    }
    let parts: Vec<&str> = field_path.split('.').collect();
    let (last, parents) = parts
        .split_last()
        .expect("split yields at least one element");
    let mut table = doc.as_table_mut();
    for part in parents {
        if !table.contains_key(part) || !table[part].is_table() {
            table[part] = Item::Table(Table::new());
        }
        table = table[part].as_table_mut().expect("created table");
    }
    table[last] = leaf;
}

pub fn unset_field(path: &Path, field_path: &str, spec: &ConfigSpec) -> Result<(), WriteError> {
    let mut doc = read_document(path).map_err(WriteError::Io)?;
    remove_dotted(&mut doc, field_path);

    let rendered = doc.to_string();
    let parsed: toml::Value = toml::from_str(&rendered)
        .map_err(|e| WriteError::Io(anyhow::anyhow!("re-parse after unset: {e}")))?;
    let errors: Vec<Diagnostic> = validate_value(spec, &parsed)
        .into_iter()
        .filter(|d| d.severity == Severity::Error)
        .collect();
    if !errors.is_empty() {
        return Err(WriteError::Validation(errors));
    }

    atomic_write(path, &rendered).map_err(WriteError::Io)
}

fn remove_dotted(doc: &mut DocumentMut, field_path: &str) {
    if field_path.is_empty() {
        return;
    }
    let parts: Vec<&str> = field_path.split('.').collect();
    let (last, parents) = parts
        .split_last()
        .expect("split yields at least one element");
    let mut table = doc.as_table_mut();
    for part in parents {
        match table.get_mut(part).and_then(Item::as_table_mut) {
            Some(child) => table = child,
            None => return,
        }
    }
    table.remove(last);
}

pub(crate) fn atomic_write(path: &Path, body: &str) -> anyhow::Result<()> {
    let tmp = path.with_extension(format!("toml.tmp-{}", ulid::Ulid::generate()));
    std::fs::write(&tmp, body).with_context(|| format!("write temp {}", tmp.display()))?;
    std::fs::rename(&tmp, path).with_context(|| format!("replace {}", path.display()))?;
    Ok(())
}

/// 结构校验之后的语义校验：field_path -> 校验规则。
fn semantic_validate(field_path: &str, raw: &str) -> Result<(), String> {
    match field_path {
        "hotkey.trigger" | "hotkey.cancel" | "hotkey.resume" => crate::hotkey::parse::parse(raw)
            .map(|_| ())
            .map_err(|e| e.to_string()),
        _ => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::schema::{spec_for, SchemaId};

    fn temp_file(body: &str) -> std::path::PathBuf {
        let dir =
            std::env::temp_dir().join(format!("shuohua-fieldwrite-{}", ulid::Ulid::generate()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("config.toml");
        std::fs::write(&path, body).unwrap();
        path
    }

    #[test]
    fn set_nested_field_creates_intermediate_tables_and_preserves_comments() {
        let path = temp_file("# keep me\n[hotkey]\ntrigger = \"f16\"\n");
        let spec = spec_for(SchemaId::Main);

        set_field(&path, "voice.vad.threshold", TypedInput::Float(0.4), &spec).unwrap();

        let body = std::fs::read_to_string(&path).unwrap();
        assert!(body.contains("# keep me"), "comment preserved: {body}");
        assert!(body.contains("threshold = 0.4"), "value written: {body}");
        crate::config::main::parse(&body).unwrap();
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn set_field_rejects_out_of_range_without_writing() {
        let path = temp_file("[hotkey]\ntrigger = \"f16\"\n");
        let spec = spec_for(SchemaId::Main);
        let before = std::fs::read_to_string(&path).unwrap();

        let err =
            set_field(&path, "voice.vad.threshold", TypedInput::Float(1.5), &spec).unwrap_err();

        assert!(matches!(err, WriteError::Validation(_)), "{err:?}");
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            before,
            "file untouched"
        );
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn set_field_writes_typed_values() {
        let path = temp_file("[hotkey]\ntrigger = \"f16\"\n");
        let spec = spec_for(SchemaId::Main);

        set_field(&path, "voice.auto_paste", TypedInput::Bool(false), &spec).unwrap();
        set_field(
            &path,
            "overlay.max_text_lines",
            TypedInput::Integer(3),
            &spec,
        )
        .unwrap();
        set_field(
            &path,
            "overlay.position",
            TypedInput::Str("top".into()),
            &spec,
        )
        .unwrap();

        let body = std::fs::read_to_string(&path).unwrap();
        assert!(body.contains("auto_paste = false"), "{body}");
        assert!(body.contains("max_text_lines = 3"), "{body}");
        assert!(body.contains("position = \"top\""), "{body}");
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn set_field_writes_string_arrays() {
        let path = temp_file("name = \"default\"\n[asr]\ninstance = \"apple\"\n");
        let spec = spec_for(SchemaId::Profile);

        set_field(
            &path,
            "asr.hotwords",
            TypedInput::StrArray(vec!["Rust".into(), "tokio".into()]),
            &spec,
        )
        .unwrap();

        let body = std::fs::read_to_string(&path).unwrap();
        assert!(body.contains("hotwords = [\"Rust\", \"tokio\"]"), "{body}");
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn set_field_is_idempotent() {
        let path = temp_file("[hotkey]\ntrigger = \"f16\"\n");
        let spec = spec_for(SchemaId::Main);
        set_field(
            &path,
            "overlay.position",
            TypedInput::Str("middle".into()),
            &spec,
        )
        .unwrap();
        let first = std::fs::read_to_string(&path).unwrap();
        set_field(
            &path,
            "overlay.position",
            TypedInput::Str("middle".into()),
            &spec,
        )
        .unwrap();
        assert_eq!(first, std::fs::read_to_string(&path).unwrap());
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn unset_field_removes_key() {
        let path = temp_file("[hotkey]\ntrigger = \"f16\"\n[overlay]\nposition = \"top\"\n");
        let spec = spec_for(SchemaId::Main);

        unset_field(&path, "overlay.position", &spec).unwrap();

        let body = std::fs::read_to_string(&path).unwrap();
        assert!(!body.contains("position"), "key removed: {body}");
        crate::config::main::parse(&body).unwrap();
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn unset_required_field_is_rejected() {
        let path = temp_file("[hotkey]\ntrigger = \"f16\"\n");
        let spec = spec_for(SchemaId::Main);
        let before = std::fs::read_to_string(&path).unwrap();

        let err = unset_field(&path, "hotkey.trigger", &spec).unwrap_err();

        assert!(matches!(err, WriteError::Validation(_)), "{err:?}");
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            before,
            "file untouched"
        );
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn set_field_rejects_unparseable_hotkey() {
        let path = temp_file("[hotkey]\ntrigger = \"f16\"\n");
        let before = std::fs::read_to_string(&path).unwrap();
        let spec = spec_for(SchemaId::Main);

        let err = set_field(
            &path,
            "hotkey.trigger",
            TypedInput::Str("not a key".into()),
            &spec,
        )
        .unwrap_err();

        assert!(matches!(err, WriteError::Semantic(_)), "{err:?}");
        assert_eq!(std::fs::read_to_string(&path).unwrap(), before);
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn set_resume_hotkey_rejects_invalid_combo() {
        let path = temp_file("[hotkey]\ntrigger = \"f16\"\n");
        let spec = spec_for(SchemaId::Main);

        let err = set_field(
            &path,
            "hotkey.resume",
            TypedInput::Str("not+a+key".to_string()),
            &spec,
        )
        .unwrap_err();

        assert!(matches!(err, WriteError::Semantic(_)));
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }
}
