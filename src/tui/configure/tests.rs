use super::*;
use crate::config::field_view::{ControlKind, FieldOrigin};
use crate::config::TestConfigHome as ConfigHomeGuard;
use crate::tui::configure::render::*;
use crossterm::event::KeyCode;

struct TestConfig {
    dir: std::path::PathBuf,
}

impl TestConfig {
    fn new() -> Self {
        let base =
            std::env::temp_dir().join(format!("shuohua-cfg-edit-{}", ulid::Ulid::generate()));
        let dir = base.join("shuohua");
        std::fs::create_dir_all(&dir).unwrap();
        Self { dir }
    }

    fn write_main(&self, body: &str) {
        std::fs::write(self.main_path(), body).unwrap();
    }

    fn main_path(&self) -> std::path::PathBuf {
        self.dir.join("config.toml")
    }

    fn write_profile(&self, name: &str, body: &str) -> std::path::PathBuf {
        let path = self.dir.join("profile").join(format!("{name}.toml"));
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, body).unwrap();
        path
    }

    fn write_under(&self, sub: &str, name: &str, body: &str) -> std::path::PathBuf {
        let path = self.dir.join(sub).join(format!("{name}.toml"));
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, body).unwrap();
        path
    }

    fn configure_page(&self) -> super::ConfigurePage {
        // 测试删除真实文件，绝不触碰开发者的 ~/.Trash。
        super::ConfigurePage::new().with_deleter(crate::trash::remove_deleter())
    }
}

fn press(code: crossterm::event::KeyCode) -> crossterm::event::KeyEvent {
    crossterm::event::KeyEvent {
        code,
        modifiers: crossterm::event::KeyModifiers::NONE,
        kind: crossterm::event::KeyEventKind::Press,
        state: crossterm::event::KeyEventState::empty(),
    }
}

fn sample_row(group: &str, field_path: &str, display: &str, source: &str) -> SettingsRow {
    SettingsRow {
        can_unset: true,
        group: group.to_string(),
        field_path: field_path.to_string(),
        display_key: display.to_string(),
        value: "ok".to_string(),
        default_value: String::new(),
        origin: FieldOrigin::Set,
        control: ControlKind::Text,
        editable: false,
        secret: false,
        source: source.to_string(),
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
fn configure_initial_selection_is_left_main_module() {
    let page = ConfigurePage::new();

    assert_eq!(page.module, ConfigureModule::Main);
    assert_eq!(page.focus, ConfigureFocus::Modules);
    assert_eq!(page.selected, 0);
}

#[test]
fn selected_config_source_tracks_current_module_row() {
    let mut page = ConfigurePage::new();
    page.module = ConfigureModule::Main;
    page.rows = vec![
        sample_row("main", "config", "config", "/tmp/shuohua/config.toml"),
        sample_row(
            "asr",
            "local_vad",
            "local_vad",
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
fn jk_moves_selection_within_focused_list() {
    let mut page = ConfigurePage::new();
    page.module = ConfigureModule::Overview;
    page.focus = ConfigureFocus::Modules;

    page.move_selection(1);
    assert_eq!(page.module, ConfigureModule::Profile);

    page.focus = ConfigureFocus::Fields;
    // Two field rows for the same source — j/k navigates fields.
    page.rows = vec![
        sample_row(
            "profile",
            "name",
            "name",
            "/tmp/shuohua/profile/default.toml",
        ),
        sample_row(
            "profile",
            "asr.instance",
            "asr.instance",
            "/tmp/shuohua/profile/default.toml",
        ),
    ];
    page.module = ConfigureModule::Profile;
    page.selected = 0;
    page.selected_source_idx = 0;

    page.move_selection(1);
    assert_eq!(page.module, ConfigureModule::Profile);
    assert_eq!(page.selected, 1);
}

#[test]
fn overview_module_uses_main_rows_for_navigation() {
    // Overview and Main share the same rows; j/k must work on both.
    let mut page = ConfigurePage::new();
    page.module = ConfigureModule::Overview;
    page.focus = ConfigureFocus::Fields;
    page.rows = vec![
        sample_row(
            "main",
            "hotkey.trigger",
            "hotkey.trigger",
            "/tmp/shuohua/config.toml",
        ),
        sample_row(
            "main",
            "voice.auto_paste",
            "voice.auto_paste",
            "/tmp/shuohua/config.toml",
        ),
    ];
    page.selected = 0;
    assert_eq!(page.current_len(), 2, "Overview must see Main rows");
    page.move_selection(1);
    assert_eq!(page.selected, 1);
}

#[test]
fn cycle_source_advances_and_resets_field() {
    let mut page = ConfigurePage::new();
    page.module = ConfigureModule::Profile;
    page.focus = ConfigureFocus::Fields;
    page.rows = vec![
        sample_row(
            "profile",
            "name",
            "name",
            "/tmp/shuohua/profile/default.toml",
        ),
        sample_row(
            "profile",
            "asr.instance",
            "asr.instance",
            "/tmp/shuohua/profile/coding.toml",
        ),
    ];
    page.selected_source_idx = 0;
    page.selected = 0;

    page.cycle_source(1);
    assert_eq!(page.selected_source_idx, 1);
    assert_eq!(
        page.selected, 0,
        "field selection resets after source switch"
    );
}

#[test]
fn llm_create_opens_draft_without_replacing_configure_page() {
    crate::i18n::init("en-US");
    let mut page = ConfigurePage::new();
    page.module = ConfigureModule::PostProcessor;

    let status = page.start_llm_create();

    assert!(status.contains("LLM"));
    assert!(page.draft_active());
    assert!(page.modal.is_none());
    let theme = TuiTheme::default();
    let modules = module_nav_lines(&page, &theme)
        .iter()
        .flat_map(|line| line.spans.iter())
        .map(|span| span.content.as_ref())
        .collect::<String>();
    assert!(modules.contains("Post"));
}

#[test]
fn llm_create_rows_show_localized_labels() {
    crate::i18n::init("en-US");
    let mut page = ConfigurePage::new();
    page.start_llm_create();

    let labels: Vec<String> = page
        .llm_draft()
        .unwrap()
        .rows()
        .iter()
        .map(|r| r.display_key.clone())
        .collect();
    let joined = labels.join(" | ");
    assert!(joined.contains("provider preset"), "{joined}");
    assert!(joined.contains("name"), "{joined}");
    assert!(joined.contains("base URL"), "{joined}");
    assert!(joined.contains("system prompt"), "{joined}");
}

#[test]
fn llm_create_preset_cycles_via_shared_select_editor_and_reseeds() {
    use crossterm::event::{KeyCode, KeyEvent};
    let mut page = ConfigurePage::new();
    page.start_llm_create();

    // preset 是第一行；Enter 打开共享 Select 编辑器
    page.feed_draft_key(KeyEvent::from(KeyCode::Enter));
    assert!(matches!(
        page.editing.as_ref().unwrap().control,
        ControlKind::Select(_)
    ));
    // Right 循环到下一个预设，Enter 提交回 draft 并触发 reseed
    page.feed_edit_key(KeyEvent::from(KeyCode::Right));
    page.feed_edit_key(KeyEvent::from(KeyCode::Enter));

    let form = page.llm_draft().unwrap();
    assert_ne!(form.get("preset"), "openai");
    // 换预设后 base_url 被联动重设（不再是 openai 默认）。
    assert_ne!(form.get("base_url"), "https://api.openai.com/v1");
}

#[test]
fn llm_create_end_to_end_writes_file_and_refreshes_sources() {
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    crate::i18n::init("en-US");
    let cfg = TestConfig::new();
    let _env = ConfigHomeGuard::set(cfg.dir.parent().unwrap());
    let mut page = cfg.configure_page();
    page.module = ConfigureModule::PostProcessor;
    page.start_llm_create();
    {
        let form = page.llm_draft_mut().unwrap();
        // 先选 anthropic 预设（联动出 format=anthropic），再覆盖具体字段。
        form.edit("preset", "anthropic".to_string());
        form.on_changed("preset");
        form.edit("file_id", "team_cleaner".to_string());
        form.edit("name", "team-cleaner".to_string());
        form.edit("base_url", "https://api.anthropic.com".to_string());
        form.edit("model", "claude-test".to_string());
        form.edit("system_prompt", "system cleanup".to_string());
        form.edit("prompt", "input: {{text}}".to_string());
    }

    let outcome = page.feed_draft_key(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL));

    assert!(!outcome.reload_config);
    assert!(outcome.status.unwrap().contains("LLM"));
    assert!(!page.draft_active());
    let path = cfg.dir.join("post/team_cleaner.toml");
    let body = std::fs::read_to_string(&path).unwrap();
    assert!(body.contains("format = \"anthropic\""), "{body}");
    assert!(body.contains("name = \"team-cleaner\""), "{body}");
    assert!(body.contains("system cleanup"), "{body}");
    assert!(body.contains("input: {{text}}"), "{body}");
    assert!(page
        .sources_for_current_module()
        .iter()
        .any(|source| source.ends_with("team_cleaner.toml")));
}

#[test]
fn asr_create_end_to_end_seeds_provider_file_and_refreshes_sources() {
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    crate::i18n::init("en-US");
    let cfg = TestConfig::new();
    let _env = ConfigHomeGuard::set(cfg.dir.parent().unwrap());
    let mut page = cfg.configure_page();
    page.module = ConfigureModule::AsrProvider;

    page.on_key(KeyEvent::from(KeyCode::Char('n')));
    assert!(page.draft_active());
    assert_eq!(page.selected_source_idx, 0);
    {
        let form = page.asr_draft_mut().unwrap();
        form.apply_edit("type", "doubao").unwrap();
        form.apply_edit("file_id", "my_doubao").unwrap();
        // 新建==编辑：secret 在内存 draft 里填完，^S 一次写完整文件。
        form.apply_edit("app_key", "app-test").unwrap();
        form.apply_edit("access_key", "access-test").unwrap();
    }

    let outcome = page.feed_draft_key(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL));

    assert!(!outcome.reload_config);
    assert!(outcome.status.unwrap().contains("ASR"));
    assert!(!page.draft_active());
    let path = cfg.dir.join("asr/my_doubao.toml");
    let body = std::fs::read_to_string(&path).unwrap();
    assert!(body.contains("type = \"doubao\""), "{body}");
    assert!(body.contains("app_key = \"app-test\""), "{body}");
    assert!(body.contains("access_key = \"access-test\""), "{body}");
    assert!(page
        .sources_for_current_module()
        .iter()
        .any(|source| source.ends_with("my_doubao.toml")));
}

#[cfg(unix)]
#[test]
fn asr_create_rejects_dangling_symlink_without_writing_target() {
    use std::os::unix::fs::symlink;

    let root = std::env::temp_dir().join(format!(
        "shuohua-asr-create-symlink-{}",
        ulid::Ulid::generate()
    ));
    let asr_dir = root.join("asr");
    std::fs::create_dir_all(&asr_dir).unwrap();
    let target = root.join("outside.toml");
    symlink(&target, asr_dir.join("blocked.toml")).unwrap();
    let mut draft = AsrDraftDoc::new();
    draft.apply_edit("file_id", "blocked").unwrap();

    let error = draft.commit(&asr_dir).unwrap_err();

    assert!(error.to_string().contains("already exists"), "{error:#}");
    assert!(
        !target.exists(),
        "dangling symlink target must not be created"
    );
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn concurrent_asr_create_allows_exactly_one_writer() {
    let root = std::env::temp_dir().join(format!(
        "shuohua-asr-create-race-{}",
        ulid::Ulid::generate()
    ));
    let asr_dir = root.join("asr");
    let mut draft = AsrDraftDoc::new();
    draft.apply_edit("file_id", "shared").unwrap();
    let barrier = std::sync::Arc::new(std::sync::Barrier::new(3));
    let handles = (0..2)
        .map(|_| {
            let draft = draft.clone();
            let asr_dir = asr_dir.clone();
            let barrier = std::sync::Arc::clone(&barrier);
            std::thread::spawn(move || {
                barrier.wait();
                draft.commit(&asr_dir)
            })
        })
        .collect::<Vec<_>>();
    barrier.wait();
    let results = handles
        .into_iter()
        .map(|handle| handle.join().unwrap())
        .collect::<Vec<_>>();

    assert_eq!(results.iter().filter(|result| result.is_ok()).count(), 1);
    assert_eq!(results.iter().filter(|result| result.is_err()).count(), 1);
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn asr_create_tencent_seeds_provider_file_and_refreshes_sources() {
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    crate::i18n::init("en-US");
    let cfg = TestConfig::new();
    let _env = ConfigHomeGuard::set(cfg.dir.parent().unwrap());
    let mut page = cfg.configure_page();
    page.module = ConfigureModule::AsrProvider;

    page.on_key(KeyEvent::from(KeyCode::Char('n')));
    assert!(page.draft_active());
    {
        let form = page.asr_draft_mut().unwrap();
        form.apply_edit("type", "tencent").unwrap();
        form.apply_edit("file_id", "tencent").unwrap();
        // 在 draft 里改一个 schema 字段（控件由 field_view 派生）。
        form.apply_edit("engine_model_type", "16k_zh_en").unwrap();
    }

    let outcome = page.feed_draft_key(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL));

    assert!(!outcome.reload_config);
    assert!(outcome.status.unwrap().contains("ASR"));
    assert!(!page.draft_active());
    let path = cfg.dir.join("asr/tencent.toml");
    let body = std::fs::read_to_string(&path).unwrap();
    assert!(body.contains("type = \"tencent\""), "{body}");
    assert!(body.contains("engine_model_type = \"16k_zh_en\""), "{body}");
    assert!(page
        .sources_for_current_module()
        .iter()
        .any(|source| source.ends_with("tencent.toml")));
}

#[test]
fn asr_create_aliyun_seeds_provider_file_and_refreshes_sources() {
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    crate::i18n::init("zh-CN");
    let cfg = TestConfig::new();
    let _env = ConfigHomeGuard::set(cfg.dir.parent().unwrap());
    let mut page = cfg.configure_page();
    page.module = ConfigureModule::AsrProvider;

    page.on_key(KeyEvent::from(KeyCode::Char('n')));
    {
        let form = page.asr_draft_mut().unwrap();
        form.apply_edit("type", "aliyun").unwrap();
        form.apply_edit("file_id", "aliyun").unwrap();
        form.apply_edit("api_key", "sk-test").unwrap();
        form.apply_edit("workspace_id", "workspace-test").unwrap();
    }

    let outcome = page.feed_draft_key(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL));

    assert!(!outcome.reload_config);
    assert!(!page.draft_active());
    let path = cfg.dir.join("asr/aliyun.toml");
    let body = std::fs::read_to_string(&path).unwrap();
    // draft 填完后一次落盘：默认 model/语言来自模板，secret 为用户所填。
    assert!(body.contains("type = \"aliyun\""), "{body}");
    assert!(body.contains("model = \"fun-asr-realtime\""), "{body}");
    assert!(body.contains("language_hints = [\"zh\"]"), "{body}");
    assert!(body.contains("api_key = \"sk-test\""), "{body}");
    assert!(body.contains("workspace_id = \"workspace-test\""), "{body}");
    assert!(body.contains("finalize_timeout_ms = 12000"), "{body}");
    assert!(page
        .sources_for_current_module()
        .iter()
        .any(|source| source.ends_with("aliyun.toml")));
}

#[test]
fn asr_source_tabs_show_asr_new_entry_label() {
    crate::i18n::init("en-US");
    let mut page = ConfigurePage::new();
    page.module = ConfigureModule::AsrProvider;
    let theme = TuiTheme::default();

    let text = source_tabs(&page, &theme)
        .into_iter()
        .map(|(span, _)| span.content.into_owned())
        .collect::<String>();

    assert!(text.contains("+ New ASR"), "{text}");
    assert!(!text.contains("+ New LLM"), "{text}");
}

#[test]
fn delete_post_component_removes_file_and_refreshes() {
    crate::i18n::init("en-US");
    let cfg = TestConfig::new();
    let _env = ConfigHomeGuard::set(cfg.dir.parent().unwrap());
    let mut page = cfg.configure_page();
    page.module = ConfigureModule::PostProcessor;

    let path = cfg.dir.join("post/to_delete.toml");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(
        &path,
        "type = \"llm\"\nname = \"to_delete\"\nbase_url = \"https://a\"\napi_key = \"k\"\nmodel = \"m\"\nprompt = \"{{text}}\"\n",
    )
    .unwrap();
    page.refresh();

    let idx = page
        .sources_for_current_module()
        .iter()
        .position(|s| s.ends_with("to_delete.toml"))
        .expect("component listed as a source");
    page.selected_source_idx = idx;
    // 选中的是 llm 组件 → 可测连通性。
    assert_eq!(
        page.selected_llm_component_id().as_deref(),
        Some("to_delete")
    );

    // x 只是请求删除，先进入待确认，文件还在。
    page.request_delete();
    assert!(page.pending_delete.is_some());
    assert!(path.exists());

    // 非 y 键取消，文件仍在。
    use crossterm::event::{KeyCode, KeyEvent};
    page.resolve_pending_delete(KeyEvent::from(KeyCode::Esc));
    assert!(page.pending_delete.is_none());
    assert!(path.exists());

    // 再次 x → y 确认，才真删。
    page.request_delete();
    let outcome = page
        .resolve_pending_delete(KeyEvent::from(KeyCode::Char('y')))
        .unwrap();
    assert!(outcome.status.unwrap().contains("deleted"));
    assert!(!path.exists());
    assert!(!page
        .sources_for_current_module()
        .iter()
        .any(|s| s.ends_with("to_delete.toml")));
}

#[test]
fn create_profile_draft_writes_file_and_refreshes_sources() {
    crate::i18n::init("en-US");
    let cfg = TestConfig::new();
    let _env = ConfigHomeGuard::set(cfg.dir.parent().unwrap());
    cfg.write_main("[profile]\ndefault = \"default\"\n");
    cfg.write_profile(
        "default",
        "name = \"default\"\n[asr]\ninstance = \"apple\"\n",
    );
    cfg.write_under("asr", "apple", "type = \"apple\"\n");
    let mut page = cfg.configure_page();
    page.module = ConfigureModule::Profile;

    let status = page.start_profile_create();
    assert!(status.contains("profile"));
    let Some(Draft::Profile(form)) = page.draft.as_mut() else {
        panic!("profile create should open a draft");
    };
    form.edit("file_id", "meeting".to_string());
    form.edit("name", "Meeting Notes".to_string());
    form.edit("asr_instance", "apple".to_string());
    let outcome = page.commit_draft();

    assert!(!outcome.reload_config);
    assert!(outcome.status.unwrap().contains("created profile"));
    assert!(page.draft.is_none());
    assert!(cfg.dir.join("profile/meeting.toml").exists());
    assert!(page
        .sources_for_current_module()
        .iter()
        .any(|path| path.ends_with("meeting.toml")));
    assert!(
        page.composer
            .as_ref()
            .is_some_and(|composer| composer.profile_path.ends_with("meeting.toml")),
        "created profile should be ready for chain editing"
    );
}

#[test]
fn profile_draft_asr_instance_opens_select_editor() {
    crate::i18n::init("en-US");
    let cfg = TestConfig::new();
    let _env = ConfigHomeGuard::set(cfg.dir.parent().unwrap());
    cfg.write_under("asr", "apple", "type = \"apple\"\n");
    cfg.write_under("asr", "team", "type = \"doubao\"\n");
    let mut page = cfg.configure_page();
    page.module = ConfigureModule::Profile;
    page.start_profile_create();

    let Some(Draft::Profile(form)) = page.draft.as_mut() else {
        panic!("profile create should open a draft");
    };
    form.selected = 2;
    page.feed_draft_key(press(KeyCode::Enter));

    let edit = page
        .editing
        .as_ref()
        .expect("select row opens inline editor");
    assert_eq!(edit.target, EditTarget::Draft("asr_instance".to_string()));
    match &edit.control {
        ControlKind::Select(opts) => {
            assert_eq!(opts, &vec!["apple".to_string(), "team".to_string()]);
        }
        other => panic!("asr_instance should use Select editor, got {other:?}"),
    }
}

#[test]
fn profile_composer_footer_hints_include_chain_add_without_duplicate_new() {
    crate::i18n::init("en-US");
    let cfg = TestConfig::new();
    let _env = ConfigHomeGuard::set(cfg.dir.parent().unwrap());
    cfg.write_main("[profile]\ndefault = \"default\"\n");
    cfg.write_profile(
        "default",
        "name = \"D\"\n[asr]\ninstance = \"apple\"\n[post]\nchain = []\n",
    );
    cfg.write_under("asr", "apple", "type = \"apple\"\n");
    cfg.write_under(
        "post",
        "cleanup",
        "type = \"llm\"\nname = \"cleanup\"\nbase_url = \"https://x\"\napi_key = \"k\"\nmodel = \"m\"\nprompt = \"{{text}}\"\n",
    );

    let mut page = cfg.configure_page();
    page.module = ConfigureModule::Profile;
    page.focus = ConfigureFocus::Fields;
    page.refresh();

    let hints = page.key_hints();
    let rendered = hints
        .iter()
        .map(|hint| format!("{} {}", hint.keys, crate::i18n::tr(hint.label_key, &[])))
        .collect::<Vec<_>>()
        .join("  ");

    assert!(
        rendered.contains("n new profile"),
        "Profile creation should be explicit: {rendered}"
    );
    assert!(
        rendered.contains("a add chain member"),
        "chain member add hint missing: {rendered}"
    );
    assert_eq!(
        rendered.matches("n new").count(),
        1,
        "new hint should not be duplicated: {rendered}"
    );
}

#[test]
fn profile_composer_footer_distinguishes_member_remove_from_profile_delete() {
    use crate::tui::configure::profile_composer::ComposerRowKind;
    crate::i18n::init("en-US");
    let cfg = TestConfig::new();
    let _env = ConfigHomeGuard::set(cfg.dir.parent().unwrap());
    cfg.write_main("[profile]\ndefault = \"default\"\n");
    cfg.write_profile(
        "default",
        "name = \"D\"\n[asr]\ninstance = \"apple\"\n[post]\nchain = [\"cleanup\"]\n",
    );
    cfg.write_under("asr", "apple", "type = \"apple\"\n");
    cfg.write_under(
        "post",
        "cleanup",
        "type = \"llm\"\nname = \"cleanup\"\nbase_url = \"https://x\"\napi_key = \"k\"\nmodel = \"m\"\nprompt = \"{{text}}\"\n",
    );

    let mut page = cfg.configure_page();
    page.module = ConfigureModule::Profile;
    page.focus = ConfigureFocus::Fields;
    page.refresh();
    let member_idx = page
        .composer
        .as_ref()
        .unwrap()
        .rows()
        .iter()
        .position(|row| matches!(row.kind, ComposerRowKind::ChainMember { .. }))
        .unwrap();
    page.composer.as_mut().unwrap().selected = member_idx;

    let rendered = page
        .key_hints()
        .iter()
        .map(|hint| format!("{} {}", hint.keys, crate::i18n::tr(hint.label_key, &[])))
        .collect::<Vec<_>>()
        .join("  ");

    assert!(
        rendered.contains("x remove chain member"),
        "chain member remove hint missing: {rendered}"
    );
    assert!(
        rendered.contains("Shift-J/K reorder"),
        "chain member reorder hint missing: {rendered}"
    );
    assert!(
        !rendered.contains("x delete profile"),
        "chain member row must not advertise profile delete: {rendered}"
    );
}

#[test]
fn profile_composer_footer_shows_drop_invalid_when_needed() {
    crate::i18n::init("en-US");
    let cfg = TestConfig::new();
    let _env = ConfigHomeGuard::set(cfg.dir.parent().unwrap());
    cfg.write_main("[profile]\ndefault = \"default\"\n");
    cfg.write_profile(
        "default",
        "name = \"D\"\n[asr]\ninstance = \"apple\"\napp_key = \"stale\"\n[post]\nchain = []\n",
    );
    cfg.write_under("asr", "apple", "type = \"apple\"\n");

    let mut page = cfg.configure_page();
    page.module = ConfigureModule::Profile;
    page.focus = ConfigureFocus::Fields;
    page.refresh();

    let rendered = page
        .key_hints()
        .iter()
        .map(|hint| format!("{} {}", hint.keys, crate::i18n::tr(hint.label_key, &[])))
        .collect::<Vec<_>>()
        .join("  ");

    assert!(
        rendered.contains("X drop invalid"),
        "invalid override cleanup hint missing: {rendered}"
    );
}

#[test]
fn profile_composer_edit_writes_override_and_x_removes_member() {
    use crate::tui::configure::profile_composer::ComposerRowKind;
    crate::i18n::init("en-US");
    let cfg = TestConfig::new();
    let _env = ConfigHomeGuard::set(cfg.dir.parent().unwrap());
    cfg.write_main("[profile]\ndefault = \"default\"\n");
    let profile_path = cfg.write_profile(
        "default",
        "name = \"D\"\n[asr]\ninstance = \"doubao\"\n[post]\nchain = [\"deepseek\"]\n",
    );
    cfg.write_under(
        "asr",
        "doubao",
        "type = \"doubao\"\napp_key = \"a\"\naccess_key = \"b\"\n",
    );
    cfg.write_under(
        "post",
        "deepseek",
        "type = \"llm\"\nname = \"deepseek\"\nbase_url = \"https://x\"\napi_key = \"k\"\nmodel = \"deepseek-chat\"\nprompt = \"{{text}}\"\n",
    );

    let mut page = cfg.configure_page();
    page.module = ConfigureModule::Profile;
    // Entering Profile builds a composer (source strip picks the single profile).
    page.refresh();
    page.selected_source_idx = page
        .sources_for_current_module()
        .iter()
        .position(|s| s.ends_with("default.toml"))
        .unwrap();
    page.sync_composer();
    assert!(
        page.composer.is_some(),
        "entering Profile builds a composer"
    );

    // Select the inherited llm `model` override row (origin Default) and edit it.
    let model_idx = page
        .composer
        .as_ref()
        .unwrap()
        .rows()
        .iter()
        .position(
            |r| matches!(&r.kind, ComposerRowKind::LlmOverride { field, .. } if field == "model"),
        )
        .unwrap();
    page.composer.as_mut().unwrap().selected = model_idx;
    assert_eq!(
        page.composer.as_ref().unwrap().rows()[model_idx].row.origin,
        FieldOrigin::Default,
        "model is inherited before override"
    );

    // Enter opens the inline editor (Text) targeting the composer.
    page.focus = ConfigureFocus::Fields;
    page.on_key(press(KeyCode::Enter));
    assert!(page.is_editing(), "Enter opens composer field editor");
    page.set_buffer("deepseek-v4");
    let outcome = page.commit_edit();
    assert!(!outcome.reload_config);

    // Override written to the profile file and re-resolves as Set.
    let body = std::fs::read_to_string(&profile_path).unwrap();
    assert!(body.contains("deepseek-v4"), "override written: {body}");
    let model_idx = page
        .composer
        .as_ref()
        .unwrap()
        .rows()
        .iter()
        .position(
            |r| matches!(&r.kind, ComposerRowKind::LlmOverride { field, .. } if field == "model"),
        )
        .unwrap();
    let model_row = &page.composer.as_ref().unwrap().rows()[model_idx];
    assert_eq!(model_row.row.origin, FieldOrigin::Set);
    assert_eq!(model_row.row.value, "deepseek-v4");

    // `x` on the chain member removes it from the chain.
    let member_idx = page
        .composer
        .as_ref()
        .unwrap()
        .rows()
        .iter()
        .position(
            |r| matches!(&r.kind, ComposerRowKind::ChainMember { id, .. } if id == "deepseek"),
        )
        .unwrap();
    page.composer.as_mut().unwrap().selected = member_idx;
    page.on_key(press(KeyCode::Char('x')));
    let body = std::fs::read_to_string(&profile_path).unwrap();
    assert!(
        !body.contains("deepseek"),
        "member removed from chain: {body}"
    );
}

#[test]
fn create_profile_draft_shows_error_without_overwriting_existing_file() {
    crate::i18n::init("en-US");
    let cfg = TestConfig::new();
    let _env = ConfigHomeGuard::set(cfg.dir.parent().unwrap());
    cfg.write_main("[profile]\ndefault = \"default\"\n");
    let path = cfg.write_profile("default", "keep me");
    cfg.write_under("asr", "apple", "type = \"apple\"\n");
    let mut page = cfg.configure_page();
    page.module = ConfigureModule::Profile;

    page.start_profile_create();
    let Some(Draft::Profile(form)) = page.draft.as_mut() else {
        panic!("profile create should open a draft");
    };
    form.edit("file_id", "default".to_string());
    form.edit("name", "Default".to_string());
    form.edit("asr_instance", "apple".to_string());
    let outcome = page.commit_draft();

    assert!(outcome.status.is_none());
    assert!(page
        .edit_error
        .as_ref()
        .unwrap()
        .message
        .contains("already exists"));
    assert_eq!(std::fs::read_to_string(path).unwrap(), "keep me");
}

#[test]
fn delete_profile_removes_file_and_refreshes_sources() {
    crate::i18n::init("en-US");
    let cfg = TestConfig::new();
    let _env = ConfigHomeGuard::set(cfg.dir.parent().unwrap());
    cfg.write_main("[profile]\ndefault = \"default\"\n");
    cfg.write_profile(
        "default",
        "name = \"default\"\n[asr]\ninstance = \"apple\"\n",
    );
    let path = cfg.write_profile(
        "meeting",
        "name = \"Meeting\"\n[asr]\ninstance = \"apple\"\n",
    );
    let mut page = cfg.configure_page();
    page.module = ConfigureModule::Profile;
    page.selected_source_idx = page
        .sources_for_current_module()
        .iter()
        .position(|source| source.ends_with("meeting.toml"))
        .unwrap();

    let outcome = page.delete_selected_profile();

    assert!(!outcome.reload_config);
    assert!(outcome.status.unwrap().contains("deleted profile"));
    assert!(!path.exists());
    assert!(!page
        .sources_for_current_module()
        .iter()
        .any(|source| source.ends_with("meeting.toml")));
}

#[test]
fn delete_referenced_profile_sets_error_popup() {
    crate::i18n::init("en-US");
    let cfg = TestConfig::new();
    let _env = ConfigHomeGuard::set(cfg.dir.parent().unwrap());
    cfg.write_main("[profile]\ndefault = \"default\"\n");
    let path = cfg.write_profile(
        "default",
        "name = \"default\"\n[asr]\ninstance = \"apple\"\n",
    );
    let mut page = cfg.configure_page();
    page.module = ConfigureModule::Profile;

    let outcome = page.delete_selected_profile();

    assert!(outcome.status.is_none());
    assert!(page
        .edit_error
        .as_ref()
        .unwrap()
        .message
        .contains("default profile"));
    assert!(path.exists());
}

#[test]
fn detail_scroll_clamps_to_recorded_max() {
    let mut page = ConfigurePage::new();
    page.detail_max_scroll.set(10);
    page.scroll_detail(5);
    assert_eq!(page.detail_scroll, 5);
    page.scroll_detail(100);
    assert_eq!(page.detail_scroll, 10, "must not scroll past content");
    page.scroll_detail(-100);
    assert_eq!(page.detail_scroll, 0, "must not scroll above the top");
}

#[test]
fn detail_pane_shows_full_untruncated_key() {
    crate::i18n::init("en-US");
    // 27 chars — longer than the 22-char list column, so only the detail pane
    // can show it in full.
    let long_key = "vad.min_silence_start_voice";
    let mut page = ConfigurePage::new();
    page.rows = vec![SettingsRow {
        can_unset: true,
        group: "asr".to_string(),
        field_path: long_key.to_string(),
        display_key: long_key.to_string(),
        value: "1200".to_string(),
        default_value: "800".to_string(),
        origin: FieldOrigin::Set,
        control: ControlKind::Number {
            min: None,
            max: None,
            float: false,
        },
        editable: true,
        secret: false,
        source: "/tmp/shuohua/asr/apple.toml".to_string(),
        description_key: None,
    }];
    page.module = ConfigureModule::AsrProvider;
    page.selected_source_idx = 0;
    page.selected = 0;
    page.focus = ConfigureFocus::Fields;

    let theme = TuiTheme::default();
    let text = detail_lines(&page, &theme, 60)
        .iter()
        .flat_map(|line| line.spans.iter())
        .map(|span| span.content.as_ref())
        .collect::<String>();

    assert!(
        text.contains(long_key),
        "detail must show the full key: {text}"
    );
    assert!(text.contains("800"), "detail must show the default: {text}");
}

#[test]
fn detail_pane_preserves_multiline_value_newlines() {
    crate::i18n::init("en-US");
    let mut page = ConfigurePage::new();
    page.rows = vec![SettingsRow {
        can_unset: true,
        group: "post".to_string(),
        field_path: "system_prompt".to_string(),
        display_key: "system_prompt".to_string(),
        value: "First line.\nSecond line.".to_string(),
        default_value: String::new(),
        origin: FieldOrigin::Set,
        control: ControlKind::MultilineText,
        editable: true,
        secret: false,
        source: "/tmp/shuohua/post/x.toml".to_string(),
        description_key: None,
    }];
    page.module = ConfigureModule::PostProcessor;
    page.selected_source_idx = 0;
    page.selected = 0;
    page.focus = ConfigureFocus::Fields;

    let theme = TuiTheme::default();
    // Wide enough that neither line would wrap — so two lines only appear if the
    // newline is preserved (not collapsed into one).
    let per_line: Vec<String> = detail_lines(&page, &theme, 80)
        .iter()
        .map(|line| {
            line.spans
                .iter()
                .map(|span| span.content.as_ref())
                .collect::<String>()
        })
        .collect();

    assert!(
        per_line
            .iter()
            .any(|l| l.contains("First line.") && !l.contains("Second line.")),
        "first prompt line stands alone: {per_line:?}"
    );
    assert!(
        per_line.iter().any(|l| l.contains("Second line.")),
        "second prompt line preserved on its own row: {per_line:?}"
    );
}

#[test]
fn navigation_marks_only_modules_with_problems() {
    crate::i18n::init("en-US");
    let mut page = ConfigurePage::new();
    page.module = ConfigureModule::AsrProvider;
    page.focus = ConfigureFocus::Fields;
    // This render test owns its module problem inputs; do not inherit counts
    // from whichever XDG_CONFIG_HOME another parallel test or CI image exposes.
    page.overview_counts = Vec::new();
    let theme = TuiTheme::default();

    let render = |page: &ConfigurePage| {
        module_nav_lines(page, &theme)
            .iter()
            .flat_map(|line| line.spans.iter())
            .map(|span| span.content.as_ref())
            .collect::<String>()
    };

    // No problems: the module shows its title but no raw count or marker.
    let clean = render(&page);
    assert!(clean.contains("* ASR"), "{clean}");
    assert!(
        !clean.contains('●'),
        "clean module must not show a marker: {clean}"
    );

    // A validation error surfaces a red problem marker.
    page.overview_counts = vec![("asr".to_string(), 1, 0)];
    let flagged = render(&page);
    assert!(
        flagged.contains('●'),
        "flagged module must show a marker: {flagged}"
    );
}

#[test]
fn item_list_keeps_source_out_of_dense_rows() {
    // source_lines (the live source-list renderer used by render_page for non-Main modules)
    // must show the file stem ("apple") but NOT the full path or any field values —
    // those belong in the field-area panel, not the source-list band.
    crate::i18n::init("en-US");
    let mut page = ConfigurePage::new();
    page.module = ConfigureModule::AsrProvider;
    page.rows = vec![SettingsRow {
        can_unset: true,
        group: "asr".to_string(),
        field_path: "local_vad".to_string(),
        display_key: "local_vad".to_string(),
        value: "on".to_string(),
        default_value: String::new(),
        origin: FieldOrigin::Set,
        control: ControlKind::Select(vec!["auto".into(), "on".into(), "off".into()]),
        editable: false,
        secret: false,
        source: "/tmp/shuohua/asr/apple.toml".to_string(),
        description_key: Some("config.field.local_vad.description"),
    }];

    let theme = TuiTheme::default();
    let text = source_lines(&page, &theme)
        .iter()
        .flat_map(|line| line.spans.iter())
        .map(|span| span.content.as_ref())
        .collect::<String>();

    assert!(text.contains("apple"));
    assert!(!text.contains("apple.local_vad"));
    assert!(!text.contains("on"));
    assert!(!text.contains("/tmp/shuohua/asr/apple.toml"));
}

#[test]
fn detail_uses_schema_description_and_source() {
    // In the new layout, non-Main modules render a source list (source_lines) and a field area.
    // The source list shows the file stem; the field rows carry the schema description.
    // Assert: source list shows file stem "apple", field rows include description text.
    crate::i18n::init("en-US");
    let mut page = ConfigurePage::new();
    page.module = ConfigureModule::AsrProvider;
    page.selected = 0;
    page.rows = vec![SettingsRow {
        can_unset: true,
        group: "asr".to_string(),
        field_path: "local_vad".to_string(),
        display_key: "local_vad".to_string(),
        value: "on".to_string(),
        default_value: String::new(),
        origin: FieldOrigin::Set,
        control: ControlKind::Select(vec!["auto".into(), "on".into(), "off".into()]),
        editable: false,
        secret: false,
        source: "/tmp/shuohua/asr/apple.toml".to_string(),
        description_key: Some("config.field.local_vad.description"),
    }];

    let theme = TuiTheme::default();

    // Source list shows the file stem, not the full path.
    let source_text = source_lines(&page, &theme)
        .iter()
        .flat_map(|line| line.spans.iter())
        .map(|span| span.content.as_ref())
        .collect::<String>();
    assert!(source_text.contains("apple"));
    assert!(!source_text.contains("/tmp/shuohua/asr/apple.toml"));

    // Field rows for the selected source include the schema description.
    let rows = selected_source_rows(&page);
    let field_text = field_lines_with_edit(rows, &page, &theme)
        .iter()
        .flat_map(|line| line.spans.iter())
        .map(|span| span.content.as_ref())
        .collect::<String>();
    assert!(field_text.contains("shuohua local VAD"));
}

#[test]
fn main_uses_single_field_list() {
    // main_grouped_lines is the live renderer used by render_page for the Main module.
    // It splits display_key on '.' to render a section header + sub-key — so "hotkey.trigger"
    // becomes section "hotkey" + item key "trigger". The full dot-joined key must NOT appear,
    // nor should the source path.
    crate::i18n::init("en-US");
    let mut page = ConfigurePage::new();
    page.module = ConfigureModule::Main;
    page.rows = vec![SettingsRow {
        can_unset: true,
        group: "main".to_string(),
        field_path: "hotkey.trigger".to_string(),
        display_key: "hotkey.trigger".to_string(),
        value: "f16".to_string(),
        default_value: String::new(),
        origin: FieldOrigin::Set,
        control: ControlKind::Text,
        editable: true,
        secret: false,
        source: "/tmp/shuohua/config.toml".to_string(),
        description_key: Some("config.field.hotkey.trigger.description"),
    }];

    let theme = TuiTheme::default();
    let text = main_grouped_lines(&page, &theme)
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
fn main_selected_row_shows_cursor_marker() {
    // main_grouped_lines must render "▶ " on the Nth field row when page.selected == N.
    // Section headers are not counted — only field rows increment the index.
    crate::i18n::init("en-US");
    let mut page = ConfigurePage::new();
    page.module = ConfigureModule::Main;
    page.rows = vec![
        SettingsRow {
            can_unset: true,
            group: "main".to_string(),
            field_path: "hotkey.trigger".to_string(),
            display_key: "hotkey.trigger".to_string(),
            value: "f16".to_string(),
            default_value: String::new(),
            origin: FieldOrigin::Set,
            control: ControlKind::Text,
            editable: true,
            secret: false,
            source: "/tmp/shuohua/config.toml".to_string(),
            description_key: None,
        },
        SettingsRow {
            can_unset: true,
            group: "main".to_string(),
            field_path: "overlay.position".to_string(),
            display_key: "overlay.position".to_string(),
            value: "bottom".to_string(),
            default_value: String::new(),
            origin: FieldOrigin::Default,
            control: ControlKind::Select(vec!["top".to_string(), "bottom".to_string()]),
            editable: true,
            secret: false,
            source: "/tmp/shuohua/config.toml".to_string(),
            description_key: None,
        },
    ];

    let theme = TuiTheme::default();

    // selected = 0: first field row gets the marker, second does not.
    page.selected = 0;
    let lines = main_grouped_lines(&page, &theme);
    // Find lines that contain "trigger" (field row 0) and "position" (field row 1).
    let trigger_line_text: String = lines
        .iter()
        .find(|l| l.spans.iter().any(|s| s.content.contains("trigger")))
        .unwrap()
        .spans
        .iter()
        .map(|s| s.content.as_ref())
        .collect();
    let position_line_text: String = lines
        .iter()
        .find(|l| l.spans.iter().any(|s| s.content.contains("position")))
        .unwrap()
        .spans
        .iter()
        .map(|s| s.content.as_ref())
        .collect();
    assert!(
        trigger_line_text.contains('▶'),
        "selected row 0 must show '▶' marker; got: {trigger_line_text:?}"
    );
    assert!(
        !position_line_text.contains('▶'),
        "unselected row 1 must NOT show '▶' marker; got: {position_line_text:?}"
    );

    // selected = 1: second field row gets the marker.
    page.selected = 1;
    let lines = main_grouped_lines(&page, &theme);
    let position_line_text2: String = lines
        .iter()
        .find(|l| l.spans.iter().any(|s| s.content.contains("position")))
        .unwrap()
        .spans
        .iter()
        .map(|s| s.content.as_ref())
        .collect();
    assert!(
        position_line_text2.contains('▶'),
        "selected row 1 must show '▶' marker; got: {position_line_text2:?}"
    );
}

#[test]
fn main_groups_fields_by_section() {
    crate::i18n::init("en-US");
    let mut page = ConfigurePage::new();
    page.module = ConfigureModule::Main;
    page.rows = vec![
        SettingsRow {
            can_unset: true,
            group: "main".to_string(),
            field_path: "overlay.position".to_string(),
            display_key: "overlay.position".to_string(),
            value: "bottom".to_string(),
            default_value: String::new(),
            origin: FieldOrigin::Default,
            control: ControlKind::Select(vec!["top".to_string(), "bottom".to_string()]),
            editable: true,
            secret: false,
            source: "/tmp/shuohua/config.toml".to_string(),
            description_key: Some("config.field.overlay.position.description"),
        },
        SettingsRow {
            can_unset: true,
            group: "main".to_string(),
            field_path: "overlay.max_text_lines".to_string(),
            display_key: "overlay.max_text_lines".to_string(),
            value: "5".to_string(),
            default_value: String::new(),
            origin: FieldOrigin::Default,
            control: ControlKind::Number {
                min: None,
                max: None,
                float: false,
            },
            editable: true,
            secret: false,
            source: "/tmp/shuohua/config.toml".to_string(),
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
    // When module is Overview/Main, render_page routes through main_grouped_lines.
    // Verify it renders the grouped section header + field key + value.
    crate::i18n::init("en-US");
    let mut page = ConfigurePage::new();
    page.module = ConfigureModule::Main;
    page.rows = vec![SettingsRow {
        can_unset: true,
        group: "main".to_string(),
        field_path: "hotkey.trigger".to_string(),
        display_key: "hotkey.trigger".to_string(),
        value: "f16".to_string(),
        default_value: String::new(),
        origin: FieldOrigin::Set,
        control: ControlKind::Text,
        editable: true,
        secret: false,
        source: "/tmp/shuohua/config.toml".to_string(),
        description_key: Some("config.field.hotkey.trigger.description"),
    }];

    let theme = TuiTheme::default();
    let text = main_grouped_lines(&page, &theme)
        .iter()
        .flat_map(|line| line.spans.iter())
        .map(|span| span.content.as_ref())
        .collect::<String>();

    // Section header "hotkey" and sub-key "trigger" appear separately (not as dot-joined key).
    assert!(text.contains("hotkey"));
    assert!(text.contains("trigger"));
    assert!(text.contains("f16"));
}

#[test]
fn profile_list_is_file_selection_and_detail_expands_fields() {
    // source_lines (source-list panel) shows file stems, not field values.
    // field_lines_with_edit over selected_source_rows shows the fields for the selected file.
    // Sources are sorted: coding.toml < default.toml, so selected=0 → coding.toml.
    crate::i18n::init("en-US");
    let mut page = ConfigurePage::new();
    page.module = ConfigureModule::Profile;
    page.rows = vec![
        SettingsRow {
            can_unset: true,
            group: "profile".to_string(),
            field_path: "name".to_string(),
            display_key: "name".to_string(),
            value: "default".to_string(),
            default_value: String::new(),
            origin: FieldOrigin::Set,
            control: ControlKind::Text,
            editable: false,
            secret: false,
            source: "/tmp/shuohua/profile/default.toml".to_string(),
            description_key: Some("config.field.name.description"),
        },
        SettingsRow {
            can_unset: true,
            group: "profile".to_string(),
            field_path: "asr.instance".to_string(),
            display_key: "asr.instance".to_string(),
            value: "doubao".to_string(),
            default_value: String::new(),
            origin: FieldOrigin::Set,
            control: ControlKind::Text,
            editable: false,
            secret: false,
            source: "/tmp/shuohua/profile/coding.toml".to_string(),
            description_key: Some("config.field.asr.instance.description"),
        },
        SettingsRow {
            can_unset: true,
            group: "profile".to_string(),
            field_path: "post.chain".to_string(),
            display_key: "post.chain".to_string(),
            value: "[\"deepseek\"]".to_string(),
            default_value: String::new(),
            origin: FieldOrigin::Set,
            control: ControlKind::ReadOnly,
            editable: false,
            secret: false,
            source: "/tmp/shuohua/profile/coding.toml".to_string(),
            description_key: Some("config.field.post.chain.description"),
        },
    ];
    page.selected = 0; // coding.toml (sorted first)

    let theme = TuiTheme::default();

    // Source list shows file stems; field values must not leak into the source-list band.
    let list = source_lines(&page, &theme)
        .iter()
        .flat_map(|line| line.spans.iter())
        .map(|span| span.content.as_ref())
        .collect::<String>();
    assert!(list.contains("coding"));
    assert!(!list.contains("deepseek"));

    // Field area for the selected source (coding.toml) shows its fields and descriptions.
    // The full source path must not appear in the field rows; field names must not be prefixed
    // with the source stem.
    let rows = selected_source_rows(&page);
    let detail = field_lines_with_edit(rows, &page, &theme)
        .iter()
        .flat_map(|line| line.spans.iter())
        .map(|span| span.content.as_ref())
        .collect::<String>();
    assert!(detail.contains("asr.instance"));
    assert!(!detail.contains("coding.asr.instance"));
    assert!(detail.contains("deepseek"));
    assert!(detail.contains("ASR instance ID"));
    assert!(!detail.contains("/tmp/shuohua/profile/coding.toml"));
    assert!(!detail.contains("reload/status"));
    assert!(!detail.contains("actions"));
}

#[test]
fn detail_preserves_multiline_values() {
    // field_lines_with_edit (the live field-area renderer for non-Main modules) must include
    // multiline values. compact_value collapses newlines to spaces, so both "line one" and
    // "line two" must appear in the output (as space-joined content in the value cell).
    crate::i18n::init("en-US");
    let mut page = ConfigurePage::new();
    page.module = ConfigureModule::PostProcessor;
    page.selected = 0;
    page.rows = vec![SettingsRow {
        can_unset: true,
        group: "post".to_string(),
        field_path: "prompt".to_string(),
        display_key: "prompt".to_string(),
        value: "line one\nline two".to_string(),
        default_value: String::new(),
        origin: FieldOrigin::Set,
        control: ControlKind::Text,
        editable: false,
        secret: false,
        source: "/tmp/shuohua/post/cleanup.toml".to_string(),
        description_key: Some("config.field.prompt.description"),
    }];

    let theme = TuiTheme::default();
    let rows = selected_source_rows(&page);
    let text = field_lines_with_edit(rows, &page, &theme)
        .iter()
        .flat_map(|line| line.spans.iter())
        .map(|span| span.content.as_ref())
        .collect::<String>();

    assert!(text.contains("line one"));
    assert!(text.contains("line two"));
}

#[test]
fn enter_edit_toggles_bool_and_commits() {
    let dir = TestConfig::new();
    dir.write_main("[hotkey]\ntrigger = \"f16\"\n");
    let mut page = dir.configure_page();

    page.begin_edit_for(
        "voice.auto_paste",
        dir.main_path(),
        ControlKind::Toggle,
        "true",
    );
    page.toggle_or_cycle(1);
    let outcome = page.commit_edit();

    assert!(outcome.reload_config, "config.toml edit triggers reload");
    assert!(page.editing.is_none());
    assert!(page.edit_error.is_none());
    let body = std::fs::read_to_string(dir.main_path()).unwrap();
    assert!(body.contains("auto_paste = false"), "{body}");
}

#[test]
fn edit_profile_file_writes_without_reload() {
    let dir = TestConfig::new();
    let path = dir.write_profile(
        "default",
        "name = \"default\"\n[asr]\ninstance = \"doubao\"\n",
    );
    let mut page = dir.configure_page();

    page.begin_edit_for("asr.instance", path.clone(), ControlKind::Text, "doubao");
    page.set_buffer("apple");
    let outcome = page.commit_edit();

    assert!(
        !outcome.reload_config,
        "non-main file edit must not request reload"
    );
    assert_eq!(
        outcome.status.as_deref(),
        Some(crate::i18n::tr("tui.configure.edit.saved_local", &[]).as_str())
    );
    let body = std::fs::read_to_string(&path).unwrap();
    assert!(body.contains("instance = \"apple\""), "{body}");
}

#[test]
fn reset_profile_field_removes_key_without_reload() {
    let dir = TestConfig::new();
    let path = dir.write_profile(
        "default",
        "name = \"default\"\n[asr]\ninstance = \"doubao\"\nhotwords = [\"foo\"]\n",
    );
    let mut page = dir.configure_page();

    let outcome = page.reset_field_to_default("asr.hotwords", path.clone());

    assert!(!outcome.reload_config, "non-main reset must not reload");
    let body = std::fs::read_to_string(&path).unwrap();
    assert!(!body.contains("hotwords"), "key removed: {body}");
}

#[test]
fn invalid_number_keeps_value_and_sets_error() {
    let dir = TestConfig::new();
    dir.write_main("[hotkey]\ntrigger = \"f16\"\n");
    let before = std::fs::read_to_string(dir.main_path()).unwrap();
    let mut page = dir.configure_page();

    page.begin_edit_for(
        "voice.vad.threshold",
        dir.main_path(),
        ControlKind::Number {
            min: Some(0.0),
            max: Some(1.0),
            float: true,
        },
        "0.5",
    );
    page.set_buffer("1.5");
    let outcome = page.commit_edit();

    assert!(!outcome.reload_config);
    assert!(page.edit_error.is_some(), "error popup set");
    assert_eq!(
        std::fs::read_to_string(dir.main_path()).unwrap(),
        before,
        "file untouched"
    );
}

#[test]
fn enter_key_starts_editing_selected_editable_row() {
    use crate::tui::page::Page;
    use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};

    let mut page = super::ConfigurePage::new();
    page.module = super::ConfigureModule::Main;
    page.focus = super::ConfigureFocus::Fields;
    page.selected = 0;
    page.rows = vec![SettingsRow {
        can_unset: true,
        group: "main".to_string(),
        field_path: "voice.auto_paste".to_string(),
        display_key: "voice.auto_paste".to_string(),
        value: "true".to_string(),
        default_value: String::new(),
        origin: FieldOrigin::Default,
        control: ControlKind::Toggle,
        editable: true,
        secret: false,
        source: "/tmp/shuohua/config.toml".to_string(),
        description_key: None,
    }];

    let _ = page.on_key(KeyEvent {
        code: KeyCode::Enter,
        modifiers: KeyModifiers::empty(),
        kind: KeyEventKind::Press,
        state: crossterm::event::KeyEventState::empty(),
    });
    assert!(page.is_editing(), "Enter on editable row enters edit mode");
}

#[test]
fn enter_key_on_non_editable_row_returns_status_not_edit() {
    use crate::tui::page::Page;
    use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};

    let mut page = super::ConfigurePage::new();
    page.module = super::ConfigureModule::Main;
    page.focus = super::ConfigureFocus::Fields;
    page.selected = 0;
    page.rows = vec![SettingsRow {
        can_unset: true,
        group: "main".to_string(),
        field_path: "voice.auto_paste".to_string(),
        display_key: "voice.auto_paste".to_string(),
        value: "true".to_string(),
        default_value: String::new(),
        origin: FieldOrigin::Default,
        control: ControlKind::Toggle,
        editable: false,
        secret: false,
        source: "/tmp/shuohua/config.toml".to_string(),
        description_key: None,
    }];

    let outcome = page.on_key(KeyEvent {
        code: KeyCode::Enter,
        modifiers: KeyModifiers::empty(),
        kind: KeyEventKind::Press,
        state: crossterm::event::KeyEventState::empty(),
    });
    assert!(
        !page.is_editing(),
        "Enter on non-editable row must not enter edit mode"
    );
    assert!(
        outcome.status.is_some(),
        "non-editable Enter must return a status message"
    );
}

#[test]
fn d_key_resets_set_field_and_returns_reload_command() {
    use crate::tui::page::Page;
    use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};

    let dir = TestConfig::new();
    dir.write_main("[hotkey]\ntrigger = \"f16\"\n[overlay]\nposition = \"top\"\n");

    let mut page = super::ConfigurePage::new();
    page.module = super::ConfigureModule::Main;
    page.focus = super::ConfigureFocus::Fields;
    page.selected = 0;
    page.rows = vec![SettingsRow {
        can_unset: true,
        group: "main".to_string(),
        field_path: "overlay.position".to_string(),
        display_key: "overlay.position".to_string(),
        value: "top".to_string(),
        default_value: String::new(),
        origin: FieldOrigin::Set,
        control: ControlKind::Select(vec!["top".into(), "middle".into(), "bottom".into()]),
        editable: true,
        secret: false,
        source: dir.main_path().to_string_lossy().into_owned(),
        description_key: None,
    }];

    let outcome = page.on_key(KeyEvent {
        code: KeyCode::Char('D'),
        modifiers: KeyModifiers::SHIFT,
        kind: KeyEventKind::Press,
        state: crossterm::event::KeyEventState::empty(),
    });

    assert_eq!(
        outcome.command,
        Some(crate::ipc::protocol::Command::ReloadConfig),
        "D on Set editable row must return ReloadConfig command"
    );

    let body = std::fs::read_to_string(dir.main_path()).unwrap();
    assert!(
        !body.contains("position"),
        "overlay.position must be removed from config file; got: {body}"
    );

    let _ = std::fs::remove_dir_all(dir.dir.parent().unwrap_or(&dir.dir));
}

#[test]
fn modal_multiline_saves_full_value_without_reload() {
    let dir = TestConfig::new();
    let path = dir.dir.join("post/x.toml");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(
        &path,
        "type = \"llm\"\nname = \"x\"\nbase_url = \"https://a\"\napi_key = \"k\"\nmodel = \"m\"\nprompt = \"old\"\n",
    )
    .unwrap();
    let mut page = dir.configure_page();
    page.modal = Some(super::modal::ModalEditor::new(
        "prompt".into(),
        super::modal::EditTarget::File(path.clone()),
        super::modal::ModalKind::Multiline,
        "old".into(),
    ));
    if let Some(m) = &mut page.modal {
        m.buffer = "new\nline".into();
    }
    let outcome = page.commit_modal();
    assert!(!outcome.reload_config);
    assert!(page.modal.is_none());
    let body = std::fs::read_to_string(&path).unwrap();
    // toml may encode the newline literally in a multi-line/basic string; accept either
    assert!(
        body.contains("new\nline") || body.contains("new\\nline"),
        "{body}"
    );
}

#[test]
fn modal_array_saves_lines_as_string_array() {
    let dir = TestConfig::new();
    let path = dir.write_profile(
        "default",
        "name = \"default\"\n[asr]\ninstance = \"apple\"\nhotwords = [\"old\"]\n",
    );
    let mut page = dir.configure_page();
    page.modal = Some(super::modal::ModalEditor::new(
        "asr.hotwords".into(),
        super::modal::EditTarget::File(path.clone()),
        super::modal::ModalKind::Array,
        "Rust\ntokio".into(),
    ));

    let outcome = page.commit_modal();

    assert!(!outcome.reload_config);
    assert!(page.modal.is_none());
    let body = std::fs::read_to_string(&path).unwrap();
    assert!(body.contains("hotwords = [\"Rust\", \"tokio\"]"), "{body}");
}

#[test]
fn aliyun_saved_language_select_writes_single_hint_or_auto_array() {
    let dir = TestConfig::new();
    let path = dir.write_under(
        "asr",
        "aliyun",
        "type = \"aliyun\"\napi_key = \"sk-test\"\nworkspace_id = \"workspace-test\"\nlanguage_hints = [\"zh\"]\n",
    );
    let mut page = dir.configure_page();

    page.begin_edit_for(
        "language_hints",
        path.clone(),
        ControlKind::Select(vec!["zh".into(), "en".into(), "auto".into()]),
        "zh",
    );
    page.set_buffer("en");
    let outcome = page.commit_edit();
    assert!(!outcome.reload_config);
    let body = std::fs::read_to_string(&path).unwrap();
    assert!(body.contains("language_hints = [\"en\"]"), "{body}");

    page.begin_edit_for(
        "language_hints",
        path.clone(),
        ControlKind::Select(vec!["zh".into(), "en".into(), "auto".into()]),
        "en",
    );
    page.set_buffer("auto");
    let outcome = page.commit_edit();
    assert!(!outcome.reload_config);
    let body = std::fs::read_to_string(&path).unwrap();
    assert!(body.contains("language_hints = []"), "{body}");
}

#[test]
fn aliyun_saved_model_edit_writes_value_without_linked_reconciliation() {
    let dir = TestConfig::new();
    let path = dir.write_under(
        "asr",
        "aliyun",
        "type = \"aliyun\"\napi_key = \"sk-test\"\nworkspace_id = \"workspace-test\"\nmodel = \"fun-asr-realtime\"\nlanguage_hints = [\"vi\"]\nspeech_noise_threshold = 0.2\n",
    );
    let mut page = dir.configure_page();

    page.begin_edit_for(
        "model",
        path.clone(),
        ControlKind::EditableSelect(vec!["fun-asr-realtime".into()]),
        "fun-asr-realtime",
    );
    page.set_buffer("my-custom-model");
    let outcome = page.commit_edit();

    // 新建==编辑后 model 走普通 set_field：只写 model 值，不再联动改盘其它字段。
    assert!(!outcome.reload_config);
    let body = std::fs::read_to_string(&path).unwrap();
    assert!(body.contains("model = \"my-custom-model\""), "{body}");
    assert!(body.contains("language_hints = [\"vi\"]"), "{body}");
    assert!(body.contains("speech_noise_threshold = 0.2"), "{body}");
}

#[test]
fn aliyun_saved_model_editable_preset_still_accepts_custom_text() {
    let dir = TestConfig::new();
    let path = dir.write_under(
        "asr",
        "aliyun-custom",
        "type = \"aliyun\"\napi_key = \"sk-test\"\nworkspace_id = \"workspace-test\"\nmodel = \"old-custom-model\"\nmodel_family = \"fun\"\nlanguage_hints = [\"zh\"]\n",
    );
    let mut page = dir.configure_page();

    page.begin_edit_for(
        "model",
        path.clone(),
        ControlKind::EditableSelect(vec![
            "fun-asr-realtime".into(),
            "paraformer-realtime-v2".into(),
        ]),
        "old-custom-model",
    );
    page.set_buffer("");
    page.feed_edit_paste("new-custom-model");
    let outcome = page.commit_edit();

    assert!(!outcome.reload_config);
    let body = std::fs::read_to_string(&path).unwrap();
    assert!(body.contains("model = \"new-custom-model\""), "{body}");
    assert!(body.contains("model_family = \"fun\""), "{body}");
}

#[test]
fn modal_paste_inserts_at_cursor() {
    let dir = TestConfig::new();
    let mut page = dir.configure_page();
    page.modal = Some(super::modal::ModalEditor::new(
        "prompt".into(),
        super::modal::EditTarget::File(dir.main_path()),
        super::modal::ModalKind::Multiline,
        "ac".into(),
    ));
    page.modal.as_mut().unwrap().move_left();

    page.feed_modal_paste("b");

    assert_eq!(page.modal.as_ref().unwrap().buffer, "abc");
}

#[test]
fn modal_secret_blank_does_not_overwrite() {
    let dir = TestConfig::new();
    let path = dir.dir.join("post/x.toml");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(
        &path,
        "type = \"llm\"\nname = \"x\"\nbase_url = \"https://a\"\napi_key = \"keep-me\"\nmodel = \"m\"\nprompt = \"p\"\n",
    )
    .unwrap();
    let mut page = dir.configure_page();
    page.modal = Some(super::modal::ModalEditor::new(
        "api_key".into(),
        super::modal::EditTarget::File(path.clone()),
        super::modal::ModalKind::Secret,
        String::new(),
    ));
    let outcome = page.commit_modal();
    assert!(page.modal.is_none());
    assert_eq!(
        outcome.status.as_deref(),
        Some(crate::i18n::tr("tui.configure.edit.unchanged", &[]).as_str())
    );
    let body = std::fs::read_to_string(&path).unwrap();
    assert!(body.contains("keep-me"), "secret untouched: {body}");
}

#[test]
fn edit_i18n_keys_exist_in_base_locales() {
    for key in [
        "tui.configure.edit.saved",
        "tui.configure.edit.saved_local",
        "tui.configure.sources",
        "tui.configure.edit.reset",
        "tui.configure.edit.not_editable",
        "tui.configure.edit.invalid",
        "tui.configure.edit.invalid_title",
        "tui.configure.edit.dismiss",
        "tui.configure.edit.not_integer",
        "tui.configure.edit.not_number",
        "tui.configure.edit.hint_toggle",
        "tui.configure.edit.hint_select",
        "tui.configure.edit.hint_number",
        "tui.configure.edit.hint_text",
        "tui.configure.edit.unchanged",
        "tui.configure.origin.set",
        "tui.configure.origin.default",
        "tui.configure.origin.required",
        "tui.configure.composer.section_asr",
        "tui.configure.composer.section_chain",
        "tui.configure.composer.section_dangling",
        "tui.configure.composer.picker_title",
        "tui.configure.composer.picker_empty",
        "tui.hint.reorder",
        "tui.hint.drop_invalid",
    ] {
        assert_ne!(
            crate::i18n::tr_lang(crate::i18n::Lang::EnUS, key, &[]),
            key,
            "missing en {key}"
        );
        assert_ne!(
            crate::i18n::tr_lang(crate::i18n::Lang::ZhCN, key, &[]),
            key,
            "missing zh {key}"
        );
    }
}

#[test]
fn l_enters_content_and_esc_returns_to_modules() {
    // 两级焦点：l/Enter 进入内容区，Esc 统一返回模块列表（进出不再对调轴向）。
    let mut page = ConfigurePage::new();
    page.module = ConfigureModule::Profile;
    page.focus = ConfigureFocus::Modules;
    page.on_key(press(KeyCode::Char('l')));
    assert_eq!(page.focus, ConfigureFocus::Fields, "l: Modules->Fields");
    page.on_key(press(KeyCode::Esc));
    assert_eq!(page.focus, ConfigureFocus::Modules, "Esc: Fields->Modules");
    // Enter 也能进入内容区。
    page.on_key(press(KeyCode::Enter));
    assert_eq!(page.focus, ConfigureFocus::Fields, "Enter: Modules->Fields");
}

#[test]
fn content_hl_cycles_source_and_resets_field() {
    // 内容区里 h/l 横向切来源；换来源重置字段选择。
    let mut page = ConfigurePage::new();
    page.module = ConfigureModule::Profile;
    page.focus = ConfigureFocus::Fields;
    page.rows = vec![
        sample_row("profile", "name", "name", "/tmp/shuohua/profile/a.toml"),
        sample_row("profile", "name", "name", "/tmp/shuohua/profile/b.toml"),
    ];
    page.selected = 1;
    page.on_key(press(KeyCode::Char('l')));
    assert_eq!(page.selected_source_idx, 1);
    assert_eq!(page.selected, 0, "switching source resets field selection");
    page.on_key(press(KeyCode::Char('h')));
    assert_eq!(page.selected_source_idx, 0, "h cycles source back");
}

#[test]
fn esc_cancels_without_writing() {
    let dir = TestConfig::new();
    dir.write_main("[hotkey]\ntrigger = \"f16\"\n");
    let before = std::fs::read_to_string(dir.main_path()).unwrap();
    let mut page = dir.configure_page();

    page.begin_edit_for(
        "overlay.position",
        dir.main_path(),
        ControlKind::Select(vec!["top".into(), "bottom".into()]),
        "bottom",
    );
    page.toggle_or_cycle(1);
    page.cancel_edit();

    assert!(page.editing.is_none());
    assert_eq!(std::fs::read_to_string(dir.main_path()).unwrap(), before);
}

#[test]
fn draft_esc_restores_pre_draft_source() {
    // Esc 取消新建应还原进入前的来源选择，而不是切到最后一个来源。
    let mut page = ConfigurePage::new();
    page.module = ConfigureModule::PostProcessor;
    page.rows = vec![
        sample_row("post", "model", "model", "/tmp/shuohua/post/a.toml"),
        sample_row("post", "model", "model", "/tmp/shuohua/post/b.toml"),
    ];
    page.selected_source_idx = 1;
    page.start_llm_create();
    assert_eq!(page.selected_source_idx, 2, "draft sits on the +New slot");
    page.feed_draft_key(press(KeyCode::Esc));
    assert_eq!(page.focus, ConfigureFocus::Modules);
    assert_eq!(
        page.selected_source_idx, 1,
        "Esc restores the pre-draft source"
    );
}

#[test]
fn draft_esc_restores_source_when_entered_via_navigation() {
    // 用 h/l 走到「+新建」进入 draft，Esc 也应还原到走过来之前的来源，而非最后一个。
    let mut page = ConfigurePage::new();
    page.module = ConfigureModule::PostProcessor;
    page.rows = vec![
        sample_row("post", "model", "model", "/tmp/shuohua/post/a.toml"),
        sample_row("post", "model", "model", "/tmp/shuohua/post/b.toml"),
    ];
    page.focus = ConfigureFocus::Fields;
    page.selected_source_idx = 0; // 停在来源 a
    page.on_key(press(KeyCode::Char('h'))); // 反向 wrap 到「+新建」，自动进入 draft
    assert!(page.draft_active(), "landing on +New opens the draft");
    page.feed_draft_key(press(KeyCode::Esc));
    assert_eq!(
        page.selected_source_idx, 0,
        "Esc returns to the source we came from, not the last one"
    );
}

#[test]
fn configure_field_list_scrolls_without_panic() {
    // The field list / detail pane render a top-aligned visible slice that
    // centers the selection (like the History list). Exercise the scroll math
    // and hit-region bookkeeping across window sizes and selections — including
    // long CJK values — to guard against slice/offset panics.
    crate::i18n::init("en-US");
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;
    let prompt = "App 上下文只用于消歧 ASR 文本，不要输出：\n\n<app>\nname: {{app_name}}\n</app>\n\n整理下面的 ASR 文本：\n{{text}}\n只输出整理后的文本。\n";
    let mk = |fp: &str, val: &str| {
        let mut r = sample_row("post", fp, fp, "/tmp/shuohua/post/deepseek.toml");
        r.value = val.to_string();
        r
    };
    let mut page = ConfigurePage::new();
    page.module = ConfigureModule::PostProcessor;
    page.focus = ConfigureFocus::Fields;
    page.rows = vec![
        mk("type", "llm"),
        mk("format", "openai"),
        mk("name", "deepseek"),
        mk("base_url", "https://api.deepseek.com"),
        mk("api_key", "••••••"),
        mk("model", "deepseek-v4-flash"),
        mk(
            "system_prompt",
            "你是 ASR 文本整理器。把语音识别文本整理成可直接使用的最终文本。规则：保留原意。",
        ),
        mk("prompt", prompt),
    ];
    page.selected_source_idx = 0;

    for (w, h) in [(80u16, 16u16), (64, 12), (60, 11), (40, 8), (30, 5)] {
        let mut term = Terminal::new(TestBackend::new(w, h)).unwrap();
        for sel in [0usize, 3, 6, 7, 6, 7, 5, 0] {
            page.selected = sel;
            page.detail_scroll = 0;
            term.draw(|f| {
                let area = f.area();
                crate::tui::configure::render::render_page(
                    f,
                    &page,
                    area,
                    &TuiTheme::default(),
                    "",
                );
            })
            .unwrap();
        }
    }
}

#[test]
fn required_field_without_default_is_not_resettable() {
    // 删掉 required 无默认值的字段会让文件缺必填项，故不提供 D 重置。
    let mut page = ConfigurePage::new();
    page.module = ConfigureModule::PostProcessor;
    page.rows = vec![SettingsRow {
        can_unset: false,
        group: "post".to_string(),
        field_path: "api_key".to_string(),
        display_key: "api_key".to_string(),
        value: "••••••".to_string(),
        default_value: String::new(),
        origin: FieldOrigin::Set,
        control: ControlKind::Text,
        editable: true,
        secret: true,
        source: "/tmp/shuohua/post/deepseek.toml".to_string(),
        description_key: None,
    }];
    page.focus = ConfigureFocus::Fields;
    page.selected = 0;
    page.selected_source_idx = 0;
    assert!(
        page.selected_editable_set().is_none(),
        "required-without-default field must not be resettable"
    );
}

#[test]
fn display_width_handles_narrow_non_ascii() {
    // 掩码用的 • 与省略号 …、破折号 — 都是窄字符（1 格），CJK 才是 2 格。
    assert_eq!(crate::tui::ui::display_width("••••••"), 6);
    assert_eq!(crate::tui::ui::display_width("…"), 1);
    assert_eq!(crate::tui::ui::display_width("—"), 1);
    assert_eq!(crate::tui::ui::display_width("你好"), 4);
    assert_eq!(crate::tui::ui::display_width("ab"), 2);
}

#[test]
fn centered_scroll_keeps_selection_in_view() {
    // Fits within the viewport → no scroll.
    assert_eq!(centered_scroll(5, 10, 20), 0);
    // Near the top → no scroll.
    assert_eq!(centered_scroll(1, 40, 10), 0);
    // Middle → selection centered.
    assert_eq!(centered_scroll(20, 40, 10), 15);
    // Near the bottom → clamp so the list doesn't scroll past its end.
    assert_eq!(centered_scroll(39, 40, 10), 30);
}

#[test]
fn enter_on_modules_focus_moves_to_fields() {
    let mut page = ConfigurePage::new();
    page.module = ConfigureModule::Profile;
    page.focus = ConfigureFocus::Modules;
    page.on_key(press(KeyCode::Enter));
    assert_eq!(page.focus, ConfigureFocus::Fields);
    assert_eq!(page.selected, 0);
}

/// Hotkey fields edit as plain text: the modal prefills the current value and
/// the user types syntax the daemon can't capture live (modifier taps, :double,
/// F13-F20). Regression: characters like `i` are inserted literally, not eaten
/// as a mode toggle, and editing never panics on the cursor.
#[test]
fn keycapture_edits_hotkey_as_plain_text() {
    use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};

    fn press(page: &mut ConfigurePage, code: KeyCode) -> super::EditOutcome {
        page.feed_modal_key(KeyEvent {
            code,
            modifiers: KeyModifiers::NONE,
            kind: KeyEventKind::Press,
            state: crossterm::event::KeyEventState::empty(),
        })
    }

    let dir = TestConfig::new();
    dir.write_main("[hotkey]\ntrigger = \"f16\"\n");
    let mut page = dir.configure_page();
    page.modal = Some(super::modal::ModalEditor::new(
        "hotkey.trigger".into(),
        super::modal::EditTarget::File(dir.main_path()),
        super::modal::ModalKind::KeyCapture,
        "f16".into(),
    ));
    // Prefilled with the current value, cursor parked at the end.
    assert_eq!(page.modal.as_ref().unwrap().buffer, "f16");
    assert_eq!(page.modal.as_ref().unwrap().cursor, 3);

    for _ in 0..3 {
        press(&mut page, KeyCode::Backspace);
    }
    // `i` is a literal character now, not a mode toggle.
    for ch in ['s', 'h', 'i', 'f', 't', '+', 'f', '1', '3'] {
        press(&mut page, KeyCode::Char(ch));
    }
    let m = page.modal.as_ref().unwrap();
    assert_eq!(m.buffer, "shift+f13");
    assert_eq!(m.cursor, m.buffer.len());

    // Enter commits and writes the field (valid syntax passes semantic validation).
    press(&mut page, KeyCode::Enter);
    assert!(page.modal.is_none());
    let saved = std::fs::read_to_string(dir.main_path()).unwrap();
    assert!(saved.contains("trigger = \"shift+f13\""), "{saved}");
}

#[test]
fn click_field_selects_row_and_focus() {
    use ratatui::layout::Rect;
    let mut page = ConfigurePage::new();
    page.module = ConfigureModule::Profile;
    page.rows = vec![
        sample_row("profile", "name", "name", "/x/a.toml"),
        sample_row("profile", "asr.instance", "asr.instance", "/x/a.toml"),
        sample_row("profile", "post.chain", "post.chain", "/x/a.toml"),
    ];
    page.hit
        .borrow_mut()
        .fields
        .push((Rect::new(24, 5, 50, 1), 2));
    let outcome = page.on_mouse(30, 5, super::MouseKind::Down);
    assert_eq!(page.focus, ConfigureFocus::Fields);
    assert_eq!(page.selected, 2);
    assert!(outcome.status.is_none());
}

#[test]
fn scroll_moves_selection_in_current_focus() {
    let mut page = ConfigurePage::new();
    page.module = ConfigureModule::Profile;
    page.focus = ConfigureFocus::Fields;
    page.rows = vec![
        sample_row("profile", "name", "name", "/x/a.toml"),
        sample_row("profile", "asr.instance", "asr.instance", "/x/a.toml"),
        sample_row("profile", "asr.hotwords", "asr.hotwords", "/x/a.toml"),
    ];
    page.selected = 0;
    page.on_mouse(0, 0, super::MouseKind::ScrollDown);
    assert_eq!(page.selected, 1);
    page.on_mouse(0, 0, super::MouseKind::ScrollUp);
    assert_eq!(page.selected, 0);
}

#[test]
fn mouse_noop_during_editing() {
    use ratatui::layout::Rect;
    let mut page = ConfigurePage::new();
    page.module = ConfigureModule::Profile;
    page.hit
        .borrow_mut()
        .fields
        .push((Rect::new(24, 5, 50, 1), 2));
    page.editing = Some(EditState {
        field_path: "name".to_string(),
        target: super::modal::EditTarget::File(std::path::PathBuf::from("/x/config.toml")),
        control: ControlKind::Text,
        cursor: "test".len(),
        buffer: "test".to_string(),
        original: "test".to_string(),
    });
    assert!(page.is_editing());
    page.on_mouse(30, 5, super::MouseKind::Down);
    // State should not change; we can't easily assert on borrow panic, but
    // the key invariant is that the method returns without side-effects.
    assert_eq!(page.selected, 0); // unchanged
}

#[test]
fn click_module_switches_focus_and_module() {
    use ratatui::layout::Rect;
    let mut page = ConfigurePage::new();
    page.module = ConfigureModule::Main;
    page.focus = ConfigureFocus::Fields;
    page.hit
        .borrow_mut()
        .modules
        .push((Rect::new(0, 1, 22, 1), 1)); // index 1 = Profile
    page.on_mouse(10, 1, super::MouseKind::Down);
    assert_eq!(page.focus, ConfigureFocus::Modules);
    assert_eq!(page.module, ConfigureModule::Profile);
    assert_eq!(page.selected, 0);
    assert_eq!(page.selected_source_idx, 0);
}

#[test]
fn click_source_switches_focus_and_selects_source() {
    use ratatui::layout::Rect;
    let mut page = ConfigurePage::new();
    page.module = ConfigureModule::Profile;
    page.focus = ConfigureFocus::Fields;
    // 注入两个真实来源，使命中的来源索引 1 存在——否则 clamp_selected 会把
    // selected_source_idx 夹回 0（且不依赖开发机上的 ~/.config 内容）。
    page.rows = vec![
        sample_row(
            "profile",
            "asr.instance",
            "asr",
            "/tmp/shuohua/profile/default.toml",
        ),
        sample_row(
            "profile",
            "asr.instance",
            "asr",
            "/tmp/shuohua/profile/agent.toml",
        ),
    ];
    page.hit
        .borrow_mut()
        .sources
        .push((Rect::new(24, 1, 18, 1), 1)); // index 1
    page.on_mouse(30, 1, super::MouseKind::Down);
    // 来源栏属于内容区，点它进入内容区（Fields）并选中该来源。
    assert_eq!(page.focus, ConfigureFocus::Fields);
    assert_eq!(page.selected_source_idx, 1);
    assert_eq!(page.selected, 0);
}

#[test]
fn click_new_llm_slot_starts_draft() {
    use ratatui::layout::Rect;
    let mut page = ConfigurePage::new();
    page.module = ConfigureModule::PostProcessor;
    page.rows = vec![sample_row(
        "post",
        "model",
        "model",
        "/tmp/shuohua/post/deepseek.toml",
    )];
    // 一个真实来源 → 「+ 新建 LLM」槽位索引 == 1。
    page.hit
        .borrow_mut()
        .sources
        .push((Rect::new(40, 1, 12, 1), 1));
    page.on_mouse(42, 1, super::MouseKind::Down);
    assert!(page.draft_active(), "clicking +New starts the draft");
}

#[test]
fn main_field_line_offsets_account_for_section_headers() {
    // Rendered layout mirrors main_grouped_lines:
    //   0 [hotkey header]
    //   1 hotkey.trigger   -> offset 1
    //   2 hotkey.cancel    -> offset 2
    //   3 [blank]
    //   4 [ui header]
    //   5 ui.language      -> offset 5
    let offsets = main_field_line_offsets(&["hotkey.trigger", "hotkey.cancel", "ui.language"]);
    assert_eq!(offsets, vec![1, 2, 5]);
}

#[test]
fn select_control_cycles_with_vim_keys() {
    use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
    fn key(ch: char) -> KeyEvent {
        KeyEvent {
            code: KeyCode::Char(ch),
            modifiers: KeyModifiers::empty(),
            kind: KeyEventKind::Press,
            state: crossterm::event::KeyEventState::empty(),
        }
    }

    let mut page = super::ConfigurePage::new();
    page.begin_edit_for(
        "backend",
        "/tmp/shuohua/config.toml".into(),
        ControlKind::Select(vec!["a".into(), "b".into(), "c".into()]),
        "a",
    );

    page.feed_edit_key(key('l'));
    assert_eq!(page.editing.as_ref().unwrap().buffer, "b");
    page.feed_edit_key(key('j'));
    assert_eq!(page.editing.as_ref().unwrap().buffer, "c");
    page.feed_edit_key(key('h'));
    assert_eq!(page.editing.as_ref().unwrap().buffer, "b");
    page.feed_edit_key(key('k'));
    assert_eq!(page.editing.as_ref().unwrap().buffer, "a");
}

// 回归：draft 打开时来源 tab 栏依旧登记命中区，点真实来源可退出 draft 切过去，
// draft 字段行也登记命中区——鼠标不再「点哪都没反应」。
#[test]
fn draft_keeps_source_strip_and_fields_clickable() {
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;
    crate::i18n::init("en-US");
    let cfg = TestConfig::new();
    let _env = ConfigHomeGuard::set(cfg.dir.parent().unwrap());
    for n in ["aaa", "bbb"] {
        let path = cfg.dir.join(format!("post/{n}.toml"));
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(
            &path,
            "type=\"llm\"\nname=\"x\"\napi_key=\"k\"\nmodel=\"m\"\nprompt=\"{{text}}\"\n",
        )
        .unwrap();
    }
    let mut page = cfg.configure_page();
    page.module = ConfigureModule::PostProcessor;
    page.refresh();
    let sources_len = page.sources_for_current_module().len();
    assert!(sources_len >= 2);

    let theme = TuiTheme::default();
    let mut term = Terminal::new(TestBackend::new(100, 30)).unwrap();

    // 点末尾「+ 新建」tab：进入 draft，且高亮该槽位（而非夹回最后一个真实来源）。
    term.draw(|f| render::render_page(f, &page, f.area(), &theme, ""))
        .unwrap();
    let (new_r, _) = *page.hit.borrow().sources.last().unwrap();
    page.on_mouse(new_r.x + 1, new_r.y, crate::tui::page::MouseKind::Down);
    assert!(page.draft_active(), "点 +新建 应进入 draft");
    assert_eq!(
        page.selected_source_idx, sources_len,
        "应高亮 +新建 槽位，而非最后一个真实来源"
    );

    term.draw(|f| render::render_page(f, &page, f.area(), &theme, ""))
        .unwrap();
    assert!(page.draft_active());
    assert!(
        !page.hit.borrow().sources.is_empty(),
        "draft 应保留可点的来源栏"
    );
    assert!(!page.hit.borrow().fields.is_empty(), "draft 字段行应可点");

    // 点第一个真实来源（idx 0）→ 退出 draft 并切到该来源。
    let (r, _) = page.hit.borrow().sources[0];
    page.on_mouse(r.x + 1, r.y, crate::tui::page::MouseKind::Down);
    assert!(!page.draft_active(), "点真实来源应退出 draft");
    assert_eq!(page.selected_source_idx, 0);
}

#[test]
fn delete_asr_instance_removes_file_and_refreshes() {
    crate::i18n::init("en-US");
    let cfg = TestConfig::new();
    let _env = ConfigHomeGuard::set(cfg.dir.parent().unwrap());
    let mut page = cfg.configure_page();
    page.module = ConfigureModule::AsrProvider;

    // Create an unreferenced ASR instance file (no profiles referencing "myprovider").
    let path = cfg.dir.join("asr/myprovider.toml");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, "type = \"doubao\"\n").unwrap();
    page.refresh();

    let idx = page
        .sources_for_current_module()
        .iter()
        .position(|s| s.ends_with("myprovider.toml"))
        .expect("ASR instance listed as a source");
    page.selected_source_idx = idx;

    // x → pending_delete; y → actually delete.
    page.request_delete();
    assert!(page.pending_delete.is_some());
    assert!(path.exists(), "file must still exist during confirmation");

    let outcome = page
        .resolve_pending_delete(KeyEvent::from(KeyCode::Char('y')))
        .unwrap();
    assert!(outcome.status.unwrap().contains("deleted ASR instance"));
    assert!(!path.exists());
    assert!(!page
        .sources_for_current_module()
        .iter()
        .any(|s| s.ends_with("myprovider.toml")));
}

#[test]
fn delete_referenced_asr_instance_sets_error_popup() {
    crate::i18n::init("en-US");
    let cfg = TestConfig::new();
    let _env = ConfigHomeGuard::set(cfg.dir.parent().unwrap());
    // Profile references "myprovider".
    cfg.write_profile(
        "default",
        "name = \"default\"\n[asr]\ninstance = \"myprovider\"\n",
    );
    cfg.write_main("[profile]\ndefault = \"default\"\n");
    let mut page = cfg.configure_page();
    page.module = ConfigureModule::AsrProvider;

    let path = cfg.dir.join("asr/myprovider.toml");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, "type = \"doubao\"\n").unwrap();
    page.refresh();

    let idx = page
        .sources_for_current_module()
        .iter()
        .position(|s| s.ends_with("myprovider.toml"))
        .expect("ASR instance listed as a source");
    page.selected_source_idx = idx;

    let outcome = page.delete_selected_asr_instance();

    assert!(outcome.status.is_none());
    assert!(
        page.edit_error
            .as_ref()
            .unwrap()
            .message
            .contains("referenced by profile"),
        "error: {:?}",
        page.edit_error
    );
    assert!(path.exists(), "file must still exist after blocked delete");
}

#[test]
fn delete_post_component_blocked_when_referenced_by_profile() {
    crate::i18n::init("en-US");
    let cfg = TestConfig::new();
    let _env = ConfigHomeGuard::set(cfg.dir.parent().unwrap());
    // Profile chain references "to_delete".
    cfg.write_profile(
        "default",
        "name = \"default\"\n[asr]\ninstance = \"apple\"\n[post]\nchain = [\"to_delete\"]\n",
    );
    cfg.write_main("[profile]\ndefault = \"default\"\n");
    let mut page = cfg.configure_page();
    page.module = ConfigureModule::PostProcessor;

    let path = cfg.dir.join("post/to_delete.toml");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(
        &path,
        "type = \"llm\"\nname = \"to_delete\"\nbase_url = \"https://a\"\napi_key = \"k\"\nmodel = \"m\"\nprompt = \"{{text}}\"\n",
    )
    .unwrap();
    page.refresh();

    let idx = page
        .sources_for_current_module()
        .iter()
        .position(|s| s.ends_with("to_delete.toml"))
        .expect("component listed as a source");
    page.selected_source_idx = idx;

    let outcome = page.delete_selected_post_component();

    assert!(outcome.status.is_none());
    assert!(
        page.edit_error
            .as_ref()
            .unwrap()
            .message
            .contains("referenced by profile"),
        "error: {:?}",
        page.edit_error
    );
    assert!(path.exists(), "file must still exist after blocked delete");
}
