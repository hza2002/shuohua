use std::cell::RefCell;
use std::collections::BTreeMap;
use std::path::PathBuf;

use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui::layout::Rect;
use ratatui::Frame;

use crate::config::field_view::{ControlKind, FieldOrigin};
use crate::config::field_write::{self, TypedInput, WriteError};
use crate::config::profile_compose_write;
use crate::config::template::LlmComponentDraft;
use crate::config::theme::TuiTheme;
use crate::ipc::protocol::{Command, Event};
use crate::tui::config_actions;
use crate::tui::configure::doctor::run_doctor;
use crate::tui::configure::hit::{HitRegions, HitTarget};
use crate::tui::configure::modal::{EditTarget, ModalEditor, ModalKind};
use crate::tui::configure::render::render_page;
use crate::tui::page::{KeyHint, KeyOutcome, MouseKind, Page};
use crate::tui::settings::{self, SettingsRow};

mod doctor;
pub(crate) mod hit;
mod modal;
mod profile_composer;
mod render;

#[cfg(test)]
mod tests;

// ---- types ----

#[derive(Debug, Clone, PartialEq)]
pub struct EditState {
    pub field_path: String,
    pub target: EditTarget,
    pub control: ControlKind,
    pub buffer: String,
    /// Cursor as a byte index into `buffer` (UTF-8 safe, like `ModalEditor`).
    pub cursor: usize,
    pub original: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EditError {
    pub field_path: String,
    pub value: String,
    pub message: String,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct EditOutcome {
    pub reload_config: bool,
    pub status: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfigureModule {
    Overview,
    Main,
    Profile,
    AsrProvider,
    PostProcessor,
}

impl ConfigureModule {
    fn next(self) -> Self {
        match self {
            Self::Overview | Self::Main => Self::Profile,
            Self::Profile => Self::AsrProvider,
            Self::AsrProvider => Self::PostProcessor,
            Self::PostProcessor => Self::Overview,
        }
    }

    fn prev(self) -> Self {
        match self {
            Self::Overview => Self::PostProcessor,
            Self::Main => Self::Overview,
            Self::Profile => Self::Overview,
            Self::AsrProvider => Self::Profile,
            Self::PostProcessor => Self::AsrProvider,
        }
    }

    pub fn inventory_module(self) -> crate::config::inventory::InventoryModule {
        match self {
            Self::Overview | Self::Main => crate::config::inventory::InventoryModule::Main,
            Self::Profile => crate::config::inventory::InventoryModule::Profile,
            Self::AsrProvider => crate::config::inventory::InventoryModule::AsrProvider,
            Self::PostProcessor => crate::config::inventory::InventoryModule::PostProcessor,
        }
    }

    fn title(self) -> String {
        match self {
            Self::Overview => crate::t!("tui.configure.main"),
            Self::Main => crate::t!("tui.configure.main"),
            Self::Profile => crate::t!("tui.configure.profile"),
            Self::AsrProvider => crate::t!("tui.configure.asr"),
            Self::PostProcessor => crate::t!("tui.configure.post"),
        }
    }

    pub(super) fn supports_new(self) -> bool {
        matches!(
            self,
            Self::AsrProvider | Self::PostProcessor | Self::Profile
        )
    }
}

/// 两级焦点：左边 Modules（模块列表）与右边 Fields（内容区）。
/// 内容区把「来源 tab 栏 + 字段列表」合成一层：h/l 横向切来源、j/k 纵向移字段；
/// 进入用 l/Enter，返回统一用 Esc。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfigureFocus {
    Modules,
    Fields,
}

/// draft 行的哨兵 source，区别于真实文件路径。
pub const DRAFT_SOURCE: &str = "<draft>";

const LLM_DRAFT_KEYS: [&str; 8] = [
    "preset",
    "file_id",
    "name",
    "base_url",
    "api_key",
    "model",
    "system_prompt",
    "prompt",
];

const PROFILE_DRAFT_KEYS: [&str; 3] = ["file_id", "name", "asr_instance"];

/// 可选的 provider 预设（= registry 里的 llm 模板 stem，如 openai/anthropic/deepseek）。
/// 新增一家 provider 只需在 registry 加模板，这里自动多一个选项。
fn llm_preset_ids() -> Vec<String> {
    crate::config::template::llm_templates()
        .filter_map(|template| {
            template
                .id
                .strip_prefix("post/")
                .map(|stem| stem.to_string())
        })
        .collect()
}

fn asr_kind_ids() -> Vec<String> {
    crate::config::template::asr_templates()
        .filter_map(|template| {
            template
                .id
                .strip_prefix("asr/")
                .map(|stem| stem.to_string())
        })
        .collect()
}

/// 新建 seed 文件的注释语言：跟随用户配置的 `ui.language`（与 `shuo config-template`
/// 同源），无法读取配置时回退到自动检测。
fn seed_lang() -> crate::i18n::Lang {
    let configured = crate::config::load_from(&crate::config::default_path())
        .map(|cfg| cfg.ui.language)
        .unwrap_or_else(|_| "auto".to_string());
    crate::i18n::resolve_lang(&configured)
}

/// 连通性测试状态（`t` 键触发，后台跑一次 check_runtime）。
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum DraftTestStatus {
    #[default]
    Idle,
    Testing,
    Ok,
    Failed(String),
}

/// 新建 LLM 组件的内存 draft。内部用现有 `LlmComponentDraft`，复用其校验/渲染/写盘。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LlmDraftForm {
    pub draft: LlmComponentDraft,
    pub selected: usize,
    /// 各字段的默认值基线。origin 着色按「当前值是否 != 默认」判断（改回原样=不算改），
    /// format reseed 会同步更新联动字段的基线，故自动填出的默认值不显示为「已改」。
    defaults: std::collections::BTreeMap<String, String>,
    model_options: Option<Vec<String>>,
    pub test_status: DraftTestStatus,
}

impl LlmDraftForm {
    pub fn new() -> Self {
        let draft =
            crate::config::template::llm_draft_from_template("post/openai").unwrap_or_else(|| {
                LlmComponentDraft {
                    template_id: "post/openai".to_string(),
                    file_id: "openai".to_string(),
                    provider_name: "openai".to_string(),
                    format: "openai".to_string(),
                    base_url: "https://api.openai.com/v1".to_string(),
                    api_key: String::new(),
                    model: "gpt-4.1-mini".to_string(),
                    system_prompt: String::new(),
                    prompt: "{{text}}".to_string(),
                }
            });
        let mut form = Self {
            draft,
            selected: 0,
            defaults: std::collections::BTreeMap::new(),
            model_options: None,
            test_status: DraftTestStatus::Idle,
        };
        // 初始所有字段都是默认值。
        for key in LLM_DRAFT_KEYS {
            let value = form.get(key);
            form.defaults.insert(key.to_string(), value);
        }
        form
    }

    pub fn get(&self, key: &str) -> String {
        match key {
            "preset" => self
                .draft
                .template_id
                .strip_prefix("post/")
                .unwrap_or("openai")
                .to_string(),
            "file_id" => self.draft.file_id.clone(),
            "name" => self.draft.provider_name.clone(),
            "base_url" => self.draft.base_url.clone(),
            "api_key" => self.draft.api_key.clone(),
            "model" => self.draft.model.clone(),
            "system_prompt" => self.draft.system_prompt.clone(),
            "prompt" => self.draft.prompt.clone(),
            _ => String::new(),
        }
    }

    pub fn edit(&mut self, key: &str, value: String) {
        match key {
            "preset" => self.draft.template_id = format!("post/{value}"),
            "file_id" => self.draft.file_id = value,
            "name" => self.draft.provider_name = value,
            "base_url" => self.draft.base_url = value,
            "api_key" => self.draft.api_key = value,
            "model" => self.draft.model = value,
            "system_prompt" => self.draft.system_prompt = value,
            "prompt" => self.draft.prompt = value,
            _ => {}
        }
        // 配置变了，之前的连通结果作废。
        self.test_status = DraftTestStatus::Idle;
        if matches!(key, "preset" | "base_url" | "api_key") {
            self.model_options = None;
        }
    }

    /// 切换 provider 预设后，从该模板重设 format/file_id/name/base_url/model，
    /// 保留 system_prompt/prompt（与 provider 无关）。同步更新这些字段的默认基线，
    /// 使联动出的值不被着色成「已改」。
    pub fn on_changed(&mut self, key: &str) {
        if key != "preset" {
            return;
        }
        let template_id = self.draft.template_id.clone();
        if let Some(seed) = crate::config::template::llm_draft_from_template(&template_id) {
            self.draft.format = seed.format;
            self.draft.file_id = seed.file_id;
            self.draft.provider_name = seed.provider_name;
            self.draft.base_url = seed.base_url;
            self.draft.model = seed.model;
            self.model_options = None;
            for key in ["file_id", "name", "base_url", "model"] {
                let value = self.get(key);
                self.defaults.insert(key.to_string(), value);
            }
        }
    }

    pub fn set_model_options(&mut self, options: Vec<String>) {
        self.model_options = (!options.is_empty()).then_some(options);
    }

    fn control_for(&self, key: &str) -> ControlKind {
        match key {
            "preset" => ControlKind::Select(llm_preset_ids()),
            "model" => self
                .model_options
                .clone()
                .map(ControlKind::Select)
                .unwrap_or(ControlKind::Text),
            "system_prompt" | "prompt" => ControlKind::MultilineText,
            _ => ControlKind::Text,
        }
    }

    /// 本地化字段标签，复用现有 `llm_create.field_*` 文案。
    fn label_for(key: &str) -> String {
        let label_key = match key {
            "preset" => "tui.configure.llm_create.field_preset",
            "file_id" => "tui.configure.llm_create.field_file_id",
            "name" => "tui.configure.llm_create.field_name",
            "base_url" => "tui.configure.llm_create.field_base_url",
            "api_key" => "tui.configure.llm_create.field_api_key",
            "model" => "tui.configure.llm_create.field_model",
            "system_prompt" => "tui.configure.llm_create.field_system_prompt",
            _ => "tui.configure.llm_create.field_prompt",
        };
        crate::i18n::tr(label_key, &[])
    }

    /// 字段说明的 i18n key（显示在 detail 面板/行尾）。
    fn desc_key_for(key: &str) -> &'static str {
        match key {
            "preset" => "tui.configure.llm_create.desc_preset",
            "file_id" => "tui.configure.llm_create.desc_file_id",
            "name" => "tui.configure.llm_create.desc_name",
            "base_url" => "tui.configure.llm_create.desc_base_url",
            "api_key" => "tui.configure.llm_create.desc_api_key",
            "model" => "tui.configure.llm_create.desc_model",
            "system_prompt" => "tui.configure.llm_create.desc_system_prompt",
            _ => "tui.configure.llm_create.desc_prompt",
        }
    }

    /// 渲染用行表。复用 `SettingsRow`：source 用哨兵值 `DRAFT_SOURCE`，group 空。
    /// origin 按「当前值 != 默认基线」判断，改回原样不算「已改」。
    pub fn rows(&self) -> Vec<SettingsRow> {
        LLM_DRAFT_KEYS
            .iter()
            .map(|key| {
                let value = self.get(key);
                let changed = self.defaults.get(*key) != Some(&value);
                let secret = *key == "api_key";
                // secret 值不明文显示：非空时用掩码。着色仍按真实值判断。
                let display = if secret && !value.is_empty() {
                    crate::config::spec::SECRET_MASK.to_string()
                } else {
                    value
                };
                SettingsRow {
                    group: String::new(),
                    field_path: (*key).to_string(),
                    display_key: Self::label_for(key),
                    value: display,
                    default_value: self.defaults.get(*key).cloned().unwrap_or_default(),
                    origin: if changed {
                        FieldOrigin::Set
                    } else {
                        FieldOrigin::Default
                    },
                    control: self.control_for(key),
                    editable: true,
                    secret,
                    // draft 行不走 D 重置（草稿有独立按键处理），值无所谓。
                    can_unset: true,
                    source: DRAFT_SOURCE.to_string(),
                    description_key: Some(Self::desc_key_for(key)),
                }
            })
            .collect()
    }

    /// 整体校验 + 写盘，返回创建的文件路径。复用现有 create_llm_component。
    pub fn commit(&self, post_dir: &std::path::Path) -> anyhow::Result<std::path::PathBuf> {
        crate::config::template::create_llm_component(post_dir, &self.draft)
    }
}

impl Default for LlmDraftForm {
    fn default() -> Self {
        Self::new()
    }
}

/// 新建 ASR 的内存草稿：一份未落盘的 TOML 文档。字段控件/校验完全复用 File 编辑器
/// 的 `field_view` + `field_write`（同一套事实源，不重复定义字段），因此「新建==编辑」
/// ——填完整份（含 secret）后 `^S` 一次写盘，`Esc` 干净丢弃，不留半成品文件。
/// `type` 像 LLM 的 `preset` 一样是首行可切 Select，切换即按 registry 模板重铺文档。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AsrDraftDoc {
    /// 选中的实现（registry 里的 asr 模板 stem：apple/aliyun/doubao/tencent）。
    kind: String,
    /// 新实例的文件名 stem（= 实例 ID），不是 schema 字段，单独一行。
    file_id: String,
    /// 内存中的 TOML 正文（含模板注释），编辑经 toml_edit round-trip 保留注释。
    body: String,
    pub selected: usize,
}

impl AsrDraftDoc {
    pub fn new() -> Self {
        let kind = asr_kind_ids()
            .into_iter()
            .next()
            .unwrap_or_else(|| "apple".to_string());
        let file_id = kind.clone();
        let mut form = Self {
            kind,
            file_id,
            body: String::new(),
            selected: 0,
        };
        form.reseed(&form.kind.clone());
        form
    }

    pub fn get(&self, key: &str) -> String {
        match key {
            "type" => self.kind.clone(),
            "file_id" => self.file_id.clone(),
            _ => String::new(),
        }
    }

    fn schema_id(&self) -> crate::config::schema::SchemaId {
        use crate::config::schema::SchemaId;
        match self.kind.as_str() {
            "aliyun" => SchemaId::AsrAliyun,
            "doubao" => SchemaId::AsrDoubao,
            "tencent" => SchemaId::AsrTencent,
            _ => SchemaId::AsrApple,
        }
    }

    fn value(&self) -> toml::Value {
        toml::from_str(&self.body).unwrap_or_else(|_| toml::Value::Table(Default::default()))
    }

    fn rel_path(&self) -> String {
        let stem = if self.file_id.is_empty() {
            "draft"
        } else {
            &self.file_id
        };
        format!("asr/{stem}.toml")
    }

    /// 按所选实现的 registry 平铺模板重铺内存文档（切换 type 时调用）。
    fn reseed(&mut self, kind: &str) {
        let template_id = format!("asr/{kind}");
        if let Some(template) =
            crate::config::template::asr_templates().find(|template| template.id == template_id)
        {
            self.kind = kind.to_string();
            self.body = crate::config::template::render_with_lang(template, seed_lang());
        }
    }

    /// 写入一个 draft 字段：type→重铺、file_id→改名、其余→按 schema coerce 后写内存
    /// 文档并做 Error 级校验（与 File 写盘同一套 `field_write::apply_field`）。
    pub fn apply_edit(&mut self, key: &str, buffer: &str) -> Result<(), String> {
        match key {
            "type" => {
                // 未自定义 file_id 时跟随默认名，与所选 provider 一致。
                if self.file_id == self.kind {
                    self.file_id = buffer.to_string();
                }
                self.reseed(buffer);
                Ok(())
            }
            "file_id" => {
                self.file_id = buffer.trim().to_string();
                Ok(())
            }
            _ => self.apply_schema_field(key, buffer),
        }
    }

    fn apply_schema_field(&mut self, key: &str, buffer: &str) -> Result<(), String> {
        let spec = crate::config::schema::spec_for(self.schema_id());
        let field = spec
            .field_for_path(key)
            .ok_or_else(|| format!("unknown field {key:?}"))?;
        // 与 File 编辑器一致：aliyun language_hints 单选映射为数组（auto=空数组）。
        let input = if key == "language_hints" {
            if buffer == "auto" {
                TypedInput::StrArray(Vec::new())
            } else {
                TypedInput::StrArray(vec![buffer.to_string()])
            }
        } else {
            coerce_input(field.kind(), buffer)?
        };
        let mut doc: toml_edit::DocumentMut =
            self.body.parse().map_err(|e| format!("draft parse: {e}"))?;
        field_write::apply_field(&mut doc, key, input, &spec)
            .map_err(|e| write_error_message(&e))?;
        self.body = doc.to_string();
        Ok(())
    }

    /// 渲染用行表：type（可切 Select）+ file_id（Text）+ 该实现的全部 schema 字段
    /// （控件由 `field_view` 派生，与编辑已落盘文件完全一致）。
    pub fn rows(&self) -> Vec<SettingsRow> {
        let mut rows = vec![
            SettingsRow {
                group: String::new(),
                field_path: "type".to_string(),
                display_key: "type".to_string(),
                value: self.kind.clone(),
                default_value: self.kind.clone(),
                origin: FieldOrigin::Default,
                control: ControlKind::Select(asr_kind_ids()),
                editable: true,
                secret: false,
                can_unset: false,
                source: DRAFT_SOURCE.to_string(),
                description_key: None,
            },
            SettingsRow {
                group: String::new(),
                field_path: "file_id".to_string(),
                display_key: "file_id".to_string(),
                value: self.file_id.clone(),
                default_value: String::new(),
                origin: FieldOrigin::Default,
                control: ControlKind::Text,
                editable: true,
                secret: false,
                can_unset: false,
                source: DRAFT_SOURCE.to_string(),
                description_key: None,
            },
        ];
        let value = self.value();
        let root = crate::config::paths::root_dir();
        let spec = crate::config::schema::spec_for(self.schema_id());
        for view in crate::config::field_view::field_views(&self.rel_path(), &spec, &value, &root) {
            if view.field_path == "type" {
                continue; // 上面已用可切换的 type 行表达
            }
            rows.push(SettingsRow {
                group: String::new(),
                display_key: view.field_path.clone(),
                field_path: view.field_path,
                value: view.effective,
                default_value: view.default_value,
                origin: view.origin,
                control: view.control,
                editable: view.editable,
                secret: view.secret,
                can_unset: view.can_unset,
                source: DRAFT_SOURCE.to_string(),
                description_key: view.description_key,
            });
        }
        rows
    }

    /// 一次落盘：校验文件名、整份文档做与 File 写盘同一套放行校验（结构性 Error 阻止，
    /// 未填 secret 允许——半成品可存，运行/doctor 再拦），拒重名后写盘。
    pub fn commit(&self, asr_dir: &std::path::Path) -> anyhow::Result<std::path::PathBuf> {
        crate::config::inventory::validate_config_file_id(&self.file_id)
            .map_err(|e| anyhow::anyhow!("invalid file name: {e}"))?;
        let value: toml::Value =
            toml::from_str(&self.body).map_err(|e| anyhow::anyhow!("draft parse: {e}"))?;
        let errors: Vec<String> = field_write::blocking_errors(
            &crate::config::schema::spec_for(self.schema_id()),
            &value,
        )
        .into_iter()
        .map(|d| format!("{}: {}", d.path, d.message))
        .collect();
        anyhow::ensure!(errors.is_empty(), "{}", errors.join("; "));
        std::fs::create_dir_all(asr_dir)
            .map_err(|e| anyhow::anyhow!("create {}: {e}", asr_dir.display()))?;
        let path = asr_dir.join(format!("{}.toml", self.file_id));
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
            .map_err(|e| {
                if e.kind() == std::io::ErrorKind::AlreadyExists {
                    anyhow::anyhow!(
                        "ASR instance {:?} already exists; pick a different file name",
                        self.file_id
                    )
                } else {
                    anyhow::anyhow!("create {}: {e}", path.display())
                }
            })?;
        std::io::Write::write_all(&mut file, self.body.as_bytes())
            .map_err(|e| anyhow::anyhow!("write {}: {e}", path.display()))?;
        Ok(path)
    }
}

impl Default for AsrDraftDoc {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProfileDraftForm {
    pub file_id: String,
    pub name: String,
    pub asr_instance: String,
    pub selected: usize,
    defaults: BTreeMap<String, String>,
}

impl ProfileDraftForm {
    pub fn new() -> Self {
        Self::new_in_root(&crate::config::paths::root_dir())
    }

    fn new_in_root(config_root: &std::path::Path) -> Self {
        let asr_instances = Self::available_asr_instances_in(config_root);
        let asr_instance = if asr_instances.iter().any(|id| id == "apple") {
            "apple".to_string()
        } else {
            asr_instances.first().cloned().unwrap_or_default()
        };
        let mut form = Self {
            file_id: String::new(),
            name: String::new(),
            asr_instance,
            selected: 0,
            defaults: BTreeMap::new(),
        };
        for key in PROFILE_DRAFT_KEYS {
            form.defaults.insert(key.to_string(), form.get(key));
        }
        form
    }

    pub fn available_asr_instances(&self) -> Vec<String> {
        Self::available_asr_instances_in(&crate::config::paths::root_dir())
    }

    fn available_asr_instances_in(config_root: &std::path::Path) -> Vec<String> {
        let mut ids = std::fs::read_dir(config_root.join("asr"))
            .ok()
            .into_iter()
            .flat_map(|entries| entries.filter_map(Result::ok))
            .filter_map(|entry| {
                let path = entry.path();
                (path.extension().and_then(|ext| ext.to_str()) == Some("toml"))
                    .then(|| path.file_stem()?.to_str().map(str::to_string))
                    .flatten()
            })
            .collect::<Vec<_>>();
        ids.sort();
        ids.dedup();
        ids
    }

    pub fn get(&self, key: &str) -> String {
        match key {
            "file_id" => self.file_id.clone(),
            "name" => self.name.clone(),
            "asr_instance" => self.asr_instance.clone(),
            _ => String::new(),
        }
    }

    pub fn edit(&mut self, key: &str, value: String) {
        match key {
            "file_id" => self.file_id = value,
            "name" => self.name = value,
            "asr_instance" => self.asr_instance = value,
            _ => {}
        }
    }

    pub fn rows(&self) -> Vec<SettingsRow> {
        self.rows_with_asr_instances(self.available_asr_instances())
    }

    #[cfg(test)]
    fn rows_in_root(&self, config_root: &std::path::Path) -> Vec<SettingsRow> {
        self.rows_with_asr_instances(Self::available_asr_instances_in(config_root))
    }

    fn rows_with_asr_instances(&self, asr_instances: Vec<String>) -> Vec<SettingsRow> {
        PROFILE_DRAFT_KEYS
            .iter()
            .map(|key| {
                let value = self.get(key);
                let changed = self.defaults.get(*key) != Some(&value);
                SettingsRow {
                    group: String::new(),
                    field_path: (*key).to_string(),
                    display_key: Self::label_for(key),
                    value,
                    default_value: self.defaults.get(*key).cloned().unwrap_or_default(),
                    origin: if changed {
                        FieldOrigin::Set
                    } else {
                        FieldOrigin::Default
                    },
                    control: match *key {
                        "asr_instance" => ControlKind::Select(asr_instances.clone()),
                        _ => ControlKind::Text,
                    },
                    editable: true,
                    secret: false,
                    can_unset: true,
                    source: DRAFT_SOURCE.to_string(),
                    description_key: Some(Self::desc_key_for(key)),
                }
            })
            .collect()
    }

    pub fn commit(&self, config_root: &std::path::Path) -> anyhow::Result<std::path::PathBuf> {
        let instances = Self::available_asr_instances_in(config_root);
        if instances.is_empty() {
            anyhow::bail!(crate::t!("tui.configure.profile_create.no_asr"));
        }
        if !instances.iter().any(|id| id == &self.asr_instance) {
            anyhow::bail!(crate::t!(
                "tui.configure.profile_create.invalid_asr",
                id = self.asr_instance
            ));
        }
        crate::config::profile::create_profile_file(
            config_root,
            &self.file_id,
            &self.name,
            &self.asr_instance,
        )
    }

    fn label_for(key: &str) -> String {
        let label_key = match key {
            "file_id" => "tui.configure.profile_create.field_file_id",
            "name" => "tui.configure.profile_create.field_name",
            "asr_instance" => "tui.configure.profile_create.field_asr_instance",
            _ => "tui.configure.profile_create.field_file_id",
        };
        crate::i18n::tr(label_key, &[])
    }

    fn desc_key_for(key: &str) -> &'static str {
        match key {
            "file_id" => "tui.configure.profile_create.desc_file_id",
            "name" => "tui.configure.profile_create.desc_name",
            "asr_instance" => "tui.configure.profile_create.desc_asr_instance",
            _ => "tui.configure.profile_create.desc_file_id",
        }
    }
}

impl Default for ProfileDraftForm {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Draft {
    Llm(Box<LlmDraftForm>),
    Asr(Box<AsrDraftDoc>),
    Profile(Box<ProfileDraftForm>),
}

impl Draft {
    fn rows(&self) -> Vec<SettingsRow> {
        match self {
            Draft::Llm(form) => form.rows(),
            Draft::Asr(form) => form.rows(),
            Draft::Profile(form) => form.rows(),
        }
    }

    fn selected(&self) -> usize {
        match self {
            Draft::Llm(form) => form.selected,
            Draft::Asr(form) => form.selected,
            Draft::Profile(form) => form.selected,
        }
    }

    fn selected_mut(&mut self) -> &mut usize {
        match self {
            Draft::Llm(form) => &mut form.selected,
            Draft::Asr(form) => &mut form.selected,
            Draft::Profile(form) => &mut form.selected,
        }
    }
}

/// Lightweight modal for the composer `a` flow: pick a post component id to
/// append to the profile chain.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemberPicker {
    pub ids: Vec<String>,
    pub selected: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DoctorState {
    pub ran_once: bool,
    pub status: Option<String>,
    pub output: String,
}

// ---- page state ----

#[derive(Debug)]
pub struct ConfigurePage {
    pub rows: Vec<SettingsRow>,
    pub selected: usize,
    pub selected_source_idx: usize,
    pub module: ConfigureModule,
    pub focus: ConfigureFocus,
    pub draft: Option<Draft>,
    /// 进入新建 draft 前选中的来源 tab；Esc 取消时用它还原，不改变来源选择。
    draft_prev_source: usize,
    /// 删除待确认：Some(显示名) 时下一个键 y=确认删除、其它=取消。
    pub pending_delete: Option<String>,
    pub doctor: DoctorState,
    pub editing: Option<EditState>,
    pub edit_error: Option<EditError>,
    pub modal: Option<ModalEditor>,
    /// Cached per-module (errors, missing) counts computed once per refresh.
    /// Each entry: (module label, error count, missing count).
    pub overview_counts: Vec<(String, usize, usize)>,
    /// Mouse hit regions populated each render frame; consumed by on_mouse.
    pub hit: RefCell<HitRegions>,
    /// Profile-module composer. Held only while the Profile module is focused;
    /// rebuilt when the selected profile changes or config refreshes so external
    /// edits are reflected. `None` for every other module.
    pub composer: Option<profile_composer::ProfileComposer>,
    /// Chain-member picker (the `a` flow): a selectable list of post component
    /// ids. `Some` while the picker is open.
    pub member_picker: Option<MemberPicker>,
    /// Vertical scroll offset of the selected-field detail pane. Reset to 0
    /// whenever the selection changes.
    pub detail_scroll: u16,
    /// Max scroll the detail pane allows (content lines beyond the viewport),
    /// recomputed each render so scroll handlers can clamp without over-scrolling.
    pub detail_max_scroll: std::cell::Cell<u16>,
    /// 删除配置文件（profile/asr/post）的策略：生产=移到系统废纸篓；测试=直接删。
    deleter: crate::trash::FileDeleter,
}

impl ConfigurePage {
    pub fn new() -> Self {
        let rows = settings::load_rows();
        let overview_counts = compute_overview_counts();
        Self {
            rows,
            selected: 0,
            selected_source_idx: 0,
            module: ConfigureModule::Main,
            focus: ConfigureFocus::Modules,
            draft: None,
            draft_prev_source: 0,
            pending_delete: None,
            doctor: DoctorState {
                ran_once: false,
                status: None,
                output: String::new(),
            },
            editing: None,
            edit_error: None,
            modal: None,
            overview_counts,
            hit: RefCell::new(HitRegions::default()),
            composer: None,
            member_picker: None,
            detail_scroll: 0,
            detail_max_scroll: std::cell::Cell::new(0),
            deleter: crate::trash::system_trash(),
        }
    }

    /// 注入删除策略（测试用：直接删，避免触碰真实 `~/.Trash`）。
    #[cfg(test)]
    fn with_deleter(mut self, deleter: crate::trash::FileDeleter) -> Self {
        self.deleter = deleter;
        self
    }

    pub fn refresh(&mut self) {
        self.rows = settings::load_rows();
        self.overview_counts = compute_overview_counts();
        self.clamp_selected();
        self.sync_composer();
    }

    /// Keep `self.composer` in step with the current module and selected profile.
    /// Rebuilds from disk whenever we are in Profile and the selected profile path
    /// changes (or after a refresh, so external edits reflect); clears it otherwise.
    fn sync_composer(&mut self) {
        if self.module != ConfigureModule::Profile {
            self.composer = None;
            self.member_picker = None;
            return;
        }
        let Some(path) = self.selected_config_source() else {
            self.composer = None;
            self.member_picker = None;
            return;
        };
        let root = crate::config::paths::root_dir();
        // Preserve the selected row across a same-profile rebuild so refreshes
        // don't jump the cursor back to the top.
        let prev_selected = self
            .composer
            .as_ref()
            .filter(|c| c.profile_path == path)
            .map(|c| c.selected);
        let mut composer = profile_composer::ProfileComposer::load(path, &root);
        if let Some(sel) = prev_selected {
            composer.selected = sel.min(composer.rows().len().saturating_sub(1));
        }
        self.composer = Some(composer);
    }

    pub fn llm_draft(&self) -> Option<&LlmDraftForm> {
        match self.draft.as_ref() {
            Some(Draft::Llm(form)) => Some(form.as_ref()),
            _ => None,
        }
    }

    pub fn llm_draft_mut(&mut self) -> Option<&mut LlmDraftForm> {
        match self.draft.as_mut() {
            Some(Draft::Llm(form)) => Some(form.as_mut()),
            _ => None,
        }
    }

    #[cfg(test)]
    pub fn asr_draft_mut(&mut self) -> Option<&mut AsrDraftDoc> {
        match self.draft.as_mut() {
            Some(Draft::Asr(form)) => Some(form.as_mut()),
            _ => None,
        }
    }

    pub fn validate(&mut self) -> String {
        self.refresh();
        self.doctor = run_doctor();
        crate::t!("tui.configure.validated")
    }

    pub fn request_reload(&mut self) -> (Command, String) {
        self.refresh();
        (
            Command::ReloadConfig,
            crate::t!("tui.configure.reload_requested"),
        )
    }

    pub fn open_selected_file(&self) -> String {
        let Some(path) = self.selected_config_source() else {
            return crate::t!("tui.configure.no_config_selected");
        };
        match config_actions::open_path(&path) {
            Ok(()) => crate::i18n::tr(
                "tui.configure.opening",
                &[("path", path.display().to_string())],
            ),
            Err(e) => crate::t!("tui.error.config_action", error = e),
        }
    }

    pub fn reveal_in_finder(&self) -> String {
        let Some(path) = self
            .selected_config_source()
            .or_else(|| self.config_directory())
        else {
            return crate::t!("tui.configure.no_config_selected");
        };
        match config_actions::reveal_in_finder(&path) {
            Ok(()) => crate::t!("tui.configure.revealing", path = path.display()),
            Err(e) => crate::t!("tui.error.config_action", error = e),
        }
    }

    pub fn start_llm_create(&mut self) -> String {
        self.edit_error = None;
        self.module = ConfigureModule::PostProcessor;
        // draft 是内容区里「+ 新建」这个来源 tab 的展开，焦点归内容区。
        self.focus = ConfigureFocus::Fields;
        let sources_len = self.sources_for_current_module().len();
        // 记住进入前选中的来源，Esc 取消时还原（不因新建而改变来源选择）；
        // 若本就停在「+新建」槽位，退到最后一个真实来源。
        self.draft_prev_source = self.selected_source_idx.min(sources_len.saturating_sub(1));
        // 让来源 tab 栏高亮末尾的「+ 新建 LLM」槽位。
        self.selected_source_idx = sources_len;
        self.draft = Some(Draft::Llm(Box::default()));
        crate::t!("tui.configure.llm_create.started")
    }

    pub fn start_asr_create(&mut self) -> String {
        self.edit_error = None;
        self.module = ConfigureModule::AsrProvider;
        self.focus = ConfigureFocus::Fields;
        let sources_len = self.sources_for_current_module().len();
        self.draft_prev_source = self.selected_source_idx.min(sources_len.saturating_sub(1));
        self.selected_source_idx = sources_len;
        self.draft = Some(Draft::Asr(Box::default()));
        crate::t!("tui.configure.asr_create.started")
    }

    fn start_create_for_current_module(&mut self) -> String {
        match self.module {
            ConfigureModule::AsrProvider => self.start_asr_create(),
            ConfigureModule::PostProcessor => self.start_llm_create(),
            ConfigureModule::Profile => self.start_profile_create(),
            _ => String::new(),
        }
    }

    pub fn draft_active(&self) -> bool {
        self.draft.is_some()
    }

    pub fn draft_supports_test(&self) -> bool {
        matches!(self.draft, Some(Draft::Llm(_)))
    }

    /// draft 表的导航层：j/k 选行、Enter 打开对应控件（复用现有编辑器）、
    /// ^S 提交、Esc 取消。字段内编辑由 feed_edit_key/feed_modal_key 处理。
    pub fn feed_draft_key(&mut self, key: KeyEvent) -> EditOutcome {
        if self.draft.is_none() || key.kind != KeyEventKind::Press {
            return EditOutcome::default();
        }
        match key.code {
            // Esc 是统一的「返回上一层」：取消新建、回到左边模块列表，并把来源选择
            // 还原成进入前那个（Esc 只是取消，不该顺带切换来源）。
            KeyCode::Esc => {
                self.discard_draft();
                self.selected_source_idx = self
                    .draft_prev_source
                    .min(self.sources_for_current_module().len().saturating_sub(1));
                self.focus = ConfigureFocus::Modules;
                EditOutcome {
                    reload_config: false,
                    status: Some(self.draft_cancelled_status()),
                }
            }
            // h 把 draft 当内容区里的一个来源 tab：横向退回上一个真实来源（留在内容区）。
            KeyCode::Char('h') | KeyCode::Left => {
                self.discard_draft();
                self.selected_source_idx =
                    self.sources_for_current_module().len().saturating_sub(1);
                self.focus = ConfigureFocus::Fields;
                EditOutcome::default()
            }
            KeyCode::Char('s') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.commit_draft()
            }
            KeyCode::Char('j') | KeyCode::Down => {
                self.move_draft_selection(1);
                EditOutcome::default()
            }
            KeyCode::Char('k') | KeyCode::Up => {
                self.move_draft_selection(-1);
                EditOutcome::default()
            }
            KeyCode::Enter => {
                self.open_draft_field_editor();
                EditOutcome::default()
            }
            _ => EditOutcome::default(),
        }
    }

    /// 丢弃 draft。来源选择的落点由调用方决定（Esc 还原进入前、h 退到相邻来源），
    /// 因为两者语义不同。
    fn discard_draft(&mut self) {
        self.draft = None;
    }

    fn move_draft_selection(&mut self, delta: isize) {
        let Some(draft) = self.draft.as_mut() else {
            return;
        };
        let len = draft.rows().len() as isize;
        let selected = draft.selected_mut();
        *selected = (*selected as isize + delta).clamp(0, len - 1) as usize;
    }

    /// Enter 打开当前 draft 行的编辑器：MultilineText → ModalEditor，
    /// Text/Select → EditState，目标一律 Draft(key)。
    fn open_draft_field_editor(&mut self) {
        let rows = self.draft_rows();
        let selected = self.draft_selected();
        let Some(row) = rows.get(selected) else {
            return;
        };
        let key = row.field_path.clone();
        let control = row.control.clone();
        let original = row.value.clone();
        self.edit_error = None;
        if let Some(kind) = ModalEditor::kind_for(&control, row.secret) {
            self.modal = Some(ModalEditor::new(
                key.clone(),
                EditTarget::Draft(key),
                kind,
                original,
            ));
        } else {
            self.editing = Some(EditState {
                field_path: key.clone(),
                target: EditTarget::Draft(key),
                control,
                cursor: original.len(),
                buffer: original.clone(),
                original,
            });
        }
    }

    fn draft_rows(&self) -> Vec<SettingsRow> {
        self.draft.as_ref().map(Draft::rows).unwrap_or_default()
    }

    fn draft_selected(&self) -> usize {
        self.draft.as_ref().map_or(0, Draft::selected)
    }

    fn draft_cancelled_status(&self) -> String {
        match self.draft.as_ref() {
            Some(Draft::Asr(_)) => crate::t!("tui.configure.asr_create.cancelled"),
            Some(Draft::Profile(_)) => crate::t!("tui.configure.profile_create.cancelled"),
            _ => crate::t!("tui.configure.llm_create.cancelled"),
        }
    }

    fn draft_title_for_error(&self, draft: &Draft) -> String {
        match draft {
            Draft::Asr(_) => crate::t!("tui.configure.asr_create.title"),
            Draft::Llm(_) => crate::t!("tui.configure.llm_create.title"),
            Draft::Profile(_) => crate::t!("tui.configure.profile_create.title"),
        }
    }

    fn draft_value_for_error(&self, draft: &Draft) -> String {
        match draft {
            Draft::Asr(form) => form.get("file_id"),
            Draft::Llm(form) => form.draft.file_id.clone(),
            Draft::Profile(form) => form.get("file_id"),
        }
    }

    fn draft_created_key_for_module(&self, module: ConfigureModule) -> &'static str {
        match module {
            ConfigureModule::AsrProvider => "tui.configure.asr_create.created",
            ConfigureModule::Profile => "tui.configure.profile_create.created",
            _ => "tui.configure.llm_create.created",
        }
    }

    /// 从 draft 构造一次性连通测试用的 provider 配置（None = 当前不在 draft）。
    pub fn draft_test_config(&self) -> Option<crate::post::llm::LlmCleanupConfig> {
        let d = &self.llm_draft()?.draft;
        let format = if d.format == "anthropic" {
            crate::post::llm::ProviderFormat::Anthropic
        } else {
            crate::post::llm::ProviderFormat::OpenAi
        };
        Some(crate::post::llm::LlmCleanupConfig {
            name: if d.file_id.is_empty() {
                d.provider_name.clone()
            } else {
                d.file_id.clone()
            },
            format,
            provider_name: d.provider_name.clone(),
            base_url: d.base_url.clone(),
            api_key: d.api_key.clone(),
            model: d.model.clone(),
            extra_body: serde_json::Map::new(),
            system_prompt: Some(d.system_prompt.clone()).filter(|s| !s.trim().is_empty()),
            prompt: d.prompt.clone(),
        })
    }

    pub fn set_draft_testing(&mut self) {
        if let Some(form) = self.llm_draft_mut() {
            form.test_status = DraftTestStatus::Testing;
        }
    }

    pub fn set_draft_test_result(&mut self, result: Result<(), String>) {
        if let Some(form) = self.llm_draft_mut() {
            form.test_status = match result {
                Ok(()) => DraftTestStatus::Ok,
                Err(message) => DraftTestStatus::Failed(message),
            };
        }
    }

    pub fn set_draft_model_options(&mut self, models: Vec<String>) {
        if let Some(form) = self.llm_draft_mut() {
            form.set_model_options(models);
        }
    }

    pub fn commit_draft(&mut self) -> EditOutcome {
        let Some(draft) = self.draft.clone() else {
            return EditOutcome::default();
        };
        let result = match &draft {
            Draft::Llm(form) => form
                .commit(&crate::config::post::default_dir())
                .map(|path| (path, ConfigureModule::PostProcessor)),
            Draft::Asr(form) => form
                .commit(&crate::config::paths::root_dir().join("asr"))
                .map(|path| (path, ConfigureModule::AsrProvider)),
            Draft::Profile(form) => form
                .commit(&crate::config::paths::root_dir())
                .map(|path| (path, ConfigureModule::Profile)),
        };
        match result {
            Ok((path, module)) => {
                self.draft = None;
                self.edit_error = None;
                self.module = module;
                // 保存后停在内容区新建出的那个来源上。
                self.focus = ConfigureFocus::Fields;
                self.refresh();
                if let Some(idx) = self
                    .sources_for_current_module()
                    .iter()
                    .position(|source| source == &path)
                {
                    self.selected_source_idx = idx;
                    self.selected = 0;
                }
                EditOutcome {
                    reload_config: false,
                    status: Some(crate::i18n::tr(
                        self.draft_created_key_for_module(module),
                        &[("path", path.display().to_string())],
                    )),
                }
            }
            Err(e) => {
                self.edit_error = Some(EditError {
                    field_path: self.draft_title_for_error(&draft),
                    value: self.draft_value_for_error(&draft),
                    message: format!("{e:#}"),
                });
                EditOutcome::default()
            }
        }
    }

    pub fn start_profile_create(&mut self) -> String {
        self.start_profile_create_in_root(&crate::config::paths::root_dir())
    }

    fn start_profile_create_in_root(&mut self, config_root: &std::path::Path) -> String {
        self.edit_error = None;
        let form = ProfileDraftForm::new_in_root(config_root);
        if ProfileDraftForm::available_asr_instances_in(config_root).is_empty() {
            self.draft = None;
            return crate::t!("tui.configure.profile_create.no_asr");
        }
        self.module = ConfigureModule::Profile;
        self.focus = ConfigureFocus::Fields;
        let sources_len = self.sources_for_current_module().len();
        self.draft_prev_source = self.selected_source_idx.min(sources_len.saturating_sub(1));
        self.selected_source_idx = sources_len;
        self.draft = Some(Draft::Profile(Box::new(form)));
        crate::t!("tui.configure.profile_create.started")
    }

    /// Scroll the selected-field detail pane, clamped to the content height
    /// recorded at the last render.
    pub fn scroll_detail(&mut self, delta: i32) {
        let max = self.detail_max_scroll.get();
        let next = (self.detail_scroll as i32 + delta).clamp(0, max as i32);
        self.detail_scroll = next as u16;
    }

    /// j/k 纵向移动：模块列表里换模块，内容区里换字段。
    pub fn move_selection(&mut self, delta: isize) {
        self.detail_scroll = 0;
        match self.focus {
            ConfigureFocus::Modules => {
                self.module = if delta >= 0 {
                    self.module.next()
                } else {
                    self.module.prev()
                };
                self.selected = 0;
                self.selected_source_idx = 0;
                self.clamp_selected();
                self.sync_composer();
            }
            ConfigureFocus::Fields => {
                let len = self.current_len();
                if len == 0 {
                    self.selected = 0;
                    return;
                }
                if delta >= 0 {
                    self.selected = (self.selected + 1).min(len - 1);
                } else {
                    self.selected = self.selected.saturating_sub(1);
                }
            }
        }
    }

    /// 内容区里 h/l 横向切来源 tab；落到支持新建的末尾「+ 新建」槽位就
    /// 直接进入新建（无需再按回车）。非 per-source 模块（Main/Overview）为空操作。
    fn switch_source(&mut self, delta: isize) {
        self.detail_scroll = 0;
        let prev = self.selected_source_idx;
        self.cycle_source(delta);
        // Switching the profile tab must rebuild the composer for the new file.
        self.sync_composer();
        if self.module.supports_new()
            && self.selected_source_idx == self.sources_for_current_module().len()
            && self.draft.is_none()
        {
            self.start_create_for_current_module();
            // 经 h/l 走到「+新建」进入 draft：还原点是走过来之前那个真实来源，
            // 而不是 start_llm_create 记下的「+新建」槽位本身。
            self.draft_prev_source = prev;
        }
    }

    /// 从模块列表进入右边内容区（l/Enter）。
    fn enter_content(&mut self) {
        self.detail_scroll = 0;
        self.focus = ConfigureFocus::Fields;
        self.selected = 0;
        self.sync_composer();
    }

    pub fn rows_for_current_module(&self) -> Vec<&SettingsRow> {
        let module = self.module.inventory_module();
        self.rows
            .iter()
            .filter(|row| row.group == module.label())
            .collect()
    }

    pub fn sources_for_current_module(&self) -> Vec<PathBuf> {
        let mut sources = self
            .rows_for_current_module()
            .into_iter()
            .map(|row| PathBuf::from(&row.source))
            .collect::<Vec<_>>();
        sources.sort();
        sources.dedup();
        sources
    }

    pub fn selected_config_source(&self) -> Option<PathBuf> {
        match self.module {
            ConfigureModule::Overview => Some(crate::config::default_path()),
            ConfigureModule::Main => Some(crate::config::default_path()),
            ConfigureModule::Profile
            | ConfigureModule::PostProcessor
            | ConfigureModule::AsrProvider => self
                .sources_for_current_module()
                .get(self.selected_source_idx)
                .cloned(),
        }
    }

    fn config_directory(&self) -> Option<PathBuf> {
        crate::config::default_path()
            .parent()
            .map(|path| path.to_path_buf())
    }

    fn current_len(&self) -> usize {
        let label = self.module.inventory_module().label();
        match self.module {
            ConfigureModule::Main | ConfigureModule::Overview => {
                self.rows.iter().filter(|r| r.group == label).count()
            }
            _ => {
                let source = self.selected_config_source();
                match source {
                    Some(path) => self
                        .rows
                        .iter()
                        .filter(|r| r.group == label && std::path::Path::new(&r.source) == path)
                        .count(),
                    None => 0,
                }
            }
        }
    }

    fn clamp_selected(&mut self) {
        let len = self.current_len();
        self.selected = self.selected.min(len.saturating_sub(1));
        // 支持新建的模块允许选中末尾「+ 新建」槽位（索引 == sources.len()），与
        // cycle_source 的槽位数一致；否则会把新建槽位夹回最后一个真实来源。
        let sources_len = self.sources_for_current_module().len();
        let max_source = if self.module.supports_new() {
            sources_len
        } else {
            sources_len.saturating_sub(1)
        };
        self.selected_source_idx = self.selected_source_idx.min(max_source);
    }

    fn cycle_source(&mut self, delta: isize) {
        // 支持新建的模块多一个末尾「+ New…」槽位（索引 == sources.len()）。
        let sources = self.sources_for_current_module().len();
        let slots = if self.module.supports_new() {
            sources + 1
        } else {
            sources
        };
        if slots <= 1 {
            return;
        }
        let next = (self.selected_source_idx as isize + delta).rem_euclid(slots as isize);
        self.selected_source_idx = next as usize;
        self.selected = 0;
    }

    pub fn on_mouse(&mut self, column: u16, row: u16, kind: MouseKind) -> KeyOutcome {
        // 字段正在编辑时鼠标不动状态（编辑器独占）。draft 打开时鼠标仍然活着：
        // 来源栏和 draft 字段行都登记了命中区，点它们和普通来源一样处理。
        if self.is_editing() {
            return KeyOutcome::none();
        }
        // Wheel over the detail pane scrolls its text; elsewhere it moves the
        // field selection (which resets the detail scroll).
        let over_detail = self.hit.borrow().detail_contains(column, row);
        match kind {
            MouseKind::ScrollDown if over_detail => self.scroll_detail(1),
            MouseKind::ScrollUp if over_detail => self.scroll_detail(-1),
            MouseKind::ScrollDown if self.draft_active() => self.move_draft_selection(1),
            MouseKind::ScrollUp if self.draft_active() => self.move_draft_selection(-1),
            MouseKind::ScrollDown => {
                self.move_selection(1);
                self.clamp_selected();
            }
            MouseKind::ScrollUp => {
                self.move_selection(-1);
                self.clamp_selected();
            }
            MouseKind::Down => {
                self.detail_scroll = 0;
                let hit_target = self.hit.borrow().hit(column, row);
                if let Some(target) = hit_target {
                    self.apply_mouse_hit(target);
                }
            }
        }
        KeyOutcome::none()
    }

    fn apply_mouse_hit(&mut self, target: HitTarget) {
        // draft 打开时：点来源 tab 就像在真实来源间切换——点别的来源丢弃 draft 切过去，
        // 点末尾自己那个「+新建」槽位不动；点 draft 字段行只改 draft 选中行。
        if self.draft_active() {
            match target {
                HitTarget::Module(i) => {
                    self.discard_draft();
                    self.focus = ConfigureFocus::Modules;
                    let modules = all_modules_ordered();
                    if let Some(&m) = modules.get(i) {
                        self.module = m;
                        self.selected = 0;
                        self.selected_source_idx = 0;
                    }
                    self.clamp_selected();
                }
                HitTarget::Source(i) => {
                    let sources_len = self.sources_for_current_module().len();
                    if i < sources_len {
                        self.discard_draft();
                        self.focus = ConfigureFocus::Fields;
                        self.selected_source_idx = i;
                        self.selected = 0;
                        self.clamp_selected();
                    }
                    // i == sources_len：点的是自己这个「+新建」槽位，保持 draft。
                }
                HitTarget::Field(i) => match self.draft.as_mut() {
                    Some(Draft::Llm(form)) => {
                        let len = form.rows().len();
                        if i < len {
                            form.selected = i;
                        }
                    }
                    Some(Draft::Asr(form)) => {
                        let len = form.rows().len();
                        if i < len {
                            form.selected = i;
                        }
                    }
                    Some(Draft::Profile(form)) => {
                        let len = form.rows().len();
                        if i < len {
                            form.selected = i;
                        }
                    }
                    None => {}
                },
            }
            return;
        }
        match target {
            HitTarget::Module(i) => {
                self.focus = ConfigureFocus::Modules;
                let modules = all_modules_ordered();
                if let Some(&m) = modules.get(i) {
                    self.module = m;
                    self.selected = 0;
                    self.selected_source_idx = 0;
                }
            }
            HitTarget::Source(i) => {
                // 来源栏属于内容区，点它即进入内容区并选中该来源。
                self.focus = ConfigureFocus::Fields;
                self.selected_source_idx = i;
                self.selected = 0;
                // 点末尾「+ 新建」槽位（索引 == 真实来源数）→ 进入新建。
                if self.module.supports_new() && i == self.sources_for_current_module().len() {
                    self.start_create_for_current_module();
                }
            }
            HitTarget::Field(i) => {
                self.focus = ConfigureFocus::Fields;
                self.selected = i;
                // In Profile the field list is the composer's rows; move its cursor.
                if let Some(composer) = self.composer.as_mut() {
                    composer.selected = i.min(composer.rows().len().saturating_sub(1));
                }
            }
        }
        self.clamp_selected();
        self.sync_composer();
    }
}

// ---- edit state machine ----

impl ConfigurePage {
    pub fn begin_edit_for(
        &mut self,
        field_path: &str,
        source: std::path::PathBuf,
        control: ControlKind,
        original: &str,
    ) {
        self.edit_error = None;
        self.editing = Some(EditState {
            field_path: field_path.to_string(),
            target: EditTarget::File(source),
            control,
            cursor: original.len(),
            buffer: original.to_string(),
            original: original.to_string(),
        });
    }

    pub fn cancel_edit(&mut self) {
        self.editing = None;
    }

    #[allow(dead_code)] // used only by tests
    pub fn set_buffer(&mut self, value: &str) {
        if let Some(edit) = &mut self.editing {
            edit.buffer = value.to_string();
            edit.cursor = edit.buffer.len();
        }
    }

    pub fn push_char(&mut self, ch: char) {
        self.insert_text(&ch.to_string());
    }

    pub fn insert_text(&mut self, text: &str) {
        if let Some(edit) = &mut self.editing {
            if matches!(
                edit.control,
                ControlKind::Text | ControlKind::EditableSelect(_) | ControlKind::Number { .. }
            ) {
                edit.buffer.insert_str(edit.cursor, text);
                edit.cursor += text.len();
            }
        }
    }

    /// 光标前一个字符删除（UTF-8 安全，与 `ModalEditor::backspace` 一致）。
    pub fn backspace(&mut self) {
        if let Some(edit) = &mut self.editing {
            if edit.cursor == 0 {
                return;
            }
            let prev = edit.buffer[..edit.cursor]
                .char_indices()
                .last()
                .map(|(idx, _)| idx)
                .unwrap_or(0);
            edit.buffer.replace_range(prev..edit.cursor, "");
            edit.cursor = prev;
        }
    }

    /// 光标处向后删除一个字符（Delete 键）。
    pub fn delete_forward(&mut self) {
        if let Some(edit) = &mut self.editing {
            if edit.cursor >= edit.buffer.len() {
                return;
            }
            let next = edit.cursor
                + edit.buffer[edit.cursor..]
                    .chars()
                    .next()
                    .map(char::len_utf8)
                    .unwrap_or(0);
            edit.buffer.replace_range(edit.cursor..next, "");
        }
    }

    pub fn move_cursor_left(&mut self) {
        if let Some(edit) = &mut self.editing {
            if edit.cursor == 0 {
                return;
            }
            edit.cursor = edit.buffer[..edit.cursor]
                .char_indices()
                .last()
                .map(|(idx, _)| idx)
                .unwrap_or(0);
        }
    }

    pub fn move_cursor_right(&mut self) {
        if let Some(edit) = &mut self.editing {
            if edit.cursor >= edit.buffer.len() {
                return;
            }
            edit.cursor += edit.buffer[edit.cursor..]
                .chars()
                .next()
                .map(char::len_utf8)
                .unwrap_or(0);
        }
    }

    pub fn move_cursor_home(&mut self) {
        if let Some(edit) = &mut self.editing {
            edit.cursor = 0;
        }
    }

    pub fn move_cursor_end(&mut self) {
        if let Some(edit) = &mut self.editing {
            edit.cursor = edit.buffer.len();
        }
    }

    /// Toggle 翻转 / Select 循环；delta 取 ±1。
    pub fn toggle_or_cycle(&mut self, delta: isize) {
        let Some(edit) = &mut self.editing else {
            return;
        };
        match &edit.control {
            ControlKind::Toggle => {
                edit.buffer = if edit.buffer == "true" {
                    "false".into()
                } else {
                    "true".into()
                };
            }
            ControlKind::Select(opts) | ControlKind::EditableSelect(opts) if !opts.is_empty() => {
                match opts.iter().position(|o| o == &edit.buffer) {
                    Some(cur) => {
                        let next = (cur as isize + delta).rem_euclid(opts.len() as isize) as usize;
                        edit.buffer = opts[next].clone();
                    }
                    None => edit.buffer = opts[0].clone(),
                }
            }
            _ => {}
        }
    }

    pub fn dismiss_error(&mut self) {
        self.edit_error = None;
    }

    /// Commit an edit to the Profile composer's selected row. The composer
    /// validates against the resolved provider/component schema and writes only
    /// the profile file; a failure surfaces via the shared inline error popup.
    fn commit_composer_edit(&mut self, edit: &EditState, input: TypedInput) -> EditOutcome {
        let Some(composer) = self.composer.as_mut() else {
            self.editing = None;
            return EditOutcome::default();
        };
        match composer.commit_edit(input) {
            Ok(()) => {
                self.editing = None;
                self.edit_error = None;
                EditOutcome {
                    reload_config: false,
                    status: Some(crate::t!("tui.configure.edit.saved_local")),
                }
            }
            Err(e) => {
                self.set_edit_error(edit, format!("{e:#}"));
                EditOutcome::default()
            }
        }
    }

    /// Route a key to the Profile composer. Returns `Some` when the key was a
    /// composer action; `None` lets it fall through to the generic navigation
    /// (h/l source switch, Esc back, v/R/e/r/n, scrolling).
    fn feed_composer_key(&mut self, key: KeyEvent) -> Option<KeyOutcome> {
        use profile_composer::ComposerRowKind;
        let composer = self.composer.as_mut()?;
        let len = composer.rows().len();
        let kind = composer
            .rows()
            .get(composer.selected)
            .map(|r| r.kind.clone());
        let is_member = matches!(kind, Some(ComposerRowKind::ChainMember { .. }));
        // `D` resets an override/scalar to inherited; not meaningful on section
        // headers or chain members (they fall through to generic nav).
        let resettable = matches!(
            kind,
            Some(
                ComposerRowKind::AsrOverride { .. }
                    | ComposerRowKind::LlmOverride { .. }
                    | ComposerRowKind::Name
                    | ComposerRowKind::AsrInstance
                    | ComposerRowKind::Hotwords
            )
        );
        let shift = key.modifiers.contains(KeyModifiers::SHIFT);
        match key.code {
            KeyCode::Char('j') | KeyCode::Down if !shift => {
                if len > 0 {
                    composer.selected = (composer.selected + 1).min(len - 1);
                }
                Some(KeyOutcome::none())
            }
            KeyCode::Char('k') | KeyCode::Up if !shift => {
                composer.selected = composer.selected.saturating_sub(1);
                Some(KeyOutcome::none())
            }
            // Reorder / remove act only on chain members; on other rows fall
            // through so the generic `x` (delete profile) still works.
            KeyCode::Char('J') if is_member => {
                Some(outcome_to_keyoutcome(self.composer_result(|c| {
                    c.move_selected_member(profile_compose_write::MoveDir::Down)
                })))
            }
            KeyCode::Char('K') if is_member => {
                Some(outcome_to_keyoutcome(self.composer_result(|c| {
                    c.move_selected_member(profile_compose_write::MoveDir::Up)
                })))
            }
            KeyCode::Char('a') => {
                self.open_member_picker();
                Some(KeyOutcome::none())
            }
            KeyCode::Char('x') if is_member => Some(outcome_to_keyoutcome(
                self.composer_result(|c| c.remove_selected_member()),
            )),
            KeyCode::Char('X') => Some(outcome_to_keyoutcome(
                self.composer_result(|c| c.drop_all_invalid()),
            )),
            KeyCode::Char('D') if resettable => Some(outcome_to_keyoutcome(
                self.composer_result(|c| c.reset_selected()),
            )),
            KeyCode::Enter => {
                // Section headers / chain members are non-editable labels.
                let editable = composer
                    .rows()
                    .get(composer.selected)
                    .map(|r| {
                        !matches!(
                            r.kind,
                            ComposerRowKind::SectionHeader | ComposerRowKind::ChainMember { .. }
                        ) && r.row.editable
                    })
                    .unwrap_or(false);
                if editable {
                    self.begin_composer_edit();
                }
                Some(KeyOutcome::none())
            }
            _ => None,
        }
    }

    /// Run a composer mutation, mapping its `anyhow::Error` to the inline error
    /// display (reused verbatim from the field-edit path).
    fn composer_result(
        &mut self,
        f: impl FnOnce(&mut profile_composer::ProfileComposer) -> anyhow::Result<()>,
    ) -> EditOutcome {
        let Some(composer) = self.composer.as_mut() else {
            return EditOutcome::default();
        };
        match f(composer) {
            Ok(()) => {
                self.edit_error = None;
                EditOutcome {
                    reload_config: false,
                    status: Some(crate::t!("tui.configure.edit.saved_local")),
                }
            }
            Err(e) => {
                self.edit_error = Some(EditError {
                    field_path: crate::t!("tui.configure.profile"),
                    value: String::new(),
                    message: format!("{e:#}"),
                });
                EditOutcome::default()
            }
        }
    }

    /// Open the shared editor (inline or modal) for the selected composer row,
    /// targeting `EditTarget::Composer` so commit routes back to the composer.
    fn begin_composer_edit(&mut self) {
        let Some(composer) = self.composer.as_ref() else {
            return;
        };
        let Some(row) = composer.rows().get(composer.selected) else {
            return;
        };
        let field_path = row.row.field_path.clone();
        let control = row.row.control.clone();
        let secret = row.row.secret;
        let original = row.row.value.clone();
        self.edit_error = None;
        if let Some(kind) = ModalEditor::kind_for(&control, secret) {
            self.modal = Some(ModalEditor::new(
                field_path,
                EditTarget::Composer,
                kind,
                original,
            ));
        } else {
            self.editing = Some(EditState {
                field_path,
                target: EditTarget::Composer,
                control,
                cursor: original.len(),
                buffer: original.clone(),
                original,
            });
        }
    }

    fn open_member_picker(&mut self) {
        let ids = self
            .composer
            .as_ref()
            .map(|c| c.available_members())
            .unwrap_or_default();
        self.edit_error = None;
        self.member_picker = Some(MemberPicker { ids, selected: 0 });
    }

    pub fn feed_member_picker_key(&mut self, key: KeyEvent) -> KeyOutcome {
        let Some(picker) = self.member_picker.as_mut() else {
            return KeyOutcome::none();
        };
        let len = picker.ids.len();
        match key.code {
            KeyCode::Esc => {
                self.member_picker = None;
                KeyOutcome::none()
            }
            KeyCode::Char('j') | KeyCode::Down => {
                if len > 0 {
                    picker.selected = (picker.selected + 1).min(len - 1);
                }
                KeyOutcome::none()
            }
            KeyCode::Char('k') | KeyCode::Up => {
                picker.selected = picker.selected.saturating_sub(1);
                KeyOutcome::none()
            }
            KeyCode::Enter => {
                let id = picker.ids.get(picker.selected).cloned();
                self.member_picker = None;
                match id {
                    Some(id) => outcome_to_keyoutcome(self.composer_result(|c| c.add_member(&id))),
                    None => KeyOutcome::none(),
                }
            }
            _ => KeyOutcome::none(),
        }
    }

    pub fn commit_edit(&mut self) -> EditOutcome {
        let Some(edit) = self.editing.clone() else {
            return EditOutcome::default();
        };
        if edit.target.is_composer() {
            return self
                .commit_composer_edit(&edit, typed_input_for_control(&edit.control, &edit.buffer));
        }
        if let Some(key) = edit.target.draft_key() {
            // ASR draft 编辑内存文档，校验错误（越界等）走同一套内联错误弹窗。
            if let Some(Draft::Asr(form)) = self.draft.as_mut() {
                return match form.apply_edit(key, &edit.buffer) {
                    Ok(()) => {
                        self.editing = None;
                        self.edit_error = None;
                        EditOutcome::default()
                    }
                    Err(message) => {
                        self.set_edit_error(&edit, message);
                        EditOutcome::default()
                    }
                };
            }
            match self.draft.as_mut() {
                Some(Draft::Llm(form)) => {
                    form.edit(key, edit.buffer.clone());
                    form.on_changed(key);
                }
                Some(Draft::Profile(form)) => form.edit(key, edit.buffer.clone()),
                _ => {}
            }
            self.editing = None;
            self.edit_error = None;
            return EditOutcome::default();
        }
        let Some(source) = edit.target.file_path().cloned() else {
            return EditOutcome::default();
        };
        let Some(rel) = relative_rel_path(&source) else {
            return EditOutcome::default();
        };
        let spec = match crate::config::schema::spec_for_config_file(&source, &rel) {
            Some(Ok(spec)) => spec,
            Some(Err(e)) => {
                self.set_edit_error(&edit, format!("{e:#}"));
                return EditOutcome::default();
            }
            None => return EditOutcome::default(),
        };
        let Some(field) = spec.field_for_path(&edit.field_path) else {
            return EditOutcome::default();
        };
        let input = if rel.starts_with("asr/") && edit.field_path == "language_hints" {
            if edit.buffer == "auto" {
                Ok(TypedInput::StrArray(Vec::new()))
            } else {
                Ok(TypedInput::StrArray(vec![edit.buffer.clone()]))
            }
        } else {
            coerce_input(field.kind(), &edit.buffer)
        };
        let input = match input {
            Ok(input) => input,
            Err(message) => {
                self.set_edit_error(&edit, message);
                return EditOutcome::default();
            }
        };
        match field_write::set_field(&source, &edit.field_path, input, &spec) {
            Ok(()) => {
                self.editing = None;
                self.edit_error = None;
                self.refresh();
                let is_main = is_main_config(&source);
                let status = if is_main {
                    crate::t!("tui.configure.edit.saved")
                } else {
                    crate::t!("tui.configure.edit.saved_local")
                };
                EditOutcome {
                    reload_config: is_main,
                    status: Some(status),
                }
            }
            Err(e) => {
                self.set_edit_error(&edit, write_error_message(&e));
                EditOutcome::default()
            }
        }
    }

    pub fn reset_field_to_default(
        &mut self,
        field_path: &str,
        source: std::path::PathBuf,
    ) -> EditOutcome {
        let Some(rel) = relative_rel_path(&source) else {
            return EditOutcome::default();
        };
        let spec = match crate::config::schema::spec_for_config_file(&source, &rel) {
            Some(Ok(spec)) => spec,
            Some(Err(e)) => {
                self.edit_error = Some(EditError {
                    field_path: field_path.to_string(),
                    value: String::new(),
                    message: format!("{e:#}"),
                });
                return EditOutcome::default();
            }
            None => return EditOutcome::default(),
        };
        match field_write::unset_field(&source, field_path, &spec) {
            Ok(()) => {
                self.refresh();
                EditOutcome {
                    reload_config: is_main_config(&source),
                    status: Some(crate::t!("tui.configure.edit.reset")),
                }
            }
            Err(e) => {
                self.edit_error = Some(EditError {
                    field_path: field_path.to_string(),
                    value: String::new(),
                    message: write_error_message(&e),
                });
                EditOutcome::default()
            }
        }
    }

    fn set_edit_error(&mut self, edit: &EditState, message: String) {
        self.edit_error = Some(EditError {
            field_path: edit.field_path.clone(),
            value: edit.buffer.clone(),
            message,
        });
        // 保留编辑态、原值不动（不写盘）
    }

    pub fn is_editing(&self) -> bool {
        self.editing.is_some() || self.edit_error.is_some() || self.modal.is_some()
    }

    /// True while inline-editing a control whose value is chosen from a fixed
    /// set (toggle / select), where h/j/k/l cycle instead of typing.
    fn editing_is_choice(&self) -> bool {
        self.editing
            .as_ref()
            .is_some_and(|e| matches!(e.control, ControlKind::Toggle | ControlKind::Select(_)))
    }

    fn editing_has_presets(&self) -> bool {
        self.editing.as_ref().is_some_and(|edit| {
            matches!(
                edit.control,
                ControlKind::Select(_) | ControlKind::EditableSelect(_)
            )
        })
    }

    pub fn feed_edit_key(&mut self, key: crossterm::event::KeyEvent) -> EditOutcome {
        use crossterm::event::{KeyCode, KeyEventKind};
        if key.kind != KeyEventKind::Press {
            return EditOutcome::default();
        }
        if self.edit_error.is_some() {
            if matches!(key.code, KeyCode::Enter | KeyCode::Esc) {
                self.dismiss_error();
            }
            return EditOutcome::default();
        }
        match key.code {
            KeyCode::Esc => {
                self.cancel_edit();
                EditOutcome::default()
            }
            KeyCode::Enter => self.commit_edit(),
            KeyCode::Left => {
                if self.editing_is_choice() {
                    self.toggle_or_cycle(-1);
                } else {
                    self.move_cursor_left();
                }
                EditOutcome::default()
            }
            KeyCode::Right => {
                if self.editing_is_choice() {
                    self.toggle_or_cycle(1);
                } else {
                    self.move_cursor_right();
                }
                EditOutcome::default()
            }
            // Space cycles a choice; in text it is inserted (handled by Char(ch)).
            KeyCode::Char(' ') if self.editing_is_choice() => {
                self.toggle_or_cycle(1);
                EditOutcome::default()
            }
            KeyCode::Home => {
                self.move_cursor_home();
                EditOutcome::default()
            }
            KeyCode::End => {
                self.move_cursor_end();
                EditOutcome::default()
            }
            KeyCode::Delete => {
                self.delete_forward();
                EditOutcome::default()
            }
            // Up/Down cycle a choice too, so both arrow axes work like hjkl.
            KeyCode::Up if self.editing_has_presets() => {
                self.toggle_or_cycle(-1);
                EditOutcome::default()
            }
            KeyCode::Down if self.editing_has_presets() => {
                self.toggle_or_cycle(1);
                EditOutcome::default()
            }
            // Toggle/Select cycle with vim keys too (arrows aren't the only way).
            KeyCode::Char('h') | KeyCode::Char('k') if self.editing_is_choice() => {
                self.toggle_or_cycle(-1);
                EditOutcome::default()
            }
            KeyCode::Char('l') | KeyCode::Char('j') if self.editing_is_choice() => {
                self.toggle_or_cycle(1);
                EditOutcome::default()
            }
            KeyCode::Backspace => {
                self.backspace();
                EditOutcome::default()
            }
            // Toggle/Select 只能在预设值间循环，不接受自由输入。
            KeyCode::Char(_) if self.editing_is_choice() => EditOutcome::default(),
            KeyCode::Char(ch) => {
                self.push_char(ch);
                EditOutcome::default()
            }
            _ => EditOutcome::default(),
        }
    }

    pub fn feed_edit_paste(&mut self, text: &str) -> EditOutcome {
        if self.editing_is_choice() {
            return EditOutcome::default();
        }
        self.insert_text(text);
        EditOutcome::default()
    }

    pub fn commit_modal(&mut self) -> EditOutcome {
        let Some(m) = self.modal.clone() else {
            return EditOutcome::default();
        };
        if m.target.is_composer() {
            // Secret blank = cancel (never overwrite); else route to the composer.
            let Some(value) = m.value_to_save().map(str::to_string) else {
                self.modal = None;
                return EditOutcome {
                    reload_config: false,
                    status: Some(crate::t!("tui.configure.edit.unchanged")),
                };
            };
            let input = if m.kind == ModalKind::Array {
                TypedInput::StrArray(m.array_items())
            } else {
                TypedInput::Str(value)
            };
            let edit = EditState {
                field_path: m.field_path.clone(),
                target: EditTarget::Composer,
                control: ControlKind::Text,
                buffer: m.buffer.clone(),
                cursor: 0,
                original: String::new(),
            };
            // A modal edit lives in self.modal, not self.editing; close it here.
            // On failure commit_composer_edit set self.edit_error (error popup).
            let outcome = self.commit_composer_edit(&edit, input);
            self.modal = None;
            return outcome;
        }
        if let Some(key) = m.target.draft_key() {
            // ASR draft 的 secret/文本经内存文档写入；空白 secret（value_to_save=None）保持不变。
            if let Some(Draft::Asr(form)) = self.draft.as_mut() {
                if let Some(value) = m.value_to_save().map(str::to_string) {
                    if let Err(message) = form.apply_edit(key, &value) {
                        self.edit_error = Some(EditError {
                            field_path: m.field_path.clone(),
                            value: match m.kind {
                                ModalKind::Secret => crate::config::spec::SECRET_MASK.to_string(),
                                _ => m.buffer.clone(),
                            },
                            message,
                        });
                        self.modal = None;
                        return EditOutcome::default();
                    }
                }
                self.modal = None;
                self.edit_error = None;
                return EditOutcome::default();
            }
            match self.draft.as_mut() {
                Some(Draft::Llm(form)) => {
                    form.edit(key, m.buffer.clone());
                    form.on_changed(key);
                }
                Some(Draft::Profile(form)) => form.edit(key, m.buffer.clone()),
                _ => {}
            }
            self.modal = None;
            self.edit_error = None;
            return EditOutcome::default();
        }
        let Some(value) = m.value_to_save().map(str::to_string) else {
            self.modal = None;
            return EditOutcome {
                reload_config: false,
                status: Some(crate::t!("tui.configure.edit.unchanged")),
            };
        };
        let Some(source) = m.target.file_path().cloned() else {
            self.modal = None;
            return EditOutcome::default(); // Draft 目标在 Task 4 处理
        };
        let Some(rel) = relative_rel_path(&source) else {
            self.modal = None;
            return EditOutcome::default();
        };
        let spec = match crate::config::schema::spec_for_config_file(&source, &rel) {
            Some(Ok(spec)) => spec,
            Some(Err(e)) => {
                self.edit_error = Some(EditError {
                    field_path: m.field_path.clone(),
                    value: String::new(),
                    message: format!("{e:#}"),
                });
                self.modal = None;
                return EditOutcome::default();
            }
            None => {
                self.modal = None;
                return EditOutcome::default();
            }
        };
        let input = if m.kind == ModalKind::Array {
            TypedInput::StrArray(m.array_items())
        } else {
            TypedInput::Str(value)
        };
        match field_write::set_field(&source, &m.field_path, input, &spec) {
            Ok(()) => {
                self.modal = None;
                self.edit_error = None;
                self.refresh();
                let is_main = is_main_config(&source);
                let status = if is_main {
                    crate::t!("tui.configure.edit.saved")
                } else {
                    crate::t!("tui.configure.edit.saved_local")
                };
                EditOutcome {
                    reload_config: is_main,
                    status: Some(status),
                }
            }
            Err(e) => {
                self.edit_error = Some(EditError {
                    field_path: m.field_path.clone(),
                    value: match m.kind {
                        ModalKind::Secret => crate::config::spec::SECRET_MASK.to_string(),
                        _ => m.buffer.clone(),
                    },
                    message: write_error_message(&e),
                });
                self.modal = None; // error popup takes over
                EditOutcome::default()
            }
        }
    }

    pub fn delete_selected_profile(&mut self) -> EditOutcome {
        if self.module != ConfigureModule::Profile {
            return EditOutcome::default();
        }
        let Some(source) = self.selected_config_source() else {
            return EditOutcome {
                reload_config: false,
                status: Some(crate::t!("tui.configure.no_config_selected")),
            };
        };
        let Some(profile_id) = source.file_stem().and_then(|stem| stem.to_str()) else {
            return EditOutcome::default();
        };
        match crate::config::profile::delete_profile_file(
            &crate::config::paths::root_dir(),
            profile_id,
            &self.deleter,
        ) {
            Ok(path) => {
                self.edit_error = None;
                self.refresh();
                EditOutcome {
                    reload_config: false,
                    status: Some(crate::t!(
                        "tui.configure.profile_delete.deleted",
                        path = path.display()
                    )),
                }
            }
            Err(e) => {
                self.edit_error = Some(EditError {
                    field_path: crate::t!("tui.configure.profile_delete.profile"),
                    value: profile_id.to_string(),
                    message: format!("{e:#}"),
                });
                EditOutcome::default()
            }
        }
    }

    /// 请求删除当前选中项：不直接删，先置为待确认（下一个键 y 才真删）。
    pub fn request_delete(&mut self) -> KeyOutcome {
        if !matches!(
            self.module,
            ConfigureModule::Profile
                | ConfigureModule::AsrProvider
                | ConfigureModule::PostProcessor
        ) {
            return KeyOutcome::none();
        }
        let Some(source) = self.selected_config_source() else {
            return KeyOutcome::status(crate::t!("tui.configure.no_config_selected"));
        };
        let name = source
            .file_stem()
            .and_then(|stem| stem.to_str())
            .unwrap_or_default()
            .to_string();
        self.pending_delete = Some(name.clone());
        KeyOutcome::status(crate::i18n::tr(
            "tui.configure.delete_confirm.prompt",
            &[("name", name)],
        ))
    }

    /// 处理待确认删除的按键：y/Enter 确认，其它取消。返回 None 表示当前无待确认。
    fn resolve_pending_delete(&mut self, key: KeyEvent) -> Option<KeyOutcome> {
        self.pending_delete.take()?;
        match key.code {
            KeyCode::Char('y') | KeyCode::Char('Y') | KeyCode::Enter => {
                let outcome = match self.module {
                    ConfigureModule::Profile => self.delete_selected_profile(),
                    ConfigureModule::AsrProvider => self.delete_selected_asr_instance(),
                    ConfigureModule::PostProcessor => self.delete_selected_post_component(),
                    _ => EditOutcome::default(),
                };
                Some(outcome_to_keyoutcome(outcome))
            }
            _ => Some(KeyOutcome::status(crate::t!(
                "tui.configure.delete_confirm.cancelled"
            ))),
        }
    }

    pub fn delete_selected_asr_instance(&mut self) -> EditOutcome {
        if self.module != ConfigureModule::AsrProvider {
            return EditOutcome::default();
        }
        let Some(source) = self.selected_config_source() else {
            return EditOutcome {
                reload_config: false,
                status: Some(crate::t!("tui.configure.no_config_selected")),
            };
        };
        let Some(stem) = source.file_stem().and_then(|s| s.to_str()) else {
            return EditOutcome::default();
        };
        let stem = stem.to_string();
        match crate::config::profile::delete_asr_instance_file(
            &crate::config::paths::root_dir(),
            &stem,
            &self.deleter,
        ) {
            Ok(path) => {
                self.edit_error = None;
                // refresh() -> clamp_selected() 已按 supports_new 夹好来源索引，无需再手夹。
                self.refresh();
                self.selected = 0;
                EditOutcome {
                    reload_config: false,
                    status: Some(crate::i18n::tr(
                        "tui.configure.asr_delete.deleted",
                        &[("path", path.display().to_string())],
                    )),
                }
            }
            Err(e) => {
                self.edit_error = Some(EditError {
                    field_path: crate::t!("tui.configure.asr_delete.asr_instance"),
                    value: stem,
                    message: format!("{e:#}"),
                });
                EditOutcome::default()
            }
        }
    }

    pub fn delete_selected_post_component(&mut self) -> EditOutcome {
        if self.module != ConfigureModule::PostProcessor {
            return EditOutcome::default();
        }
        // +新建槽位没有文件；selected_config_source() 返回 None，走「未选中」提示。
        let Some(source) = self.selected_config_source() else {
            return EditOutcome {
                reload_config: false,
                status: Some(crate::t!("tui.configure.no_config_selected")),
            };
        };
        let stem = source
            .file_stem()
            .and_then(|s| s.to_str())
            .map(str::to_string);
        let Some(stem) = stem else {
            return EditOutcome {
                reload_config: false,
                status: Some(crate::t!("tui.configure.no_config_selected")),
            };
        };
        match crate::config::profile::delete_post_component_file(
            &crate::config::paths::root_dir(),
            &stem,
            &self.deleter,
        ) {
            Ok(_) => {
                self.edit_error = None;
                // refresh() -> clamp_selected() 已按 supports_new 夹好来源索引，无需再手夹。
                self.refresh();
                self.selected = 0;
                EditOutcome {
                    reload_config: false,
                    status: Some(crate::t!(
                        "tui.configure.component_delete.deleted",
                        path = source.display()
                    )),
                }
            }
            Err(e) => {
                self.edit_error = Some(EditError {
                    field_path: crate::t!("tui.configure.component_delete.component"),
                    value: source.display().to_string(),
                    message: format!("{e:#}"),
                });
                EditOutcome::default()
            }
        }
    }

    /// 若当前选中的是一个已存在的 llm 类型 post 组件，返回它的 chain id（bare stem）。
    /// 供普通模式下 `t` 测试连通性用。llm-ness 按文件 `type` 判断。
    pub fn selected_llm_component_id(&self) -> Option<String> {
        if self.module != ConfigureModule::PostProcessor {
            return None;
        }
        let source = self.selected_config_source()?;
        if source.parent()?.file_name()?.to_str()? != "post" {
            return None;
        }
        let name = source.file_stem()?.to_str()?;
        let root = source.parent()?.parent()?;
        if matches!(
            crate::config::post::resolve_kind_in_root(root, name),
            Some(crate::config::post::PostKind::Llm)
        ) {
            Some(name.to_string())
        } else {
            None
        }
    }

    pub fn selected_asr_instance_id(&self) -> Option<String> {
        if self.module != ConfigureModule::AsrProvider {
            return None;
        }
        let source = self.selected_config_source()?;
        if source.parent()?.file_name()?.to_str()? != "asr" {
            return None;
        }
        source.file_stem()?.to_str().map(str::to_string)
    }

    pub fn cancel_modal(&mut self) {
        self.modal = None;
    }

    pub fn feed_modal_key(&mut self, key: crossterm::event::KeyEvent) -> EditOutcome {
        use crossterm::event::{KeyCode, KeyEventKind, KeyModifiers};
        if key.kind != KeyEventKind::Press {
            return EditOutcome::default();
        }
        // Hotkey fields (`ModalKind::KeyCapture`) edit as plain text like any
        // single-line field: the daemon's global CGEventTap and the terminal
        // can't reliably capture this app's hotkey vocabulary (modifier-only
        // taps, `:double`, left/right sides, F13-F20), so the modal teaches the
        // syntax instead of pretending to capture live keys.
        let kind = self.modal.as_ref().map(|m| m.kind);

        match key.code {
            KeyCode::Esc => {
                self.cancel_modal();
                EditOutcome::default()
            }
            KeyCode::Char('s') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.commit_modal()
            }
            KeyCode::Char('y') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                if let Ok(text) = crate::platform::clipboard::read_string() {
                    if let Some(m) = &mut self.modal {
                        m.insert_str(&text);
                    }
                }
                EditOutcome::default()
            }
            KeyCode::Enter => {
                if matches!(kind, Some(ModalKind::Multiline)) {
                    if let Some(m) = &mut self.modal {
                        m.newline();
                    }
                    EditOutcome::default()
                } else {
                    self.commit_modal()
                }
            }
            KeyCode::Backspace => {
                if let Some(m) = &mut self.modal {
                    m.backspace();
                }
                EditOutcome::default()
            }
            KeyCode::Left => {
                if let Some(m) = &mut self.modal {
                    m.move_left();
                }
                EditOutcome::default()
            }
            KeyCode::Right => {
                if let Some(m) = &mut self.modal {
                    m.move_right();
                }
                EditOutcome::default()
            }
            KeyCode::Up => {
                if let Some(m) = &mut self.modal {
                    m.move_up();
                }
                EditOutcome::default()
            }
            KeyCode::Down => {
                if let Some(m) = &mut self.modal {
                    m.move_down();
                }
                EditOutcome::default()
            }
            KeyCode::Char(ch) => {
                if let Some(m) = &mut self.modal {
                    m.push_char(ch);
                }
                EditOutcome::default()
            }
            _ => EditOutcome::default(),
        }
    }

    pub fn feed_modal_paste(&mut self, text: &str) -> EditOutcome {
        if let Some(m) = &mut self.modal {
            m.insert_str(text);
        }
        EditOutcome::default()
    }

    fn selected_settings_row(&self) -> Option<&crate::tui::settings::SettingsRow> {
        let label = self.module.inventory_module().label();
        match self.module {
            ConfigureModule::Main | ConfigureModule::Overview => self
                .rows
                .iter()
                .filter(|r| r.group == label)
                .nth(self.selected),
            _ => {
                let source = self.selected_config_source()?;
                self.rows
                    .iter()
                    .filter(|r| r.group == label && std::path::Path::new(&r.source) == source)
                    .nth(self.selected)
            }
        }
    }

    fn selected_editable(
        &self,
    ) -> Option<(
        String,
        std::path::PathBuf,
        crate::config::field_view::ControlKind,
        String,
    )> {
        let row = self.selected_settings_row()?;
        if !row.editable {
            return None;
        }
        Some((
            row.field_path.clone(),
            std::path::PathBuf::from(&row.source),
            row.control.clone(),
            row.value.clone(),
        ))
    }

    fn selected_editable_set(&self) -> Option<(String, std::path::PathBuf)> {
        let row = self.selected_settings_row()?;
        // 只允许重置「已改且删掉后仍合法」的字段；required 无默认值的删不得。
        if !row.editable
            || !row.can_unset
            || row.origin != crate::config::field_view::FieldOrigin::Set
        {
            return None;
        }
        Some((
            row.field_path.clone(),
            std::path::PathBuf::from(&row.source),
        ))
    }

    /// Context-aware footer hints. Keep in sync with `feed_draft_key`,
    /// `feed_modal_key`, `feed_edit_key` and `on_key`.
    fn build_key_hints(&self) -> Vec<KeyHint> {
        if self.pending_delete.is_some() {
            return vec![
                KeyHint::new("y", "tui.hint.confirm"),
                KeyHint::new("Esc", "tui.hint.cancel"),
            ];
        }
        if self.draft_active() {
            let mut hints = vec![
                KeyHint::new("j/k", "tui.hint.move"),
                KeyHint::new("Enter", "tui.hint.edit"),
                KeyHint::new("^S", "tui.hint.save"),
                KeyHint::new("Esc", "tui.hint.cancel"),
            ];
            if self.draft_supports_test() {
                hints.insert(2, KeyHint::new("t", "tui.hint.test"));
                hints.insert(2, KeyHint::new("m", "tui.hint.models"));
            }
            return hints;
        }
        if self.modal.is_some() {
            return vec![
                KeyHint::new("^S", "tui.hint.save"),
                KeyHint::new("Esc", "tui.hint.cancel"),
            ];
        }
        if self.editing.is_some() || self.edit_error.is_some() {
            return vec![
                KeyHint::new("Enter", "tui.hint.save"),
                KeyHint::new("Esc", "tui.hint.cancel"),
            ];
        }
        let composer_kind = self.selected_composer_row_kind();
        let mut hints = self.navigation_hints();
        self.push_context_hints(&mut hints, composer_kind);
        self.push_module_action_hints(&mut hints, composer_kind);
        hints.push(KeyHint::new("e", "tui.hint.open_file"));
        hints.push(KeyHint::new("r", "tui.hint.reveal"));
        hints.push(KeyHint::new("v", "tui.hint.validate"));
        hints.push(KeyHint::new("R", "tui.hint.reload"));
        hints
    }

    fn navigation_hints(&self) -> Vec<KeyHint> {
        if self.focus == ConfigureFocus::Modules {
            return vec![
                KeyHint::new("j/k", "tui.hint.move"),
                KeyHint::new("l/Enter", "tui.hint.enter"),
            ];
        }

        let mut hints = vec![KeyHint::new("j/k", "tui.hint.move")];
        if !matches!(
            self.module,
            ConfigureModule::Main | ConfigureModule::Overview
        ) {
            hints.push(KeyHint::new("h/l", "tui.hint.source"));
        }
        if self.detail_max_scroll.get() > 0 {
            hints.push(KeyHint::new("PgUp/PgDn", "tui.hint.scroll"));
        }
        hints.push(KeyHint::new("Enter", "tui.hint.edit"));
        hints.push(KeyHint::new("Esc", "tui.hint.back"));
        hints
    }

    fn push_context_hints(
        &self,
        hints: &mut Vec<KeyHint>,
        composer_kind: Option<&profile_composer::ComposerRowKind>,
    ) {
        if composer_kind.is_some_and(composer_row_can_reset) {
            hints.push(KeyHint::new("D", "tui.hint.reset"));
            return;
        }

        if composer_kind.is_none()
            && self
                .selected_settings_row()
                .is_some_and(|row| row.origin == FieldOrigin::Set && row.editable && row.can_unset)
        {
            hints.push(KeyHint::new("D", "tui.hint.reset"));
        }
    }

    fn push_module_action_hints(
        &self,
        hints: &mut Vec<KeyHint>,
        composer_kind: Option<&profile_composer::ComposerRowKind>,
    ) {
        match self.module {
            ConfigureModule::PostProcessor => {
                hints.push(KeyHint::new("n", "tui.hint.new"));
                hints.push(KeyHint::new("x", "tui.hint.delete"));
                if self.selected_llm_component_id().is_some() {
                    hints.push(KeyHint::new("t", "tui.hint.test"));
                }
            }
            ConfigureModule::AsrProvider => {
                hints.push(KeyHint::new("n", "tui.hint.new"));
                hints.push(KeyHint::new("x", "tui.hint.delete"));
            }
            ConfigureModule::Profile => {
                hints.push(KeyHint::new("n", "tui.hint.new_profile"));
                if composer_kind.is_some() {
                    hints.push(KeyHint::new("a", "tui.hint.add_chain_member"));
                }
                if composer_kind.is_some_and(composer_row_is_chain_member) {
                    hints.push(KeyHint::new("Shift-J/K", "tui.hint.reorder"));
                }
                if self.composer_has_invalid_rows() {
                    hints.push(KeyHint::new("X", "tui.hint.drop_invalid"));
                }
                if composer_kind.is_some_and(composer_row_is_chain_member) {
                    hints.push(KeyHint::new("x", "tui.hint.remove_chain_member"));
                } else {
                    hints.push(KeyHint::new("x", "tui.hint.delete_profile"));
                }
            }
            ConfigureModule::Overview | ConfigureModule::Main => {}
        }
    }

    fn selected_composer_row_kind(&self) -> Option<&profile_composer::ComposerRowKind> {
        if self.module != ConfigureModule::Profile || self.focus != ConfigureFocus::Fields {
            return None;
        }
        let composer = self.composer.as_ref()?;
        composer.rows().get(composer.selected).map(|row| &row.kind)
    }

    fn composer_has_invalid_rows(&self) -> bool {
        self.module == ConfigureModule::Profile
            && self.focus == ConfigureFocus::Fields
            && self.composer.as_ref().is_some_and(|composer| {
                composer
                    .rows()
                    .iter()
                    .any(|row| row.row.origin == FieldOrigin::Error)
            })
    }
}

fn outcome_to_keyoutcome(outcome: EditOutcome) -> crate::tui::page::KeyOutcome {
    use crate::tui::page::KeyOutcome;
    let status = outcome.status.unwrap_or_default();
    if outcome.reload_config {
        KeyOutcome::command_and_status(crate::ipc::protocol::Command::ReloadConfig, status)
    } else if status.is_empty() {
        KeyOutcome::none()
    } else {
        KeyOutcome::status(status)
    }
}

/// Map an edited buffer to a `TypedInput` by the row's control shape. The
/// composer re-validates the value against the resolved schema, so a loose
/// coercion here (e.g. non-numeric text in a Number field) is caught there and
/// surfaced as an inline error rather than a silent bad write.
fn typed_input_for_control(control: &ControlKind, raw: &str) -> TypedInput {
    match control {
        ControlKind::Toggle => TypedInput::Bool(raw == "true"),
        ControlKind::Number { float, .. } => {
            if *float {
                raw.trim()
                    .parse::<f64>()
                    .map(TypedInput::Float)
                    .unwrap_or_else(|_| TypedInput::Str(raw.to_string()))
            } else {
                raw.trim()
                    .parse::<i64>()
                    .map(TypedInput::Integer)
                    .unwrap_or_else(|_| TypedInput::Str(raw.to_string()))
            }
        }
        ControlKind::Array => TypedInput::StrArray(
            raw.lines()
                .map(str::trim)
                .filter(|l| !l.is_empty())
                .map(str::to_string)
                .collect(),
        ),
        // Text / MultilineText / Select all commit a string (a Select commits the
        // chosen id). ReadOnly/KeyCapture rows are never opened for editing by the
        // composer, so they never reach this coercion.
        _ => TypedInput::Str(raw.to_string()),
    }
}

fn composer_row_can_reset(kind: &profile_composer::ComposerRowKind) -> bool {
    use profile_composer::ComposerRowKind;
    matches!(
        kind,
        ComposerRowKind::AsrOverride { .. }
            | ComposerRowKind::LlmOverride { .. }
            | ComposerRowKind::Name
            | ComposerRowKind::AsrInstance
            | ComposerRowKind::Hotwords
    )
}

fn composer_row_is_chain_member(kind: &profile_composer::ComposerRowKind) -> bool {
    matches!(kind, profile_composer::ComposerRowKind::ChainMember { .. })
}

fn coerce_input(kind: crate::config::spec::ValueKind, raw: &str) -> Result<TypedInput, String> {
    use crate::config::spec::ValueKind;
    match kind {
        ValueKind::Bool => Ok(TypedInput::Bool(raw == "true")),
        ValueKind::Integer => raw
            .trim()
            .parse::<i64>()
            .map(TypedInput::Integer)
            .map_err(|_| crate::t!("tui.configure.edit.not_integer")),
        ValueKind::Float => raw
            .trim()
            .parse::<f64>()
            .map(TypedInput::Float)
            .map_err(|_| crate::t!("tui.configure.edit.not_number")),
        _ => Ok(TypedInput::Str(raw.to_string())),
    }
}

fn write_error_message(e: &WriteError) -> String {
    match e {
        WriteError::Validation(d) => d
            .first()
            .map(|d| format!("{}: {}", d.path, d.message))
            .unwrap_or_else(|| crate::t!("tui.configure.edit.invalid")),
        WriteError::Semantic(m) => m.clone(),
        WriteError::Io(e) => e.to_string(),
    }
}

/// Compute (label, errors, missing) counts per module from inventory.
/// Called once per refresh so render never calls inventory::load() per frame.
fn compute_overview_counts() -> Vec<(String, usize, usize)> {
    use crate::config::inventory::{self, InventoryStatus};
    let inv = inventory::load();
    [
        ConfigureModule::Main,
        ConfigureModule::Profile,
        ConfigureModule::AsrProvider,
        ConfigureModule::PostProcessor,
    ]
    .iter()
    .map(|module| {
        let label = module.inventory_module().label();
        let errors = inv
            .entries()
            .filter(|e| e.module.label() == label && e.status == InventoryStatus::Error)
            .count();
        let missing = inv
            .entries()
            .filter(|e| e.module.label() == label && e.status == InventoryStatus::Missing)
            .count();
        (label.to_string(), errors, missing)
    })
    .collect()
}

/// Module order matching render.rs all_modules() — used in on_mouse for hit-target mapping.
fn all_modules_ordered() -> Vec<ConfigureModule> {
    vec![
        ConfigureModule::Main,
        ConfigureModule::Profile,
        ConfigureModule::AsrProvider,
        ConfigureModule::PostProcessor,
    ]
}

fn is_main_config(source: &std::path::Path) -> bool {
    source.file_name().and_then(|n| n.to_str()) == Some("config.toml")
}

fn relative_rel_path(path: &std::path::Path) -> Option<String> {
    let root = crate::config::paths::config_home().join("shuohua");
    if let Ok(rel) = path.strip_prefix(&root) {
        let parts: Vec<&str> = rel
            .components()
            .filter_map(|c| c.as_os_str().to_str())
            .collect();
        if !parts.is_empty() {
            return Some(parts.join("/"));
        }
    }
    // Fallback: locate a path component literally named "shuohua" (covers temp dirs / tests).
    let marker = std::path::Path::new("shuohua");
    let mut found = false;
    let mut parts = Vec::new();
    for component in path.components() {
        let text = component.as_os_str().to_str()?;
        if found {
            parts.push(text);
        } else if text == marker {
            found = true;
        }
    }
    if found && !parts.is_empty() {
        Some(parts.join("/"))
    } else {
        path.file_name()
            .and_then(|n| n.to_str())
            .map(str::to_string)
    }
}

impl Page for ConfigurePage {
    fn apply_event(&mut self, event: &Event, _active: bool) {
        if let Event::ConfigReloaded { .. } = event {
            self.refresh();
        }
    }

    fn on_key(&mut self, key: KeyEvent) -> KeyOutcome {
        if key.kind != KeyEventKind::Press {
            return KeyOutcome::none();
        }
        // 待确认删除时，先解决它（y 确认 / 其它取消），不进普通导航。
        if let Some(outcome) = self.resolve_pending_delete(key) {
            return outcome;
        }
        // 成员选择器打开时独占按键。
        if self.member_picker.is_some() {
            return self.feed_member_picker_key(key);
        }
        if self.draft_active() {
            return outcome_to_keyoutcome(self.feed_draft_key(key));
        }
        // Profile 模块的内容区由 composer 驱动：j/k 移行、Enter 编辑、D/a/x/X、
        // Shift-J/K 重排。其余键（h/l 切来源、Esc 返回、v/R/e/r/n 等）落回下面通用导航。
        if self.module == ConfigureModule::Profile
            && self.composer.is_some()
            && self.focus == ConfigureFocus::Fields
        {
            if let Some(outcome) = self.feed_composer_key(key) {
                return outcome;
            }
        }
        // 两级导航，进出用同一组语义：j/k 纵向移动（模块列表选模块、内容区选字段）；
        // 内容区里 h/l 横向切来源 tab，模块列表里 l 进入内容区；Esc 统一返回上一层。
        let in_content = self.focus == ConfigureFocus::Fields;
        match key.code {
            KeyCode::Char('j') | KeyCode::Down => self.move_selection(1),
            KeyCode::Char('k') | KeyCode::Up => self.move_selection(-1),
            KeyCode::Char('l') | KeyCode::Right => {
                if in_content {
                    self.switch_source(1);
                } else {
                    self.enter_content();
                }
            }
            KeyCode::Char('h') | KeyCode::Left if in_content => self.switch_source(-1),
            KeyCode::Esc if in_content => self.focus = ConfigureFocus::Modules,
            KeyCode::PageDown => self.scroll_detail(4),
            KeyCode::PageUp => self.scroll_detail(-4),
            // Vim-style half-page scroll of the detail pane — reachable on
            // laptops without dedicated PageUp/PageDown keys.
            KeyCode::Char('d') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.scroll_detail(4)
            }
            KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.scroll_detail(-4)
            }
            KeyCode::Char('v') => return KeyOutcome::status(self.validate()),
            KeyCode::Char('R') => {
                let (cmd, status) = self.request_reload();
                return KeyOutcome::command_and_status(cmd, status);
            }
            KeyCode::Char('n') if self.module.supports_new() => {
                return KeyOutcome::status(self.start_create_for_current_module());
            }
            KeyCode::Char('x')
                if matches!(
                    self.module,
                    ConfigureModule::Profile
                        | ConfigureModule::AsrProvider
                        | ConfigureModule::PostProcessor
                ) =>
            {
                return self.request_delete();
            }
            KeyCode::Char('e') => return KeyOutcome::status(self.open_selected_file()),
            KeyCode::Char('r') => return KeyOutcome::status(self.reveal_in_finder()),
            KeyCode::Enter => {
                if !in_content {
                    // 模块列表按 Enter → 进入右边内容区。
                    self.enter_content();
                } else if self.module.supports_new()
                    && self.selected_source_idx == self.sources_for_current_module().len()
                {
                    // 内容区停在末尾「+ New LLM…」槽位 → 进入新建。
                    return KeyOutcome::status(self.start_create_for_current_module());
                } else if let Some((field_path, source, control, original)) =
                    self.selected_editable()
                {
                    let secret = self
                        .selected_settings_row()
                        .map(|r| r.secret)
                        .unwrap_or(false);
                    if let Some(kind) = ModalEditor::kind_for(&control, secret) {
                        self.edit_error = None;
                        self.modal = Some(ModalEditor::new(
                            field_path,
                            EditTarget::File(source),
                            kind,
                            original,
                        ));
                    } else {
                        self.begin_edit_for(&field_path, source, control, &original);
                    }
                } else {
                    return KeyOutcome::status(crate::t!("tui.configure.edit.not_editable"));
                }
            }
            KeyCode::Char('D') => {
                if let Some((field_path, source)) = self.selected_editable_set() {
                    return outcome_to_keyoutcome(self.reset_field_to_default(&field_path, source));
                }
            }
            _ => {}
        }
        KeyOutcome::none()
    }

    fn on_enter(&mut self) {
        self.refresh();
    }

    fn key_hints(&self) -> Vec<KeyHint> {
        self.build_key_hints()
    }

    fn render(&self, frame: &mut Frame, area: Rect, theme: &TuiTheme, footer_status: &str) {
        render_page(frame, self, area, theme, footer_status);
    }
}

// ---- rendering ----

#[cfg(test)]
mod draft_tests {
    use super::*;

    #[test]
    fn rows_are_in_spec_order() {
        let form = LlmDraftForm::new();
        let keys: Vec<String> = form.rows().iter().map(|r| r.field_path.clone()).collect();
        assert_eq!(
            keys,
            vec![
                "preset",
                "file_id",
                "name",
                "base_url",
                "api_key",
                "model",
                "system_prompt",
                "prompt"
            ]
        );
    }

    #[test]
    fn profile_rows_are_in_spec_order_and_asr_is_select() {
        let dir =
            std::env::temp_dir().join(format!("shuohua-profile-draft-{}", ulid::Ulid::generate()));
        let root = dir.join("shuohua");
        std::fs::create_dir_all(root.join("asr")).unwrap();
        std::fs::write(root.join("asr/apple.toml"), "type = \"apple\"\n").unwrap();
        std::fs::write(root.join("asr/team.toml"), "type = \"doubao\"\n").unwrap();

        let form = ProfileDraftForm::new_in_root(&root);
        let rows = form.rows_in_root(&root);
        let keys: Vec<String> = rows.iter().map(|r| r.field_path.clone()).collect();
        assert_eq!(keys, vec!["file_id", "name", "asr_instance"]);
        assert!(rows.iter().all(|row| row.source == DRAFT_SOURCE));
        match &rows[2].control {
            ControlKind::Select(opts) => {
                assert_eq!(opts.as_slice(), ["apple".to_string(), "team".to_string()]);
            }
            other => panic!("asr_instance control should be Select, got {other:?}"),
        }
        assert_eq!(form.get("asr_instance"), "apple");

        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn profile_start_is_blocked_without_asr_instances() {
        crate::i18n::init("en-US");
        let dir =
            std::env::temp_dir().join(format!("shuohua-profile-draft-{}", ulid::Ulid::generate()));
        let root = dir.join("shuohua");
        std::fs::create_dir_all(&root).unwrap();

        let mut page = ConfigurePage::new();
        page.module = ConfigureModule::Profile;
        let status = page.start_profile_create_in_root(&root);

        assert!(status.contains("ASR"));
        assert!(page.draft.is_none());

        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn preset_row_is_a_select_over_registry_templates() {
        let form = LlmDraftForm::new();
        let preset = &form.rows()[0];
        match &preset.control {
            ControlKind::Select(opts) => {
                // registry 至少带 openai / anthropic / deepseek。
                assert!(opts.contains(&"openai".to_string()), "{opts:?}");
                assert!(opts.contains(&"anthropic".to_string()), "{opts:?}");
                assert!(opts.contains(&"deepseek".to_string()), "{opts:?}");
            }
            other => panic!("preset control should be Select, got {other:?}"),
        }
        assert_eq!(preset.value, "openai");
    }

    #[test]
    fn rows_color_only_fields_differing_from_default() {
        let mut form = LlmDraftForm::new();
        // 新建时所有字段都是默认，不该显示为「已改」（蓝）。
        assert!(form.rows().iter().all(|r| r.origin == FieldOrigin::Default));

        // 改成不同的值 → 蓝。
        form.edit("model", "gpt-test".to_string());
        let model_origin = |f: &LlmDraftForm| {
            f.rows()
                .iter()
                .find(|r| r.field_path == "model")
                .unwrap()
                .origin
        };
        assert_eq!(model_origin(&form), FieldOrigin::Set);

        // 改回与默认一模一样 → 不再算「已改」。
        let default_model = form
            .rows()
            .iter()
            .find(|r| r.field_path == "model")
            .unwrap()
            .default_value
            .clone();
        form.edit("model", default_model);
        assert_eq!(model_origin(&form), FieldOrigin::Default);
    }

    #[test]
    fn switching_preset_does_not_color_reseeded_fields() {
        let mut form = LlmDraftForm::new();
        form.edit("preset", "anthropic".to_string());
        form.on_changed("preset");
        let origin =
            |f: &LlmDraftForm, k: &str| f.rows().iter().find(|r| r.field_path == k).unwrap().origin;
        // 联动出的默认值不算「已改」。
        assert_eq!(origin(&form, "base_url"), FieldOrigin::Default);
        assert_eq!(origin(&form, "file_id"), FieldOrigin::Default);
        assert_eq!(origin(&form, "model"), FieldOrigin::Default);
        // 但 preset 本身是用户选的，算「已改」。
        assert_eq!(origin(&form, "preset"), FieldOrigin::Set);
    }

    #[test]
    fn prompt_rows_are_multiline() {
        let form = LlmDraftForm::new();
        let control = |k: &str| {
            form.rows()
                .iter()
                .find(|r| r.field_path == k)
                .unwrap()
                .control
                .clone()
        };
        assert_eq!(control("system_prompt"), ControlKind::MultilineText);
        assert_eq!(control("prompt"), ControlKind::MultilineText);
    }

    #[test]
    fn fetched_models_make_model_row_selectable_but_custom_value_still_allowed() {
        let mut form = LlmDraftForm::new();
        form.set_model_options(vec!["gpt-4.1-mini".to_string(), "gpt-5.5".to_string()]);
        let model = form
            .rows()
            .into_iter()
            .find(|r| r.field_path == "model")
            .unwrap();
        assert_eq!(
            model.control,
            ControlKind::Select(vec!["gpt-4.1-mini".to_string(), "gpt-5.5".to_string()])
        );

        form.edit("model", "custom-model".to_string());
        assert_eq!(form.get("model"), "custom-model");
    }

    #[test]
    fn inline_edit_cursor_moves_and_inserts_mid_string() {
        use crossterm::event::{KeyCode, KeyEvent};
        let mut page = ConfigurePage::new();
        page.draft = Some(Draft::Llm(Box::default()));
        page.editing = Some(EditState {
            field_path: "model".to_string(),
            target: EditTarget::Draft("model".to_string()),
            control: ControlKind::Text,
            cursor: "abc".len(),
            buffer: "abc".to_string(),
            original: "abc".to_string(),
        });
        // 光标左移两格，在中间插入 X。
        page.feed_edit_key(KeyEvent::from(KeyCode::Left));
        page.feed_edit_key(KeyEvent::from(KeyCode::Left));
        page.feed_edit_key(KeyEvent::from(KeyCode::Char('X')));
        assert_eq!(page.editing.as_ref().unwrap().buffer, "aXbc");
        // 退格删掉光标前的 X。
        page.feed_edit_key(KeyEvent::from(KeyCode::Backspace));
        assert_eq!(page.editing.as_ref().unwrap().buffer, "abc");
        // Home 到行首插入，End 到行尾。
        page.feed_edit_key(KeyEvent::from(KeyCode::Home));
        page.feed_edit_key(KeyEvent::from(KeyCode::Char('_')));
        assert_eq!(page.editing.as_ref().unwrap().buffer, "_abc");
    }

    #[test]
    fn inline_edit_paste_inserts_at_cursor() {
        use crossterm::event::{KeyCode, KeyEvent};
        let mut page = ConfigurePage::new();
        page.draft = Some(Draft::Llm(Box::default()));
        page.editing = Some(EditState {
            field_path: "model".to_string(),
            target: EditTarget::Draft("model".to_string()),
            control: ControlKind::Text,
            cursor: "ab".len(),
            buffer: "ab".to_string(),
            original: "ab".to_string(),
        });

        page.feed_edit_key(KeyEvent::from(KeyCode::Left));
        page.feed_edit_paste("中文");

        let edit = page.editing.as_ref().unwrap();
        assert_eq!(edit.buffer, "a中文b");
        assert_eq!(edit.cursor, "a中文".len());
    }

    #[test]
    fn api_key_row_is_secret() {
        let form = LlmDraftForm::new();
        let api_key = form
            .rows()
            .into_iter()
            .find(|r| r.field_path == "api_key")
            .unwrap();
        assert!(api_key.secret);
    }

    #[test]
    fn switching_preset_reseeds_provider_fields_but_keeps_prompt() {
        let mut form = LlmDraftForm::new();
        form.edit("prompt", "keep me".to_string());
        form.edit("preset", "anthropic".to_string());
        form.on_changed("preset");

        let get = |k: &str| {
            form.rows()
                .iter()
                .find(|r| r.field_path == k)
                .unwrap()
                .value
                .clone()
        };
        assert_eq!(get("base_url"), "https://api.anthropic.com");
        assert_eq!(get("file_id"), "anthropic");
        assert_eq!(get("name"), "anthropic");
        assert_eq!(get("model"), "claude-haiku-4-5"); // provider 默认模型
        assert_eq!(get("prompt"), "keep me"); // 与 provider 无关，保留
    }

    #[test]
    fn switching_asr_type_reseeds_doc_and_follows_default_file_id() {
        crate::i18n::init("en-US");
        let mut form = AsrDraftDoc::new();
        let first_kind = asr_kind_ids()[0].clone();
        assert_eq!(form.get("type"), first_kind);
        assert_eq!(form.get("file_id"), first_kind);

        // 切换 type 重铺内存文档，并（未自定义 file_id 时）让默认文件名跟随实现。
        form.apply_edit("type", "doubao").unwrap();
        assert_eq!(form.get("type"), "doubao");
        assert_eq!(form.get("file_id"), "doubao");
        assert!(form.rows().iter().any(|r| r.field_path == "resource_id"));

        // 一旦自定义 file_id，再切 type 不覆盖用户输入。
        form.apply_edit("file_id", "work").unwrap();
        form.apply_edit("type", "tencent").unwrap();
        assert_eq!(form.get("type"), "tencent");
        assert_eq!(form.get("file_id"), "work");
        assert!(form
            .rows()
            .iter()
            .any(|r| r.field_path == "engine_model_type"));
    }

    #[test]
    fn switching_preset_to_deepseek_fills_base_url_and_model() {
        let mut form = LlmDraftForm::new();
        form.edit("preset", "deepseek".to_string());
        form.on_changed("preset");
        let get = |k: &str| {
            form.rows()
                .iter()
                .find(|r| r.field_path == k)
                .unwrap()
                .value
                .clone()
        };
        // deepseek 是 openai 兼容：base_url/model 自动填好，用户只需填 api_key。
        assert!(get("base_url").contains("deepseek"), "{}", get("base_url"));
        assert!(!get("model").is_empty());
    }

    #[test]
    fn draft_navigation_and_field_edit_via_shared_editors() {
        use crossterm::event::{KeyCode, KeyEvent};
        let mut page = ConfigurePage::new();
        page.start_llm_create();
        assert!(page.draft_active());

        page.feed_draft_key(KeyEvent::from(KeyCode::Char('j')));
        assert_eq!(page.llm_draft().unwrap().selected, 1);

        // Enter 打开 file_id 的内联编辑（Text → EditState，target=Draft）
        page.feed_draft_key(KeyEvent::from(KeyCode::Enter));
        assert!(page.editing.is_some());
        assert_eq!(
            page.editing.as_ref().unwrap().target,
            EditTarget::Draft("file_id".to_string())
        );
    }

    #[test]
    fn draft_prompt_opens_modal_editor() {
        use crossterm::event::{KeyCode, KeyEvent};
        let mut page = ConfigurePage::new();
        page.start_llm_create();
        // 移到最后一行（prompt，多行）。
        let last = page.llm_draft().unwrap().rows().len() - 1;
        for _ in 0..last {
            page.feed_draft_key(KeyEvent::from(KeyCode::Char('j')));
        }
        page.feed_draft_key(KeyEvent::from(KeyCode::Enter));
        assert!(page.modal.is_some());
        assert_eq!(
            page.modal.as_ref().unwrap().target,
            EditTarget::Draft("prompt".to_string())
        );
    }

    #[test]
    fn draft_esc_cancels() {
        use crossterm::event::{KeyCode, KeyEvent};
        let mut page = ConfigurePage::new();
        page.start_llm_create();
        page.feed_draft_key(KeyEvent::from(KeyCode::Esc));
        assert!(!page.draft_active());
    }

    #[test]
    fn draft_test_config_reflects_draft_fields() {
        let mut page = ConfigurePage::new();
        page.start_llm_create();
        page.llm_draft_mut()
            .unwrap()
            .edit("model", "gpt-x".to_string());
        let cfg = page.draft_test_config().unwrap();
        assert_eq!(cfg.model, "gpt-x");
        assert!(matches!(
            cfg.format,
            crate::post::llm::ProviderFormat::OpenAi
        ));
    }

    #[test]
    fn test_result_sets_status_and_edit_resets_it() {
        let mut page = ConfigurePage::new();
        page.start_llm_create();
        page.set_draft_testing();
        assert_eq!(
            page.llm_draft().unwrap().test_status,
            DraftTestStatus::Testing
        );
        page.set_draft_test_result(Err("401".to_string()));
        assert_eq!(
            page.llm_draft().unwrap().test_status,
            DraftTestStatus::Failed("401".to_string())
        );
        // 再改字段 → 结果作废回到 Idle。
        page.llm_draft_mut()
            .unwrap()
            .edit("api_key", "sk".to_string());
        assert_eq!(page.llm_draft().unwrap().test_status, DraftTestStatus::Idle);
    }

    #[test]
    fn draft_h_leaves_back_to_content() {
        use crossterm::event::{KeyCode, KeyEvent};
        let mut page = ConfigurePage::new();
        page.start_llm_create();
        page.feed_draft_key(KeyEvent::from(KeyCode::Char('h')));
        assert!(!page.draft_active());
        assert_eq!(page.focus, ConfigureFocus::Fields);
    }

    #[test]
    fn draft_esc_returns_to_modules() {
        // Esc 统一返回左边模块列表，而不是停在内容区某个来源上。
        use crossterm::event::{KeyCode, KeyEvent};
        let mut page = ConfigurePage::new();
        page.start_llm_create();
        page.feed_draft_key(KeyEvent::from(KeyCode::Esc));
        assert!(!page.draft_active());
        assert_eq!(page.focus, ConfigureFocus::Modules);
    }

    #[test]
    fn editing_draft_field_writes_back_to_draft_not_disk() {
        use crossterm::event::{KeyCode, KeyEvent};
        let mut page = ConfigurePage::new();
        page.draft = Some(Draft::Llm(Box::default()));
        // 模拟已 Enter 进入 base_url 内联编辑（target=Draft）
        page.editing = Some(EditState {
            field_path: "base_url".to_string(),
            target: EditTarget::Draft("base_url".to_string()),
            control: ControlKind::Text,
            cursor: "https://example.test/v1".len(),
            buffer: "https://example.test/v1".to_string(),
            original: "https://api.openai.com/v1".to_string(),
        });
        let out = page.feed_edit_key(KeyEvent::from(KeyCode::Enter));
        assert!(!out.reload_config);
        assert!(page.editing.is_none());
        assert_eq!(
            page.llm_draft().unwrap().get("base_url"),
            "https://example.test/v1"
        );
    }

    #[test]
    fn commit_writes_valid_component_file() {
        let dir = std::env::temp_dir().join(format!("shuohua-draft-{}", ulid::Ulid::generate()));
        let post = dir.join("post");
        let mut form = LlmDraftForm::new();
        form.edit("file_id", "my_openai".to_string());
        form.edit("name", "my-openai".to_string());
        form.edit("model", "gpt-test".to_string());

        let path = form.commit(&post).unwrap();
        assert_eq!(path, post.join("my_openai.toml"));
        let body = std::fs::read_to_string(&path).unwrap();
        assert!(body.contains("format = \"openai\""));
        assert!(body.contains("name = \"my-openai\""));
        toml::from_str::<toml::Value>(&body).unwrap();
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn commit_rejects_empty_model() {
        let dir = std::env::temp_dir().join(format!("shuohua-draft-{}", ulid::Ulid::generate()));
        let post = dir.join("post");
        let mut form = LlmDraftForm::new();
        form.edit("model", String::new());
        let err = form.commit(&post).unwrap_err();
        assert!(format!("{err:#}").contains("model"));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn asr_draft_rows_are_type_file_id_then_schema_fields() {
        crate::i18n::init("en-US");
        let mut form = AsrDraftDoc::new();
        form.apply_edit("type", "doubao").unwrap();
        let rows = form.rows();
        assert_eq!(rows[0].field_path, "type");
        assert_eq!(rows[1].field_path, "file_id");
        match &rows[0].control {
            ControlKind::Select(opts) => {
                for kind in ["apple", "doubao", "aliyun", "tencent"] {
                    assert!(opts.contains(&kind.to_string()), "{opts:?} missing {kind}");
                }
            }
            other => panic!("type control should be Select, got {other:?}"),
        }
        assert_eq!(rows[1].control, ControlKind::Text);
        // schema 字段跟在后面，控件由 field_view 派生（与编辑已落盘文件一致）。
        let resource = rows.iter().find(|r| r.field_path == "resource_id").unwrap();
        assert!(matches!(resource.control, ControlKind::EditableSelect(_)));
        let app_key = rows.iter().find(|r| r.field_path == "app_key").unwrap();
        assert!(app_key.secret);
    }

    #[test]
    fn asr_draft_fill_then_commit_writes_complete_file() {
        crate::i18n::init("en-US");
        let dir =
            std::env::temp_dir().join(format!("shuohua-asr-draft-{}", ulid::Ulid::generate()));
        let asr = dir.join("asr");
        let mut form = AsrDraftDoc::new();
        form.apply_edit("type", "doubao").unwrap();
        form.apply_edit("file_id", "my_doubao").unwrap();
        // 新建==编辑：secret 等字段在 draft 里填完，^S 一次落盘完整文件。
        form.apply_edit("app_key", "app-test").unwrap();
        form.apply_edit("access_key", "access-test").unwrap();

        let path = form.commit(&asr).unwrap();

        assert_eq!(path, asr.join("my_doubao.toml"));
        let body = std::fs::read_to_string(&path).unwrap();
        assert!(body.contains("type = \"doubao\""), "{body}");
        assert!(body.contains("app_key = \"app-test\""), "{body}");
        assert!(body.contains("access_key = \"access-test\""), "{body}");
        assert!(
            body.contains("resource_id = \"volc.seedasr.sauc.duration\""),
            "{body}"
        );
        toml::from_str::<crate::config::asr::doubao::DoubaoConfig>(&body).unwrap();
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn asr_draft_field_edit_validates_like_file_write() {
        crate::i18n::init("en-US");
        let mut form = AsrDraftDoc::new();
        form.apply_edit("type", "doubao").unwrap();
        // stream_mode 范围 0..=2：越界应被 apply_field 的 Error 级校验拒绝，不写内存。
        let err = form.apply_edit("stream_mode", "9").unwrap_err();
        assert!(err.contains("stream_mode"), "{err}");
        assert!(form
            .rows()
            .iter()
            .any(|r| r.field_path == "stream_mode" && r.value == "2"));
    }

    #[test]
    fn asr_draft_commit_refuses_duplicate_file() {
        crate::i18n::init("en-US");
        let dir =
            std::env::temp_dir().join(format!("shuohua-asr-draft-{}", ulid::Ulid::generate()));
        let asr = dir.join("asr");
        let form = AsrDraftDoc::new();
        form.commit(&asr).unwrap();
        let err = form.commit(&asr).unwrap_err().to_string();
        assert!(err.contains("already exists"), "{err}");
        let _ = std::fs::remove_dir_all(dir);
    }
}
