use std::path::PathBuf;

use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui::layout::Rect;
use ratatui::Frame;

use crate::config::template::LlmComponentDraft;
use crate::config::theme::TuiTheme;
use crate::ipc::protocol::{Command, Event};
use crate::tui::config_actions;
use crate::tui::configure::doctor::run_doctor;
use crate::tui::configure::render::render_page;
use crate::tui::page::{KeyOutcome, Page};
use crate::tui::settings::{self, SettingsRow};

mod doctor;
mod render;

#[cfg(test)]
mod tests;

// ---- types ----

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
            Self::Overview => crate::config::inventory::InventoryModule::Overview,
            Self::Main => crate::config::inventory::InventoryModule::Main,
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
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfigureFocus {
    Modules,
    Items,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LlmWizardStep {
    Template,
    FileId,
    ProviderName,
    Format,
    BaseUrl,
    Model,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LlmWizard {
    pub step: LlmWizardStep,
    pub templates: Vec<String>,
    pub selected_template: usize,
    pub draft: LlmComponentDraft,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DoctorState {
    pub ran_once: bool,
    pub status: Option<String>,
    pub output: String,
}

enum WizardEdit {
    Push(char),
    Backspace,
    Clear,
}

// ---- page state ----

#[derive(Debug)]
pub struct ConfigurePage {
    pub rows: Vec<SettingsRow>,
    pub selected: usize,
    pub module: ConfigureModule,
    pub focus: ConfigureFocus,
    pub llm_wizard: Option<LlmWizard>,
    pub doctor: DoctorState,
}

impl ConfigurePage {
    pub fn new() -> Self {
        Self {
            rows: settings::load_rows(),
            selected: 0,
            module: ConfigureModule::Overview,
            focus: ConfigureFocus::Modules,
            llm_wizard: None,
            doctor: DoctorState {
                ran_once: false,
                status: None,
                output: String::new(),
            },
        }
    }

    pub fn refresh(&mut self) {
        self.rows = settings::load_rows();
        self.clamp_selected();
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

    pub fn open_editor(&self) -> String {
        let Some(path) = self.selected_config_source() else {
            return crate::t!("tui.configure.no_config_selected");
        };
        match config_actions::open_in_editor(&path) {
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

    pub fn start_wizard(&mut self) -> String {
        let templates = crate::config::template::llm_templates()
            .map(|template| template.id.to_string())
            .collect::<Vec<_>>();
        let Some(first_template) = templates.first() else {
            return crate::t!("tui.configure.wizard.no_templates");
        };
        let Some(draft) = crate::config::template::llm_draft_from_template(first_template) else {
            return crate::t!("tui.configure.wizard.no_templates");
        };
        self.llm_wizard = Some(LlmWizard {
            step: LlmWizardStep::Template,
            templates,
            selected_template: 0,
            draft,
        });
        crate::t!("tui.configure.wizard.started")
    }

    pub fn is_wizard_active(&self) -> bool {
        self.llm_wizard.is_some()
    }

    pub fn feed_wizard_key(&mut self, key: KeyEvent) -> Option<String> {
        self.llm_wizard.as_ref()?;
        if key.kind != KeyEventKind::Press {
            return None;
        }
        match key.code {
            KeyCode::Esc => {
                self.llm_wizard = None;
                Some(crate::t!("tui.configure.wizard.cancelled"))
            }
            KeyCode::Enter => self.advance_wizard(),
            KeyCode::Down | KeyCode::Right => {
                self.move_wizard_selection(1);
                None
            }
            KeyCode::Up | KeyCode::Left => {
                self.move_wizard_selection(-1);
                None
            }
            KeyCode::Char('j') | KeyCode::Char('l') if self.wizard_allows_selection() => {
                self.move_wizard_selection(1);
                None
            }
            KeyCode::Char('k') | KeyCode::Char('h') if self.wizard_allows_selection() => {
                self.move_wizard_selection(-1);
                None
            }
            KeyCode::Backspace => {
                self.edit_wizard_field(WizardEdit::Backspace);
                None
            }
            KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.edit_wizard_field(WizardEdit::Clear);
                None
            }
            KeyCode::Char(ch) => {
                self.edit_wizard_field(WizardEdit::Push(ch));
                None
            }
            _ => None,
        }
    }

    pub fn move_selection(&mut self, delta: isize) {
        if self.focus == ConfigureFocus::Modules {
            self.module = if delta >= 0 {
                self.module.next()
            } else {
                self.module.prev()
            };
            self.selected = 0;
            self.clamp_selected();
            return;
        }
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

    pub fn move_focus(&mut self, delta: isize) {
        self.focus = if delta >= 0 {
            ConfigureFocus::Items
        } else {
            ConfigureFocus::Modules
        };
    }

    pub fn move_top(&mut self) {
        self.selected = 0;
    }

    pub fn move_bottom(&mut self) {
        let len = self.current_len();
        self.selected = len.saturating_sub(1);
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
                .get(self.selected)
                .cloned(),
        }
    }

    fn config_directory(&self) -> Option<PathBuf> {
        crate::config::default_path()
            .parent()
            .map(|path| path.to_path_buf())
    }

    fn current_len(&self) -> usize {
        match self.module {
            ConfigureModule::Profile
            | ConfigureModule::PostProcessor
            | ConfigureModule::AsrProvider => self.sources_for_current_module().len(),
            _ => self.rows_for_current_module().len(),
        }
    }

    fn clamp_selected(&mut self) {
        let len = self.current_len();
        self.selected = self.selected.min(len.saturating_sub(1));
    }

    fn wizard_allows_selection(&self) -> bool {
        self.llm_wizard.as_ref().is_some_and(|wizard| {
            matches!(wizard.step, LlmWizardStep::Template | LlmWizardStep::Format)
        })
    }

    fn move_wizard_selection(&mut self, delta: isize) {
        let Some(wizard) = &mut self.llm_wizard else {
            return;
        };
        match wizard.step {
            LlmWizardStep::Template => {
                let len = wizard.templates.len();
                if len == 0 {
                    return;
                }
                let next = (wizard.selected_template as isize + delta).rem_euclid(len as isize);
                wizard.selected_template = next as usize;
                if let Some(template_id) = wizard.templates.get(wizard.selected_template) {
                    if let Some(draft) =
                        crate::config::template::llm_draft_from_template(template_id)
                    {
                        wizard.draft = draft;
                    }
                }
            }
            LlmWizardStep::Format => {
                wizard.draft.format = if wizard.draft.format == "openai" {
                    "anthropic".to_string()
                } else {
                    "openai".to_string()
                };
            }
            _ => {}
        }
    }

    fn edit_wizard_field(&mut self, edit: WizardEdit) {
        let Some(wizard) = &mut self.llm_wizard else {
            return;
        };
        let target = match wizard.step {
            LlmWizardStep::FileId => Some(&mut wizard.draft.file_id),
            LlmWizardStep::ProviderName => Some(&mut wizard.draft.provider_name),
            LlmWizardStep::BaseUrl => Some(&mut wizard.draft.base_url),
            LlmWizardStep::Model => Some(&mut wizard.draft.model),
            _ => None,
        };
        let Some(value) = target else {
            return;
        };
        match edit {
            WizardEdit::Push(ch) => value.push(ch),
            WizardEdit::Backspace => {
                value.pop();
            }
            WizardEdit::Clear => value.clear(),
        }
    }

    fn advance_wizard(&mut self) -> Option<String> {
        let Some(wizard) = &mut self.llm_wizard else {
            return None;
        };
        wizard.step = match wizard.step {
            LlmWizardStep::Template => LlmWizardStep::FileId,
            LlmWizardStep::FileId => LlmWizardStep::ProviderName,
            LlmWizardStep::ProviderName => LlmWizardStep::Format,
            LlmWizardStep::Format => LlmWizardStep::BaseUrl,
            LlmWizardStep::BaseUrl => LlmWizardStep::Model,
            LlmWizardStep::Model => return self.finish_wizard(),
        };
        None
    }

    fn finish_wizard(&mut self) -> Option<String> {
        let wizard = self.llm_wizard.take()?;
        match crate::config::template::create_llm_component(
            &crate::config::post::default_dir(),
            &wizard.draft,
        ) {
            Ok(path) => {
                self.refresh();
                let status = crate::t!("tui.configure.wizard.created", path = path.display());
                let _ = config_actions::open_in_editor(&path);
                Some(status)
            }
            Err(e) => {
                let status = crate::t!("tui.configure.wizard.error", error = e);
                self.llm_wizard = Some(wizard);
                Some(status)
            }
        }
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
        match key.code {
            KeyCode::Char('j') | KeyCode::Down => self.move_selection(1),
            KeyCode::Char('k') | KeyCode::Up => self.move_selection(-1),
            KeyCode::Char('g') => self.move_top(),
            KeyCode::Char('G') => self.move_bottom(),
            KeyCode::Char('l') | KeyCode::Right => self.move_focus(1),
            KeyCode::Char('h') | KeyCode::Left => self.move_focus(-1),
            KeyCode::Char('v') => return KeyOutcome::status(self.validate()),
            KeyCode::Char('R') => {
                let (cmd, status) = self.request_reload();
                return KeyOutcome::command_and_status(cmd, status);
            }
            KeyCode::Char('n') if self.module == ConfigureModule::PostProcessor => {
                return KeyOutcome::status(self.start_wizard());
            }
            KeyCode::Char('o') => return KeyOutcome::status(self.open_editor()),
            KeyCode::Char('r') => return KeyOutcome::status(self.reveal_in_finder()),
            _ => {}
        }
        KeyOutcome::none()
    }

    fn on_enter(&mut self) {
        self.refresh();
    }

    fn render(&self, frame: &mut Frame, area: Rect, theme: &TuiTheme, footer_status: &str) {
        render_page(frame, self, area, theme, footer_status);
    }
}

// ---- rendering ----
