#![cfg_attr(not(test), allow(dead_code))]

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Severity {
    Error,
    Warning,
    #[allow(dead_code)]
    Info,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Diagnostic {
    pub severity: Severity,
    pub path: String,
    pub message: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ValueKind {
    String,
    #[allow(dead_code)]
    Bool,
    Integer,
    #[allow(dead_code)]
    Float,
    Enum,
    #[allow(dead_code)]
    Array,
    Color,
    Table,
    FreeTable,
}

#[derive(Debug, Clone)]
pub struct FieldSpec {
    name: String,
    kind: ValueKind,
    required: bool,
    default: Option<String>,
    secret: bool,
    allowed_values: Vec<String>,
    free_table: bool,
    description_key: Option<&'static str>,
}

#[derive(Debug, Clone)]
pub struct ConfigSpec {
    name: String,
    fields: Vec<FieldSpec>,
}

impl FieldSpec {
    pub fn string(name: impl Into<String>) -> Self {
        Self::new(name, ValueKind::String)
    }

    pub fn bool(name: impl Into<String>) -> Self {
        Self::new(name, ValueKind::Bool)
    }

    pub fn integer(name: impl Into<String>) -> Self {
        Self::new(name, ValueKind::Integer)
    }

    pub fn float(name: impl Into<String>) -> Self {
        Self::new(name, ValueKind::Float)
    }

    pub fn array(name: impl Into<String>) -> Self {
        Self::new(name, ValueKind::Array)
    }

    pub fn color(name: impl Into<String>) -> Self {
        Self::new(name, ValueKind::Color)
    }

    pub fn table(name: impl Into<String>) -> Self {
        Self::new(name, ValueKind::Table)
    }

    fn new(name: impl Into<String>, kind: ValueKind) -> Self {
        Self {
            name: name.into(),
            kind,
            required: false,
            default: None,
            secret: false,
            allowed_values: Vec::new(),
            free_table: false,
            description_key: None,
        }
    }

    pub fn required(mut self) -> Self {
        self.required = true;
        self
    }

    pub fn optional(mut self) -> Self {
        self.required = false;
        self
    }

    pub fn default(mut self, value: impl Into<String>) -> Self {
        self.default = Some(value.into());
        self
    }

    pub fn secret(mut self) -> Self {
        self.secret = true;
        self
    }

    pub fn allowed_values(mut self, values: impl IntoIterator<Item = impl Into<String>>) -> Self {
        self.allowed_values = values.into_iter().map(Into::into).collect();
        self.kind = ValueKind::Enum;
        self
    }

    pub fn free_table(mut self) -> Self {
        self.free_table = true;
        self.kind = ValueKind::FreeTable;
        self
    }

    pub fn description_key(mut self, key: &'static str) -> Self {
        self.description_key = Some(key);
        self
    }

    pub fn display_value(&self, value: &toml::Value) -> String {
        if self.secret {
            return match value.as_str() {
                Some("") => "<empty>".to_string(),
                Some(_) => "<set>".to_string(),
                None => "<set>".to_string(),
            };
        }
        match value {
            toml::Value::String(s) => s.clone(),
            other => other.to_string(),
        }
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn required_without_default(&self) -> bool {
        self.required && self.default.is_none()
    }

    pub fn is_secret(&self) -> bool {
        self.secret
    }

    pub fn kind(&self) -> ValueKind {
        self.kind
    }

    pub fn description_key_value(&self) -> Option<&'static str> {
        self.description_key
    }
}

impl ConfigSpec {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            fields: Vec::new(),
        }
    }

    pub fn field(mut self, field: FieldSpec) -> Self {
        self.fields.push(field);
        self
    }

    pub fn field_for_path(&self, path: &str) -> Option<&FieldSpec> {
        self.fields.iter().find(|field| field.name == path)
    }

    pub fn fields(&self) -> &[FieldSpec] {
        &self.fields
    }
}

pub fn validate_value(spec: &ConfigSpec, value: &toml::Value) -> Vec<Diagnostic> {
    if !value.is_table() {
        return vec![Diagnostic {
            severity: Severity::Error,
            path: spec.name.clone(),
            message: "expected top-level TOML table".to_string(),
        }];
    };

    let mut diagnostics = Vec::new();

    for field in &spec.fields {
        match value_at_path(value, field.name()) {
            Some(actual) => validate_field(field, actual, &mut diagnostics),
            None if field.required && field.default.is_none() => diagnostics.push(Diagnostic {
                severity: Severity::Error,
                path: field.name.clone(),
                message: "required field missing".to_string(),
            }),
            None => {}
        }
    }

    collect_unknown_fields(spec, value, "", &mut diagnostics);

    diagnostics
}

fn value_at_path<'a>(value: &'a toml::Value, path: &str) -> Option<&'a toml::Value> {
    let mut current = value;
    for part in path.split('.') {
        current = current.as_table()?.get(part)?;
    }
    Some(current)
}

fn parent_path(path: &str) -> Option<&str> {
    path.rsplit_once('.').map(|(parent, _)| parent)
}

fn collect_unknown_fields(
    spec: &ConfigSpec,
    value: &toml::Value,
    prefix: &str,
    diagnostics: &mut Vec<Diagnostic>,
) {
    let Some(table) = value.as_table() else {
        return;
    };

    for (key, child) in table {
        let path = if prefix.is_empty() {
            key.clone()
        } else {
            format!("{prefix}.{key}")
        };

        if spec.field_for_path(&path).is_some() {
            collect_unknown_fields(spec, child, &path, diagnostics);
            continue;
        }
        if is_under_free_table(spec, &path) {
            continue;
        }
        if child.is_table() && has_descendant_field(spec, &path) {
            collect_unknown_fields(spec, child, &path, diagnostics);
            continue;
        }

        diagnostics.push(Diagnostic {
            severity: Severity::Warning,
            path,
            message: "unknown field".to_string(),
        });
    }
}

fn is_under_free_table(spec: &ConfigSpec, path: &str) -> bool {
    let mut current = parent_path(path);
    while let Some(parent) = current {
        if spec
            .field_for_path(parent)
            .is_some_and(|field| field.free_table)
        {
            return true;
        }
        current = parent_path(parent);
    }
    false
}

fn has_descendant_field(spec: &ConfigSpec, path: &str) -> bool {
    let prefix = format!("{path}.");
    spec.fields
        .iter()
        .any(|field| field.name().starts_with(&prefix))
}

fn validate_field(field: &FieldSpec, value: &toml::Value, diagnostics: &mut Vec<Diagnostic>) {
    match field.kind {
        ValueKind::String => {
            if !value.is_str() {
                push_type_error(field, "string", diagnostics);
                return;
            }
            if field.secret && value.as_str().is_some_and(str::is_empty) {
                diagnostics.push(Diagnostic {
                    severity: Severity::Error,
                    path: field.name.clone(),
                    message: "secret field cannot be empty".to_string(),
                });
            }
        }
        ValueKind::Integer => {
            if !value.is_integer() {
                push_type_error(field, "integer", diagnostics);
                return;
            }
        }
        ValueKind::Float => {
            if !value.is_float() && !value.is_integer() {
                push_type_error(field, "float", diagnostics);
                return;
            }
        }
        ValueKind::Bool => {
            if !value.is_bool() {
                push_type_error(field, "bool", diagnostics);
                return;
            }
        }
        ValueKind::Enum => {
            let Some(actual) = value.as_str() else {
                push_type_error(field, "string", diagnostics);
                return;
            };
            if !field.allowed_values.iter().any(|allowed| allowed == actual) {
                diagnostics.push(Diagnostic {
                    severity: Severity::Error,
                    path: field.name.clone(),
                    message: format!(
                        "invalid value {actual:?}; expected one of {}",
                        field.allowed_values.join(", ")
                    ),
                });
            }
        }
        ValueKind::Array => {
            if !value.is_array() {
                push_type_error(field, "array", diagnostics);
                return;
            }
        }
        ValueKind::Color => {
            let valid_hex = value
                .as_integer()
                .is_some_and(|value| (0..=0xFF_FFFF).contains(&value));
            if !value.is_str() && !valid_hex {
                push_type_error(
                    field,
                    "palette name string or 0xRRGGBB integer",
                    diagnostics,
                );
                return;
            }
        }
        ValueKind::Table => {
            if !value.is_table() {
                push_type_error(field, "table", diagnostics);
                return;
            }
        }
        ValueKind::FreeTable => {
            if !value.is_table() {
                push_type_error(field, "table", diagnostics);
                return;
            }
        }
    }

    if field.free_table {
        return;
    }

    let _ = diagnostics;
}

fn push_type_error(field: &FieldSpec, expected: &str, diagnostics: &mut Vec<Diagnostic>) {
    diagnostics.push(Diagnostic {
        severity: Severity::Error,
        path: field.name.clone(),
        message: format!("expected {expected}"),
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_spec() -> ConfigSpec {
        ConfigSpec::new("post.llm")
            .field(FieldSpec::string("name").required())
            .field(FieldSpec::string("api_key").required().secret())
            .field(
                FieldSpec::string("format")
                    .default("openai")
                    .allowed_values(["openai", "anthropic"]),
            )
            .field(FieldSpec::integer("timeout_ms").optional())
            .field(FieldSpec::table("extra_body").optional().free_table())
    }

    #[test]
    fn field_spec_keeps_description_key_through_builder_chain() {
        let field = FieldSpec::string("hotkey.trigger")
            .required()
            .description_key("config.field.hotkey.trigger.description");

        assert_eq!(
            field.description_key_value(),
            Some("config.field.hotkey.trigger.description")
        );
    }

    #[test]
    fn validate_accepts_matching_required_fields() {
        let value: toml::Value = toml::toml! {
            name = "deepseek"
            api_key = "sk-test"
            format = "openai"
            timeout_ms = 2000
            [extra_body]
            thinking = "disabled"
        }
        .into();

        let diagnostics = validate_value(&sample_spec(), &value);

        assert!(diagnostics.is_empty(), "{diagnostics:?}");
    }

    #[test]
    fn validate_reports_missing_required_fields_as_errors() {
        let value: toml::Value = toml::toml! {
            name = "deepseek"
        }
        .into();

        let diagnostics = validate_value(&sample_spec(), &value);

        assert!(diagnostics.iter().any(|diagnostic| {
            diagnostic.severity == Severity::Error
                && diagnostic.path == "api_key"
                && diagnostic.message.contains("required")
        }));
    }

    #[test]
    fn validate_reports_type_mismatch_as_error() {
        let value: toml::Value = toml::toml! {
            name = "deepseek"
            api_key = "sk-test"
            timeout_ms = "slow"
        }
        .into();

        let diagnostics = validate_value(&sample_spec(), &value);

        assert!(diagnostics.iter().any(|diagnostic| {
            diagnostic.severity == Severity::Error
                && diagnostic.path == "timeout_ms"
                && diagnostic.message.contains("integer")
        }));
    }

    #[test]
    fn validate_reports_invalid_enum_as_error() {
        let value: toml::Value = toml::toml! {
            name = "deepseek"
            api_key = "sk-test"
            format = "native"
        }
        .into();

        let diagnostics = validate_value(&sample_spec(), &value);

        assert!(diagnostics.iter().any(|diagnostic| {
            diagnostic.severity == Severity::Error
                && diagnostic.path == "format"
                && diagnostic.message.contains("openai")
                && diagnostic.message.contains("anthropic")
        }));
    }

    #[test]
    fn validate_reports_unknown_fields_as_warnings() {
        let value: toml::Value = toml::toml! {
            name = "deepseek"
            api_key = "sk-test"
            typo = true
        }
        .into();

        let diagnostics = validate_value(&sample_spec(), &value);

        assert!(diagnostics.iter().any(|diagnostic| {
            diagnostic.severity == Severity::Warning
                && diagnostic.path == "typo"
                && diagnostic.message.contains("unknown")
        }));
    }

    #[test]
    fn validate_reads_nested_dotted_paths() {
        let spec = ConfigSpec::new("main")
            .field(FieldSpec::string("hotkey.trigger").required())
            .field(
                FieldSpec::string("voice.vad.backend")
                    .default("off")
                    .allowed_values(["off", "silero"]),
            );
        let value: toml::Value = toml::toml! {
            [hotkey]
            trigger = "f16"

            [voice.vad]
            backend = "silero"
        }
        .into();

        let diagnostics = validate_value(&spec, &value);

        assert!(diagnostics.is_empty(), "{diagnostics:?}");
    }

    #[test]
    fn validate_reports_nested_unknown_fields() {
        let spec = ConfigSpec::new("main")
            .field(FieldSpec::string("hotkey.trigger").required())
            .field(FieldSpec::table("voice").optional())
            .field(FieldSpec::string("voice.vad.backend").optional());
        let value: toml::Value = toml::toml! {
            [hotkey]
            trigger = "f16"

            [voice.vad]
            backend = "off"
            typo = true
        }
        .into();

        let diagnostics = validate_value(&spec, &value);

        assert!(diagnostics.iter().any(|diagnostic| {
            diagnostic.severity == Severity::Warning
                && diagnostic.path == "voice.vad.typo"
                && diagnostic.message.contains("unknown")
        }));
    }

    #[test]
    fn free_table_allows_nested_unknown_fields() {
        let value: toml::Value = toml::toml! {
            name = "deepseek"
            api_key = "sk-test"
            [extra_body]
            provider_specific = true
            nested = { enabled = false }
        }
        .into();

        let diagnostics = validate_value(&sample_spec(), &value);

        assert!(diagnostics.is_empty(), "{diagnostics:?}");
    }

    #[test]
    fn secret_empty_string_is_an_error_and_rendered_redacted() {
        let value: toml::Value = toml::toml! {
            name = "deepseek"
            api_key = ""
        }
        .into();

        let diagnostics = validate_value(&sample_spec(), &value);

        assert!(diagnostics.iter().any(|diagnostic| {
            diagnostic.severity == Severity::Error
                && diagnostic.path == "api_key"
                && diagnostic.message.contains("empty")
        }));
        assert_eq!(
            sample_spec()
                .field_for_path("api_key")
                .unwrap()
                .display_value(value.get("api_key").unwrap()),
            "<empty>"
        );
    }

    #[test]
    fn validate_supports_bool_float_and_array_kinds() {
        let spec = ConfigSpec::new("mixed")
            .field(FieldSpec::bool("enabled").required())
            .field(FieldSpec::float("threshold").required())
            .field(FieldSpec::array("items").required());
        let value: toml::Value = toml::toml! {
            enabled = true
            threshold = 0.5
            items = ["a", "b"]
        }
        .into();

        let diagnostics = validate_value(&spec, &value);

        assert!(diagnostics.is_empty(), "{diagnostics:?}");
    }

    #[test]
    fn validate_accepts_integer_for_float_fields() {
        let spec =
            ConfigSpec::new("main").field(FieldSpec::float("voice.vad.threshold").required());
        let value: toml::Value = toml::toml! {
            [voice.vad]
            threshold = 1
        }
        .into();

        let diagnostics = validate_value(&spec, &value);

        assert!(diagnostics.is_empty(), "{diagnostics:?}");
    }
}
