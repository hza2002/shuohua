use std::path::{Path, PathBuf};

use anyhow::anyhow;

use crate::config::field_view::{self, ControlKind, FieldOrigin};
use crate::config::field_write::{self, typed_to_item, TypedInput, WriteError};
use crate::config::post::{resolve_kind_in_root, PostKind};
use crate::config::profile_compose_write::{self as write, MoveDir, OverrideSection};
use crate::config::schema::{self, SchemaId};
use crate::config::spec::{validate_field, ConfigSpec, FieldSpec, Severity};
use crate::tui::settings::SettingsRow;

/// What a row acts on when the user presses a key.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ComposerRowKind {
    Name,
    AsrInstance,
    AsrOverride { field: String },
    Hotwords,
    SectionHeader,
    ChainMember { id: String, kind: Option<PostKind> }, // None = dangling file
    LlmOverride { member_id: String, field: String },
}

#[derive(Debug, Clone, PartialEq)]
pub struct ComposerRow {
    pub row: SettingsRow, // for render (value/origin/control/secret/desc)
    pub kind: ComposerRowKind,
}

#[derive(Debug)]
pub struct ProfileComposer {
    pub profile_path: PathBuf,
    pub selected: usize,
    rows: Vec<ComposerRow>,
    config_root: PathBuf,
}

impl ProfileComposer {
    pub fn load(profile_path: PathBuf, config_root: &Path) -> Self {
        let mut composer = Self {
            profile_path,
            selected: 0,
            rows: Vec::new(),
            config_root: config_root.to_path_buf(),
        };
        composer.refresh();
        composer
    }

    pub fn rows(&self) -> &[ComposerRow] {
        &self.rows
    }

    pub fn refresh(&mut self) {
        let parsed = parse_file(&self.profile_path);
        self.rows = self.build_rows(&parsed);
        if self.selected >= self.rows.len() {
            self.selected = self.rows.len().saturating_sub(1);
        }
    }

    fn stem(&self) -> String {
        self.profile_path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("")
            .to_string()
    }

    fn build_rows(&self, parsed: &toml::Value) -> Vec<ComposerRow> {
        let mut rows = Vec::new();
        let profile_spec = schema::spec_for(SchemaId::Profile);

        // 1. Name
        rows.push(self.name_row(&profile_spec, parsed));

        // 2. AsrInstance
        let instance = parsed
            .get("asr")
            .and_then(|a| a.get("instance"))
            .and_then(toml::Value::as_str)
            .unwrap_or("")
            .to_string();
        rows.push(self.asr_instance_row(&profile_spec, &instance));

        // 3. Hotwords
        rows.push(self.hotwords_row(&profile_spec, parsed));

        // 4. asr overrides
        let asr_section = parsed.get("asr");
        let (asr_spec, instance_type) = self.resolve_asr_spec(&instance);
        rows.push(section_header(crate::i18n::tr(
            "tui.configure.composer.section_asr",
            &[(
                "instance",
                instance_type.as_deref().unwrap_or("missing").to_string(),
            )],
        )));
        match &asr_spec {
            Some(spec) => {
                for field in spec.fields() {
                    if is_skipped_override_field(field) {
                        continue;
                    }
                    rows.push(self.asr_override_row(spec, field, asr_section, &instance));
                }
                // Stale `[asr]` override keys not present in the resolved provider
                // schema (e.g. `app_key` after switching to an Apple instance).
                // A key is legitimate if either the Profile schema or the ASR
                // provider schema owns it — only flag keys recognized by neither.
                let is_profile_field =
                    |key: &str| profile_spec.field_for_path(&format!("asr.{key}")).is_some();
                for (key, val) in override_entries(asr_section) {
                    if is_profile_field(&key) || spec.field_for_path(&key).is_some() {
                        continue;
                    }
                    rows.push(error_override_row(
                        ComposerRowKind::AsrOverride { field: key.clone() },
                        &key,
                        &display_toml(&val),
                    ));
                }
            }
            None => {
                // Typeless/missing instance file: still surface the profile's own
                // `[asr]` override keys as Error rows, but skip those owned by
                // the Profile schema (handled by dedicated rows).
                for (key, val) in override_entries(asr_section) {
                    if profile_spec.field_for_path(&format!("asr.{key}")).is_some() {
                        continue;
                    }
                    rows.push(error_override_row(
                        ComposerRowKind::AsrOverride { field: key.clone() },
                        &key,
                        &display_toml(&val),
                    ));
                }
            }
        }

        // 5-6. chain
        rows.push(section_header(crate::t!(
            "tui.configure.composer.section_chain"
        )));
        let chain: Vec<String> = parsed
            .get("post")
            .and_then(|p| p.get("chain"))
            .and_then(toml::Value::as_array)
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(str::to_string))
                    .collect()
            })
            .unwrap_or_default();
        let overrides_section = parsed.get("post").and_then(|p| p.get("overrides"));

        let llm_spec = schema::spec_for(SchemaId::PostLlm);
        for id in &chain {
            let kind = resolve_kind_in_root(&self.config_root, id);
            rows.push(self.chain_member_row(id, kind));
            if kind == Some(PostKind::Llm) {
                let member_override = overrides_section.and_then(|o| o.get(id));
                let component = self.component_value(id);
                for field in llm_spec.fields() {
                    if is_skipped_override_field(field) {
                        continue;
                    }
                    rows.push(self.llm_override_row(
                        &llm_spec,
                        field,
                        id,
                        member_override,
                        component.as_ref(),
                    ));
                }
            }
        }

        // 8. Dangling overrides: `[post.overrides.<id>]` for an id that is NOT an
        // llm member in the chain.
        let llm_ids: Vec<String> = chain
            .iter()
            .filter(|id| resolve_kind_in_root(&self.config_root, id) == Some(PostKind::Llm))
            .cloned()
            .collect();
        let dangling: Vec<(String, toml::value::Table)> = overrides_section
            .and_then(toml::Value::as_table)
            .map(|t| {
                t.iter()
                    .filter(|(id, _)| !llm_ids.contains(id))
                    .filter_map(|(id, v)| v.as_table().map(|tbl| (id.clone(), tbl.clone())))
                    .collect()
            })
            .unwrap_or_default();
        if !dangling.is_empty() {
            rows.push(section_header(crate::t!(
                "tui.configure.composer.section_dangling"
            )));
            for (id, tbl) in dangling {
                for (field, val) in tbl {
                    rows.push(error_override_row(
                        ComposerRowKind::LlmOverride {
                            member_id: id.clone(),
                            field: field.clone(),
                        },
                        &format!("{id}.{field}"),
                        &display_toml(&val),
                    ));
                }
            }
        }

        rows
    }

    /// Resolve the provider spec for the current asr instance, plus its declared
    /// `type` (for the section header). Returns `(None, None)` when the instance
    /// file is missing or typeless.
    fn resolve_asr_spec(&self, instance: &str) -> (Option<ConfigSpec>, Option<String>) {
        if instance.is_empty() {
            return (None, None);
        }
        let path = self
            .config_root
            .join("asr")
            .join(format!("{instance}.toml"));
        let Some(value) = parse_existing(&path) else {
            return (None, None);
        };
        let instance_type = value
            .get("type")
            .and_then(toml::Value::as_str)
            .map(str::to_string);
        match schema::asr_spec_for_value(instance, &path, &value) {
            Ok(spec) => (Some(spec), instance_type),
            Err(_) => (None, instance_type),
        }
    }

    fn component_value(&self, id: &str) -> Option<toml::Value> {
        let path = self.config_root.join("post").join(format!("{id}.toml"));
        parse_existing(&path)
    }

    fn name_row(&self, spec: &ConfigSpec, parsed: &toml::Value) -> ComposerRow {
        let field = spec.field_for_path("name").expect("profile has name field");
        let derived = field_view::display_name_from_stem(&self.stem());
        let present = parsed.get("name").and_then(toml::Value::as_str);
        let (value, origin) = match present {
            Some(s) if !s.is_empty() => (s.to_string(), FieldOrigin::Set),
            _ => (derived, FieldOrigin::Default),
        };
        let row = self.spec_row(spec, field, "name", value, origin, None);
        ComposerRow {
            row,
            kind: ComposerRowKind::Name,
        }
    }

    fn asr_instance_row(&self, spec: &ConfigSpec, instance: &str) -> ComposerRow {
        let field = spec
            .field_for_path("asr.instance")
            .expect("profile has asr.instance field");
        let rel = format!("profile/{}.toml", self.stem());
        let dynamic = field_view::dynamic_domain(&rel, "asr.instance", &self.config_root);
        let control = field_view::control_for(field, dynamic);
        let mut row = self.spec_row(
            spec,
            field,
            "asr.instance",
            instance.to_string(),
            FieldOrigin::Set,
            Some(control),
        );
        row.field_path = "asr.instance".to_string();
        ComposerRow {
            row,
            kind: ComposerRowKind::AsrInstance,
        }
    }

    fn hotwords_row(&self, spec: &ConfigSpec, parsed: &toml::Value) -> ComposerRow {
        let field = spec
            .field_for_path("asr.hotwords")
            .expect("profile has asr.hotwords field");
        let present = parsed.get("asr").and_then(|a| a.get("hotwords"));
        let (value, origin) = match present {
            Some(v) => (display_toml(v), FieldOrigin::Set),
            None => (String::new(), FieldOrigin::Default),
        };
        let mut row = self.spec_row(spec, field, "asr.hotwords", value, origin, None);
        row.field_path = "asr.hotwords".to_string();
        ComposerRow {
            row,
            kind: ComposerRowKind::Hotwords,
        }
    }

    fn asr_override_row(
        &self,
        spec: &ConfigSpec,
        field: &FieldSpec,
        section: Option<&toml::Value>,
        _instance: &str,
    ) -> ComposerRow {
        let component = {
            let instance = section
                .and_then(|s| s.get("instance"))
                .and_then(toml::Value::as_str)
                .unwrap_or("");
            self.component_value(instance)
        };
        let (value, origin) = resolve_field(section, component.as_ref(), spec, field);
        let row = self.spec_row(spec, field, field.name(), value, origin, None);
        ComposerRow {
            row,
            kind: ComposerRowKind::AsrOverride {
                field: field.name().to_string(),
            },
        }
    }

    fn llm_override_row(
        &self,
        spec: &ConfigSpec,
        field: &FieldSpec,
        member_id: &str,
        member_override: Option<&toml::Value>,
        component: Option<&toml::Value>,
    ) -> ComposerRow {
        let (value, origin) = resolve_field(member_override, component, spec, field);
        let row = self.spec_row(spec, field, field.name(), value, origin, None);
        ComposerRow {
            row,
            kind: ComposerRowKind::LlmOverride {
                member_id: member_id.to_string(),
                field: field.name().to_string(),
            },
        }
    }

    fn chain_member_row(&self, id: &str, kind: Option<PostKind>) -> ComposerRow {
        let origin = if kind.is_none() {
            FieldOrigin::Error
        } else {
            FieldOrigin::Set
        };
        let row = SettingsRow {
            group: String::new(),
            field_path: id.to_string(),
            display_key: id.to_string(),
            value: String::new(),
            default_value: String::new(),
            origin,
            control: ControlKind::ReadOnly,
            editable: false,
            secret: false,
            can_unset: false,
            source: self.profile_path.display().to_string(),
            description_key: None,
        };
        ComposerRow {
            row,
            kind: ComposerRowKind::ChainMember {
                id: id.to_string(),
                kind,
            },
        }
    }

    /// Build a `SettingsRow` from a resolved `FieldSpec` value, mirroring how the
    /// draft forms populate every field.
    fn spec_row(
        &self,
        _spec: &ConfigSpec,
        field: &FieldSpec,
        field_path: &str,
        value: String,
        origin: FieldOrigin,
        control: Option<ControlKind>,
    ) -> SettingsRow {
        // `value` is already display-ready: secret masking happens once upstream
        // in `display_field`/`FieldSpec::display_value`, so don't re-mask here.
        let control = control.unwrap_or_else(|| field_view::control_for(field, None));
        let editable = control != ControlKind::ReadOnly;
        SettingsRow {
            group: String::new(),
            field_path: field_path.to_string(),
            display_key: field.name().to_string(),
            value,
            default_value: field.default_value().unwrap_or("").to_string(),
            origin,
            control,
            editable,
            secret: field.is_secret(),
            can_unset: !field.required_without_default(),
            source: self.profile_path.display().to_string(),
            description_key: field.description_key_value(),
        }
    }

    // ---- actions ----

    pub fn commit_edit(&mut self, input: TypedInput) -> anyhow::Result<()> {
        let Some(kind) = self.selected_kind().cloned() else {
            return Ok(());
        };
        match kind {
            ComposerRowKind::Name => {
                set_field(&self.profile_path, "name", input, SchemaId::Profile)?;
            }
            ComposerRowKind::AsrInstance => {
                set_field(&self.profile_path, "asr.instance", input, SchemaId::Profile)?;
            }
            ComposerRowKind::Hotwords => {
                set_field(&self.profile_path, "asr.hotwords", input, SchemaId::Profile)?;
            }
            ComposerRowKind::AsrOverride { field } => {
                let instance = self.current_instance();
                let (spec, _) = self.resolve_asr_spec(&instance);
                let spec = spec.ok_or_else(|| {
                    anyhow!("asr instance {instance:?} has no resolvable provider schema")
                })?;
                validate_override(&spec, &field, &input)?;
                write::set_override(
                    &self.profile_path,
                    &OverrideSection::Asr,
                    &field,
                    typed_to_item(&input),
                )?;
            }
            ComposerRowKind::LlmOverride { member_id, field } => {
                let spec = schema::spec_for(SchemaId::PostLlm);
                validate_override(&spec, &field, &input)?;
                write::set_override(
                    &self.profile_path,
                    &OverrideSection::Overrides(member_id),
                    &field,
                    typed_to_item(&input),
                )?;
            }
            ComposerRowKind::SectionHeader | ComposerRowKind::ChainMember { .. } => {}
        }
        self.refresh();
        Ok(())
    }

    pub fn reset_selected(&mut self) -> anyhow::Result<()> {
        let Some(kind) = self.selected_kind().cloned() else {
            return Ok(());
        };
        match kind {
            ComposerRowKind::AsrOverride { field } => {
                write::unset_override(&self.profile_path, &OverrideSection::Asr, &field)?;
            }
            ComposerRowKind::LlmOverride { member_id, field } => {
                write::unset_override(
                    &self.profile_path,
                    &OverrideSection::Overrides(member_id),
                    &field,
                )?;
            }
            ComposerRowKind::Name => {
                unset_field(&self.profile_path, "name", SchemaId::Profile)?;
            }
            ComposerRowKind::AsrInstance => {
                unset_field(&self.profile_path, "asr.instance", SchemaId::Profile)?;
            }
            _ => {}
        }
        self.refresh();
        Ok(())
    }

    pub fn add_member(&mut self, id: &str) -> anyhow::Result<()> {
        write::add_chain_member(&self.profile_path, id)?;
        self.refresh();
        Ok(())
    }

    pub fn remove_selected_member(&mut self) -> anyhow::Result<()> {
        if let Some(ComposerRowKind::ChainMember { id, .. }) = self.selected_kind().cloned() {
            write::remove_chain_member(&self.profile_path, &id)?;
            self.refresh();
        }
        Ok(())
    }

    pub fn move_selected_member(&mut self, dir: MoveDir) -> anyhow::Result<()> {
        if let Some(ComposerRowKind::ChainMember { id, .. }) = self.selected_kind().cloned() {
            if let Some(idx) = self.chain_index_of(&id) {
                write::move_chain_member(&self.profile_path, idx, dir)?;
                self.refresh();
            }
        }
        Ok(())
    }

    /// Collect all invalid override rows (Error origin, override kinds) — NOT
    /// dangling chain members.
    pub fn invalid_overrides(&self) -> Vec<(OverrideSection, String)> {
        self.rows
            .iter()
            .filter(|r| r.row.origin == FieldOrigin::Error)
            .filter_map(|r| match &r.kind {
                ComposerRowKind::AsrOverride { field } => {
                    Some((OverrideSection::Asr, field.clone()))
                }
                ComposerRowKind::LlmOverride { member_id, field } => {
                    Some((OverrideSection::Overrides(member_id.clone()), field.clone()))
                }
                _ => None,
            })
            .collect()
    }

    pub fn drop_all_invalid(&mut self) -> anyhow::Result<()> {
        let invalid = self.invalid_overrides();
        if !invalid.is_empty() {
            write::drop_invalid_overrides(&self.profile_path, &invalid)?;
            self.refresh();
        }
        Ok(())
    }

    /// Post component ids (bare stems) available to add to the chain, sorted.
    /// The chain allows duplicates, so ids already in the chain are still listed.
    pub fn available_members(&self) -> Vec<String> {
        let dir = self.config_root.join("post");
        let mut ids: Vec<String> = std::fs::read_dir(&dir)
            .into_iter()
            .flatten()
            .flatten()
            .filter_map(|entry| {
                let path = entry.path();
                if path.extension().and_then(|e| e.to_str()) != Some("toml") {
                    return None;
                }
                path.file_stem()
                    .and_then(|s| s.to_str())
                    .map(str::to_string)
            })
            .collect();
        ids.sort();
        ids.dedup();
        ids
    }

    pub fn selected_kind(&self) -> Option<&ComposerRowKind> {
        self.rows.get(self.selected).map(|row| &row.kind)
    }

    fn current_instance(&self) -> String {
        parse_file(&self.profile_path)
            .get("asr")
            .and_then(|a| a.get("instance"))
            .and_then(toml::Value::as_str)
            .unwrap_or("")
            .to_string()
    }

    fn chain_index_of(&self, id: &str) -> Option<usize> {
        parse_file(&self.profile_path)
            .get("post")
            .and_then(|p| p.get("chain"))
            .and_then(toml::Value::as_array)
            .and_then(|arr| arr.iter().position(|v| v.as_str() == Some(id)))
    }
}

/// resolve_field per the composer contract.
fn resolve_field(
    section_present: Option<&toml::Value>,
    component_value: Option<&toml::Value>,
    spec: &ConfigSpec,
    field: &FieldSpec,
) -> (String, FieldOrigin) {
    let override_val = section_present.and_then(|s| s.get(field.name()));
    if let Some(v) = override_val {
        if spec.field_for_path(field.name()).is_some() {
            return (display_field(field, v), FieldOrigin::Set);
        }
        // Present but not in the resolved schema.
        return (display_toml(v), FieldOrigin::Error);
    }
    if let Some(v) = component_value.and_then(|c| c.get(field.name())) {
        return (display_field(field, v), FieldOrigin::Default);
    }
    (
        field.default_value().unwrap_or("").to_string(),
        FieldOrigin::Default,
    )
}

/// Validate an override before writing it: the field must exist in the resolved
/// provider/component schema, and the new value must satisfy that field's type.
/// The profile schema's `[asr]`/`[post.overrides]` free-tables can't catch this,
/// so the composer enforces it here (design §校验模型).
fn validate_override(spec: &ConfigSpec, field: &str, input: &TypedInput) -> anyhow::Result<()> {
    let Some(field_spec) = spec.field_for_path(field) else {
        return Err(anyhow!(
            "field {field:?} is not valid for the resolved schema"
        ));
    };
    let value = typed_to_value(input);
    let mut diagnostics = Vec::new();
    validate_field(field_spec, &value, &mut diagnostics);
    let errors: Vec<String> = diagnostics
        .into_iter()
        .filter(|d| d.severity == Severity::Error)
        .map(|d| d.message)
        .collect();
    if !errors.is_empty() {
        return Err(anyhow!(errors.join("; ")));
    }
    Ok(())
}

fn typed_to_value(input: &TypedInput) -> toml::Value {
    match input {
        TypedInput::Bool(b) => toml::Value::Boolean(*b),
        TypedInput::Integer(i) => toml::Value::Integer(*i),
        TypedInput::Float(f) => toml::Value::Float(*f),
        TypedInput::Str(s) => toml::Value::String(s.clone()),
        TypedInput::StrArray(items) => {
            toml::Value::Array(items.iter().cloned().map(toml::Value::String).collect())
        }
    }
}

fn is_skipped_override_field(field: &FieldSpec) -> bool {
    matches!(field.name(), "type" | "name")
}

fn section_header(label: String) -> ComposerRow {
    ComposerRow {
        row: SettingsRow {
            group: String::new(),
            field_path: label.clone(),
            display_key: label,
            value: String::new(),
            default_value: String::new(),
            origin: FieldOrigin::Default,
            control: ControlKind::ReadOnly,
            editable: false,
            secret: false,
            can_unset: false,
            source: String::new(),
            description_key: None,
        },
        kind: ComposerRowKind::SectionHeader,
    }
}

fn error_override_row(kind: ComposerRowKind, display_key: &str, value: &str) -> ComposerRow {
    ComposerRow {
        row: SettingsRow {
            group: String::new(),
            field_path: display_key.to_string(),
            display_key: display_key.to_string(),
            value: value.to_string(),
            default_value: String::new(),
            origin: FieldOrigin::Error,
            control: ControlKind::ReadOnly,
            editable: false,
            secret: false,
            can_unset: true,
            source: String::new(),
            description_key: None,
        },
        kind,
    }
}

fn override_entries(section: Option<&toml::Value>) -> Vec<(String, toml::Value)> {
    section
        .and_then(toml::Value::as_table)
        .map(|t| t.iter().map(|(k, v)| (k.clone(), v.clone())).collect())
        .unwrap_or_default()
}

fn display_field(field: &FieldSpec, value: &toml::Value) -> String {
    if value.is_array() {
        return display_toml(value);
    }
    field.display_value(value)
}

fn display_toml(value: &toml::Value) -> String {
    match value {
        toml::Value::String(s) => s.clone(),
        toml::Value::Array(items) => items
            .iter()
            .filter_map(toml::Value::as_str)
            .collect::<Vec<_>>()
            .join("\n"),
        other => other.to_string(),
    }
}

fn parse_file(path: &Path) -> toml::Value {
    parse_existing(path).unwrap_or_else(|| toml::Value::Table(Default::default()))
}

fn parse_existing(path: &Path) -> Option<toml::Value> {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|body| toml::from_str::<toml::Value>(&body).ok())
}

fn set_field(
    path: &Path,
    field_path: &str,
    input: TypedInput,
    schema_id: SchemaId,
) -> anyhow::Result<()> {
    field_write::set_field(path, field_path, input, &schema::spec_for(schema_id))
        .map_err(write_error_to_anyhow)
}

fn unset_field(path: &Path, field_path: &str, schema_id: SchemaId) -> anyhow::Result<()> {
    field_write::unset_field(path, field_path, &schema::spec_for(schema_id))
        .map_err(write_error_to_anyhow)
}

fn write_error_to_anyhow(e: WriteError) -> anyhow::Error {
    match e {
        WriteError::Validation(diags) => anyhow!(diags
            .into_iter()
            .map(|d| d.message)
            .collect::<Vec<_>>()
            .join("; ")),
        WriteError::Semantic(msg) => anyhow!(msg),
        WriteError::Io(err) => err,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn root_with(
        profiles: &[(&str, &str)],
        asr: &[(&str, &str)],
        post: &[(&str, &str)],
    ) -> PathBuf {
        let root = std::env::temp_dir().join(format!("shuohua-composer-{}", ulid::Ulid::new()));
        for (sub, files) in [("profile", profiles), ("asr", asr), ("post", post)] {
            fs::create_dir_all(root.join(sub)).unwrap();
            for (name, body) in files {
                fs::write(root.join(sub).join(format!("{name}.toml")), body).unwrap();
            }
        }
        root
    }

    const DOUBAO: &str = "type=\"doubao\"\napp_key=\"a\"\naccess_key=\"b\"\n";
    const DEEPSEEK: &str = "type=\"llm\"\nname=\"deepseek\"\nbase_url=\"https://x\"\napi_key=\"k\"\nmodel=\"deepseek-chat\"\nprompt=\"{{text}}\"\n";
    const ZH: &str = "type=\"rule\"\npatterns=[]\n";

    #[test]
    fn resolves_inherited_override_and_dangling() {
        let profile = "name=\"D\"\n[asr]\ninstance=\"doubao\"\n[post]\nchain=[\"zh_filter\",\"deepseek\"]\n[post.overrides.deepseek]\nmodel=\"deepseek-v4\"\n";
        let root = root_with(
            &[("default", profile)],
            &[("doubao", DOUBAO)],
            &[("zh_filter", ZH), ("deepseek", DEEPSEEK)],
        );
        let c = ProfileComposer::load(root.join("profile/default.toml"), &root);

        let model = c
            .rows()
            .iter()
            .find(|r| matches!(&r.kind, ComposerRowKind::LlmOverride{member_id, field} if member_id=="deepseek" && field=="model"))
            .unwrap();
        assert_eq!(model.row.origin, FieldOrigin::Set);
        assert_eq!(model.row.value, "deepseek-v4");
        let base = c
            .rows()
            .iter()
            .find(
                |r| matches!(&r.kind, ComposerRowKind::LlmOverride{field, ..} if field=="base_url"),
            )
            .unwrap();
        assert_eq!(base.row.origin, FieldOrigin::Default);
        assert_eq!(base.row.value, "https://x");
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn stale_asr_override_after_switch_is_error() {
        let profile =
            "name=\"D\"\n[asr]\ninstance=\"apple\"\napp_key=\"stale\"\n[post]\nchain=[]\n";
        let apple = "type=\"apple\"\n";
        let root = root_with(&[("default", profile)], &[("apple", apple)], &[]);
        let c = ProfileComposer::load(root.join("profile/default.toml"), &root);
        let stale = c
            .rows()
            .iter()
            .find(|r| matches!(&r.kind, ComposerRowKind::AsrOverride{field} if field=="app_key"))
            .unwrap();
        assert_eq!(stale.row.origin, FieldOrigin::Error);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn dangling_chain_member_is_error() {
        let profile = "name=\"D\"\n[asr]\ninstance=\"doubao\"\n[post]\nchain=[\"ghost\"]\n";
        let root = root_with(&[("default", profile)], &[("doubao", DOUBAO)], &[]);
        let c = ProfileComposer::load(root.join("profile/default.toml"), &root);
        let ghost = c
            .rows()
            .iter()
            .find(|r| matches!(&r.kind, ComposerRowKind::ChainMember{id, kind} if id=="ghost" && kind.is_none()))
            .unwrap();
        assert_eq!(ghost.row.origin, FieldOrigin::Error);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn edit_inherited_creates_override_reset_removes_it() {
        let profile = "name=\"D\"\n[asr]\ninstance=\"doubao\"\n[post]\nchain=[\"deepseek\"]\n";
        let root = root_with(
            &[("default", profile)],
            &[("doubao", DOUBAO)],
            &[("deepseek", DEEPSEEK)],
        );
        let path = root.join("profile/default.toml");
        let mut c = ProfileComposer::load(path.clone(), &root);
        let idx = c
            .rows()
            .iter()
            .position(
                |r| matches!(&r.kind, ComposerRowKind::LlmOverride{field, ..} if field=="model"),
            )
            .unwrap();
        c.selected = idx;
        c.commit_edit(TypedInput::Str("deepseek-v4".into()))
            .unwrap();
        assert!(fs::read_to_string(&path).unwrap().contains("deepseek-v4"));
        let idx = c
            .rows()
            .iter()
            .position(
                |r| matches!(&r.kind, ComposerRowKind::LlmOverride{field, ..} if field=="model"),
            )
            .unwrap();
        c.selected = idx;
        c.reset_selected().unwrap();
        assert!(!fs::read_to_string(&path).unwrap().contains("deepseek-v4"));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn remove_member_and_drop_invalid() {
        let profile = "name=\"D\"\n[asr]\ninstance=\"apple\"\napp_key=\"stale\"\n[post]\nchain=[\"deepseek\"]\n";
        let apple = "type=\"apple\"\n";
        let root = root_with(
            &[("default", profile)],
            &[("apple", apple)],
            &[("deepseek", DEEPSEEK)],
        );
        let path = root.join("profile/default.toml");
        let mut c = ProfileComposer::load(path.clone(), &root);
        let idx = c
            .rows()
            .iter()
            .position(|r| matches!(&r.kind, ComposerRowKind::ChainMember{id, ..} if id=="deepseek"))
            .unwrap();
        c.selected = idx;
        c.remove_selected_member().unwrap();
        assert!(!fs::read_to_string(&path).unwrap().contains("deepseek"));
        c.drop_all_invalid().unwrap();
        assert!(!fs::read_to_string(&path).unwrap().contains("app_key"));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn rule_member_has_no_llm_override_rows() {
        let profile = "name=\"D\"\n[asr]\ninstance=\"doubao\"\n[post]\nchain=[\"zh_filter\"]\n";
        let root = root_with(
            &[("default", profile)],
            &[("doubao", DOUBAO)],
            &[("zh_filter", ZH)],
        );
        let c = ProfileComposer::load(root.join("profile/default.toml"), &root);
        let llm_rows = c
            .rows()
            .iter()
            .filter(|r| matches!(&r.kind, ComposerRowKind::LlmOverride{member_id, ..} if member_id=="zh_filter"))
            .count();
        assert_eq!(llm_rows, 0);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn add_member_appends_to_chain() {
        let profile = "name=\"D\"\n[asr]\ninstance=\"doubao\"\n[post]\nchain=[\"zh_filter\"]\n";
        let root = root_with(
            &[("default", profile)],
            &[("doubao", DOUBAO)],
            &[("zh_filter", ZH), ("deepseek", DEEPSEEK)],
        );
        let path = root.join("profile/default.toml");
        let mut c = ProfileComposer::load(path.clone(), &root);
        c.add_member("deepseek").unwrap();
        let prof = crate::config::profile::parse(&fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(prof.post.chain, vec!["zh_filter", "deepseek"]);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn move_selected_member_reorders_chain() {
        let profile =
            "name=\"D\"\n[asr]\ninstance=\"doubao\"\n[post]\nchain=[\"zh_filter\",\"deepseek\"]\n";
        let root = root_with(
            &[("default", profile)],
            &[("doubao", DOUBAO)],
            &[("zh_filter", ZH), ("deepseek", DEEPSEEK)],
        );
        let path = root.join("profile/default.toml");
        let mut c = ProfileComposer::load(path.clone(), &root);
        let idx = c
            .rows()
            .iter()
            .position(|r| matches!(&r.kind, ComposerRowKind::ChainMember{id, ..} if id=="deepseek"))
            .unwrap();
        c.selected = idx;
        c.move_selected_member(MoveDir::Up).unwrap();
        let prof = crate::config::profile::parse(&fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(prof.post.chain, vec!["deepseek", "zh_filter"]);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn commit_invalid_typed_override_is_rejected_and_not_written() {
        let profile = "name=\"D\"\n[asr]\ninstance=\"doubao\"\n[post]\nchain=[\"deepseek\"]\n";
        let root = root_with(
            &[("default", profile)],
            &[("doubao", DOUBAO)],
            &[("deepseek", DEEPSEEK)],
        );
        let path = root.join("profile/default.toml");
        let before = fs::read_to_string(&path).unwrap();
        let mut c = ProfileComposer::load(path.clone(), &root);
        // `extra_body` is a table field; a bare string must be rejected pre-write.
        let idx = c
            .rows()
            .iter()
            .position(
                |r| matches!(&r.kind, ComposerRowKind::LlmOverride{field, ..} if field=="extra_body"),
            )
            .unwrap();
        c.selected = idx;
        assert!(c
            .commit_edit(TypedInput::Str("not-a-table".into()))
            .is_err());
        assert_eq!(
            fs::read_to_string(&path).unwrap(),
            before,
            "file unchanged on rejected edit"
        );
        let _ = fs::remove_dir_all(root);
    }
}
