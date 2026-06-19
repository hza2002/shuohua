use crate::config::spec::ConfigSpec;

use super::{Template, TemplateValue, ThemePreset};

#[cfg_attr(not(test), allow(dead_code))]
pub fn render(template: &Template) -> String {
    render_with_lang(template, crate::i18n::Lang::EnUS)
}

pub fn render_with_lang(template: &Template, lang: crate::i18n::Lang) -> String {
    let mut body = String::new();
    body.push_str(&format!("# {}\n", template.title));
    body.push_str(&format!("# {}\n\n", template.description));
    body.push_str(&render_from_spec(
        &template.spec(),
        template.values,
        Some(lang),
    ));
    body
}

pub fn render_theme_preset(preset: &ThemePreset) -> String {
    preset.body.to_string()
}

fn render_from_spec(
    spec: &ConfigSpec,
    values: &[(&str, TemplateValue)],
    lang: Option<crate::i18n::Lang>,
) -> String {
    let mut body = String::new();
    let mut table_values = Vec::new();

    for field in spec.fields() {
        let Some((_, value)) = values.iter().find(|(name, _)| *name == field.name()) else {
            continue;
        };
        if matches!(value, TemplateValue::Table(_)) {
            table_values.push((field.name(), *value));
            continue;
        }
        push_field_comment(&mut body, field, lang);
        body.push_str(&format!("{} = {}\n", field.name(), render_value(value)));
    }

    if !body.is_empty() && !table_values.is_empty() {
        body.push('\n');
    }

    for (idx, (name, value)) in table_values.iter().enumerate() {
        if idx > 0 {
            body.push('\n');
        }
        if let Some(field) = spec.field_for_path(name) {
            push_field_comment(&mut body, field, lang);
        }
        body.push_str(&format!("[{name}]\n"));
        let TemplateValue::Table(entries) = value else {
            continue;
        };
        for (key, value) in *entries {
            let field_path = format!("{name}.{key}");
            if let Some(field) = spec.field_for_path(&field_path) {
                push_field_comment(&mut body, field, lang);
            }
            body.push_str(&format!("{key} = {}\n", render_value(value)));
        }
    }

    body
}

fn push_field_comment(
    body: &mut String,
    field: &crate::config::spec::FieldSpec,
    lang: Option<crate::i18n::Lang>,
) {
    let Some(lang) = lang else {
        return;
    };
    let Some(key) = field.description_key_value() else {
        return;
    };
    let text = crate::i18n::tr_lang(lang, key, &[]);
    for line in text.lines() {
        body.push_str("# ");
        body.push_str(line);
        body.push('\n');
    }
}

fn render_value(value: &TemplateValue) -> String {
    match value {
        TemplateValue::String(value) => format!("{value:?}"),
        TemplateValue::Integer(value) => value.to_string(),
        TemplateValue::Bool(value) => value.to_string(),
        TemplateValue::StringArray(values) => {
            let values = values
                .iter()
                .map(|value| format!("{value:?}"))
                .collect::<Vec<_>>()
                .join(", ");
            format!("[{values}]")
        }
        TemplateValue::InlineTable(entries) => {
            let entries = entries
                .iter()
                .map(|(key, value)| format!("{key} = {}", render_value(value)))
                .collect::<Vec<_>>()
                .join(", ");
            format!("{{ {entries} }}")
        }
        TemplateValue::Table(_) => unreachable!("tables are rendered by section"),
    }
}
