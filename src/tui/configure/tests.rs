use super::*;
use crate::config::inventory::InventoryStatus;
use crate::tui::configure::render::*;

fn sample_row(group: &str, key: &str, display: &str, source: &str) -> SettingsRow {
    SettingsRow {
        group: group.to_string(),
        key: key.to_string(),
        display_key: display.to_string(),
        value: "ok".to_string(),
        source: source.to_string(),
        status: InventoryStatus::Ok,
        description_key: None,
    }
}

#[test]
fn configure_modules_cycle_in_order() {
    assert_eq!(ConfigureModule::Overview.next(), ConfigureModule::Profile);
    assert_eq!(
        ConfigureModule::Profile.next(),
        ConfigureModule::AsrProvider
    );
    assert_eq!(
        ConfigureModule::AsrProvider.next(),
        ConfigureModule::PostProcessor
    );
    assert_eq!(
        ConfigureModule::PostProcessor.next(),
        ConfigureModule::Overview
    );
    assert_eq!(
        ConfigureModule::Overview.prev(),
        ConfigureModule::PostProcessor
    );
    assert_eq!(
        ConfigureModule::AsrProvider.inventory_module(),
        crate::config::inventory::InventoryModule::AsrProvider
    );
}

#[test]
fn selected_config_source_tracks_current_module_row() {
    let mut page = ConfigurePage::new();
    page.module = ConfigureModule::Main;
    page.rows = vec![
        sample_row("main", "config", "config", "/tmp/shuohua/config.toml"),
        sample_row(
            "asr",
            "apple.idle_pause",
            "idle_pause",
            "/tmp/shuohua/asr/apple.toml",
        ),
    ];
    page.selected = 0;

    assert_eq!(
        page.selected_config_source()
            .unwrap()
            .file_name()
            .and_then(|name| name.to_str()),
        Some("config.toml")
    );

    page.module = ConfigureModule::AsrProvider;
    page.clamp_selected();
    assert_eq!(
        page.selected_config_source().unwrap(),
        PathBuf::from("/tmp/shuohua/asr/apple.toml")
    );
}

#[test]
fn vertical_navigation_moves_focused_column() {
    let mut page = ConfigurePage::new();
    page.module = ConfigureModule::Overview;
    page.focus = ConfigureFocus::Modules;

    page.move_selection(1);
    assert_eq!(page.module, ConfigureModule::Profile);

    page.focus = ConfigureFocus::Items;
    page.rows = vec![
        sample_row(
            "profile",
            "default",
            "default",
            "/tmp/shuohua/profile/default.toml",
        ),
        sample_row(
            "profile",
            "coding",
            "coding",
            "/tmp/shuohua/profile/coding.toml",
        ),
    ];
    page.module = ConfigureModule::Profile;
    page.selected = 0;

    page.move_selection(1);
    assert_eq!(page.module, ConfigureModule::Profile);
    assert_eq!(page.selected, 1);
}

#[test]
fn llm_wizard_starts_with_template_defaults_and_allows_text_j() {
    let mut page = ConfigurePage::new();
    page.start_wizard();

    let wizard = page.llm_wizard.as_ref().unwrap();
    assert_eq!(wizard.step, LlmWizardStep::Template);
    assert_eq!(wizard.draft.format, "openai");

    page.advance_wizard();
    page.llm_wizard.as_mut().unwrap().draft.file_id.clear();
    page.edit_wizard_field(WizardEdit::Push('j'));
    page.edit_wizard_field(WizardEdit::Push('1'));

    let wizard = page.llm_wizard.as_ref().unwrap();
    assert_eq!(wizard.step, LlmWizardStep::FileId);
    assert_eq!(wizard.draft.file_id, "j1");
    assert!(!page.wizard_allows_selection());
}

#[test]
fn navigation_shows_module_counts() {
    crate::i18n::init("en-US");
    let mut page = ConfigurePage::new();
    page.rows = vec![SettingsRow {
        group: "asr".to_string(),
        key: "apple.idle_pause".to_string(),
        display_key: "idle_pause".to_string(),
        value: "true".to_string(),
        source: "/tmp/shuohua/asr/apple.toml".to_string(),
        status: InventoryStatus::Ok,
        description_key: Some("config.field.idle_pause.description"),
    }];
    page.module = ConfigureModule::AsrProvider;

    let theme = TuiTheme::default();
    let text = module_nav_lines(&page, &theme)
        .iter()
        .flat_map(|line| line.spans.iter())
        .map(|span| span.content.as_ref())
        .collect::<String>();

    assert!(text.contains("> ASR"));
    assert!(text.contains("1"));
}

#[test]
fn item_list_keeps_source_out_of_dense_rows() {
    crate::i18n::init("en-US");
    let mut page = ConfigurePage::new();
    page.module = ConfigureModule::AsrProvider;
    page.rows = vec![SettingsRow {
        group: "asr".to_string(),
        key: "apple.idle_pause".to_string(),
        display_key: "idle_pause".to_string(),
        value: "true".to_string(),
        source: "/tmp/shuohua/asr/apple.toml".to_string(),
        status: InventoryStatus::Ok,
        description_key: Some("config.field.idle_pause.description"),
    }];

    let theme = TuiTheme::default();
    let text = item_lines(&page, &theme)
        .iter()
        .flat_map(|line| line.spans.iter())
        .map(|span| span.content.as_ref())
        .collect::<String>();

    assert!(text.contains("apple"));
    assert!(!text.contains("apple.idle_pause"));
    assert!(!text.contains("true"));
    assert!(!text.contains("/tmp/shuohua/asr/apple.toml"));
}

#[test]
fn detail_uses_schema_description_and_source() {
    crate::i18n::init("en-US");
    let mut page = ConfigurePage::new();
    page.module = ConfigureModule::AsrProvider;
    page.rows = vec![SettingsRow {
        group: "asr".to_string(),
        key: "apple.idle_pause".to_string(),
        display_key: "idle_pause".to_string(),
        value: "true".to_string(),
        source: "/tmp/shuohua/asr/apple.toml".to_string(),
        status: InventoryStatus::Ok,
        description_key: Some("config.field.idle_pause.description"),
    }];

    let theme = TuiTheme::default();
    let text = detail_lines(&page, &theme)
        .iter()
        .flat_map(|line| line.spans.iter())
        .map(|span| span.content.as_ref())
        .collect::<String>();

    assert!(text.contains("/tmp/shuohua/asr/apple.toml"));
    assert!(text.contains("pause and reopen ASR sessions"));
}

#[test]
fn main_uses_single_field_list() {
    crate::i18n::init("en-US");
    let mut page = ConfigurePage::new();
    page.module = ConfigureModule::Main;
    page.rows = vec![SettingsRow {
        group: "main".to_string(),
        key: "config.hotkey.trigger".to_string(),
        display_key: "hotkey.trigger".to_string(),
        value: "f16".to_string(),
        source: "/tmp/shuohua/config.toml".to_string(),
        status: InventoryStatus::Ok,
        description_key: Some("config.field.hotkey.trigger.description"),
    }];

    let theme = TuiTheme::default();
    let text = item_lines(&page, &theme)
        .iter()
        .flat_map(|line| line.spans.iter())
        .map(|span| span.content.as_ref())
        .collect::<String>();

    assert!(text.contains("hotkey"));
    assert!(text.contains("trigger"));
    assert!(!text.contains("hotkey.trigger"));
    assert!(!text.contains("config.hotkey.trigger"));
    assert!(text.contains("f16"));
    assert!(!text.contains("/tmp/shuohua/config.toml"));
}

#[test]
fn main_groups_fields_by_section() {
    crate::i18n::init("en-US");
    let mut page = ConfigurePage::new();
    page.module = ConfigureModule::Main;
    page.rows = vec![
        SettingsRow {
            group: "main".to_string(),
            key: "config.overlay.position".to_string(),
            display_key: "overlay.position".to_string(),
            value: "bottom".to_string(),
            source: "/tmp/shuohua/config.toml".to_string(),
            status: InventoryStatus::Ok,
            description_key: Some("config.field.overlay.position.description"),
        },
        SettingsRow {
            group: "main".to_string(),
            key: "config.overlay.max_text_lines".to_string(),
            display_key: "overlay.max_text_lines".to_string(),
            value: "5".to_string(),
            source: "/tmp/shuohua/config.toml".to_string(),
            status: InventoryStatus::Ok,
            description_key: Some("config.field.overlay.max_text_lines.description"),
        },
    ];

    let theme = TuiTheme::default();
    let text = main_grouped_lines(&page, &theme)
        .iter()
        .flat_map(|line| line.spans.iter())
        .map(|span| span.content.as_ref())
        .collect::<String>();

    assert!(text.contains("overlay"));
    assert!(text.contains("position"));
    assert!(text.contains("max_text_lines"));
    assert!(!text.contains("overlay.position"));
    assert!(!text.contains("overlay.max_text_lines"));
}

#[test]
fn navigation_renders_all_modules() {
    crate::i18n::init("en-US");
    let page = ConfigurePage::new();
    let theme = TuiTheme::default();
    let text = module_nav_lines(&page, &theme)
        .iter()
        .flat_map(|line| line.spans.iter())
        .map(|span| span.content.as_ref())
        .collect::<String>();

    assert!(text.contains("Main"));
    assert!(text.contains("Profile"));
    assert_eq!(text.matches("Main").count(), 1);
}

#[test]
fn overview_can_render_main_fields() {
    crate::i18n::init("en-US");
    let mut page = ConfigurePage::new();
    page.module = ConfigureModule::Overview;
    page.rows = vec![SettingsRow {
        group: "main".to_string(),
        key: "config.hotkey.trigger".to_string(),
        display_key: "hotkey.trigger".to_string(),
        value: "f16".to_string(),
        source: "/tmp/shuohua/config.toml".to_string(),
        status: InventoryStatus::Ok,
        description_key: Some("config.field.hotkey.trigger.description"),
    }];

    let theme = TuiTheme::default();
    let text = field_lines(
        page.rows.iter().filter(|row| row.group == "main").collect(),
        None,
        &theme,
    )
    .iter()
    .flat_map(|line| line.spans.iter())
    .map(|span| span.content.as_ref())
    .collect::<String>();

    assert!(text.contains("hotkey.trigger"));
    assert!(text.contains("f16"));
}

#[test]
fn profile_list_is_file_selection_and_detail_expands_fields() {
    crate::i18n::init("en-US");
    let mut page = ConfigurePage::new();
    page.module = ConfigureModule::Profile;
    page.rows = vec![
        SettingsRow {
            group: "profile".to_string(),
            key: "default.name".to_string(),
            display_key: "name".to_string(),
            value: "default".to_string(),
            source: "/tmp/shuohua/profile/default.toml".to_string(),
            status: InventoryStatus::Ok,
            description_key: Some("config.field.name.description"),
        },
        SettingsRow {
            group: "profile".to_string(),
            key: "coding.asr.provider".to_string(),
            display_key: "asr.provider".to_string(),
            value: "doubao".to_string(),
            source: "/tmp/shuohua/profile/coding.toml".to_string(),
            status: InventoryStatus::Ok,
            description_key: Some("config.field.asr.provider.description"),
        },
        SettingsRow {
            group: "profile".to_string(),
            key: "coding.post.chain".to_string(),
            display_key: "post.chain".to_string(),
            value: "[\"llm:deepseek\"]".to_string(),
            source: "/tmp/shuohua/profile/coding.toml".to_string(),
            status: InventoryStatus::Ok,
            description_key: Some("config.field.post.chain.description"),
        },
    ];
    page.selected = 0;

    let theme = TuiTheme::default();
    let list = item_lines(&page, &theme)
        .iter()
        .flat_map(|line| line.spans.iter())
        .map(|span| span.content.as_ref())
        .collect::<String>();
    let detail = detail_lines(&page, &theme)
        .iter()
        .flat_map(|line| line.spans.iter())
        .map(|span| span.content.as_ref())
        .collect::<String>();

    assert!(list.contains("coding"));
    assert!(!list.contains("llm:deepseek"));
    assert!(detail.contains("/tmp/shuohua/profile/coding.toml"));
    assert!(detail.contains("asr.provider"));
    assert!(!detail.contains("coding.asr.provider"));
    assert!(detail.contains("llm:deepseek"));
    assert!(detail.contains("Provider name matching"));
    assert!(!detail.contains("reload/status"));
    assert!(!detail.contains("actions"));
}

#[test]
fn detail_preserves_multiline_values() {
    crate::i18n::init("en-US");
    let mut page = ConfigurePage::new();
    page.module = ConfigureModule::PostProcessor;
    page.rows = vec![SettingsRow {
        group: "post".to_string(),
        key: "cleanup.prompt".to_string(),
        display_key: "prompt".to_string(),
        value: "line one\nline two".to_string(),
        source: "/tmp/shuohua/post/llm/cleanup.toml".to_string(),
        status: InventoryStatus::Ok,
        description_key: Some("config.field.prompt.description"),
    }];

    let theme = TuiTheme::default();
    let text = detail_lines(&page, &theme)
        .iter()
        .flat_map(|line| line.spans.iter())
        .map(|span| span.content.as_ref())
        .collect::<Vec<_>>()
        .join("\n");

    assert!(text.contains("line one"));
    assert!(text.contains("line two"));
}
