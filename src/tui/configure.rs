use std::path::PathBuf;
use std::process::Command as ProcessCommand;

use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph, Wrap};
use ratatui::Frame;

use crate::config::inventory::InventoryStatus;
use crate::config::template::LlmComponentDraft;
use crate::config::theme::TuiTheme;
use crate::ipc::protocol::{Command, Event};
use crate::tui::config_actions;
use crate::tui::page::{KeyOutcome, Page};
use crate::tui::settings::{self, SettingsRow};

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
        if self.llm_wizard.is_none() {
            return None;
        }
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
        if self.llm_wizard.is_some() {
            render_wizard(frame, self, area, theme, footer_status);
            return;
        }

        let body = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([
                Constraint::Length(24),
                Constraint::Percentage(44),
                Constraint::Percentage(56),
            ])
            .split(area);

        frame.render_widget(
            Paragraph::new(module_nav_lines(self, theme)).block(
                block_for_focus(self, ConfigureFocus::Modules, theme)
                    .title(crate::t!("tui.configure.modules"))
                    .borders(Borders::ALL),
            ),
            body[0],
        );

        if self.module == ConfigureModule::Overview {
            render_overview(
                frame,
                self,
                Rect::new(
                    body[1].x,
                    body[1].y,
                    body[1].width + body[2].width,
                    body[1].height,
                ),
                theme,
                footer_status,
            );
        } else if self.module == ConfigureModule::Main {
            frame.render_widget(
                Paragraph::new(item_lines(self, theme))
                    .wrap(Wrap { trim: false })
                    .block(
                        block_for_focus(self, ConfigureFocus::Items, theme)
                            .title(focused_title(
                                self,
                                ConfigureFocus::Items,
                                self.module.title(),
                            ))
                            .borders(Borders::ALL),
                    ),
                Rect::new(
                    body[1].x,
                    body[1].y,
                    body[1].width + body[2].width,
                    body[1].height,
                ),
            );
        } else {
            frame.render_widget(
                Paragraph::new(item_lines(self, theme))
                    .wrap(Wrap { trim: false })
                    .block(
                        block_for_focus(self, ConfigureFocus::Items, theme)
                            .title(focused_title(
                                self,
                                ConfigureFocus::Items,
                                self.module.title(),
                            ))
                            .borders(Borders::ALL),
                    ),
                body[1],
            );
            frame.render_widget(
                Paragraph::new(detail_lines(self, theme))
                    .wrap(Wrap { trim: false })
                    .block(
                        block_for_focus(self, ConfigureFocus::Items, theme)
                            .title(crate::t!("tui.configure.detail"))
                            .borders(Borders::ALL),
                    ),
                body[2],
            );
        }
    }
}

// ---- rendering ----

fn render_wizard(
    frame: &mut Frame,
    page: &ConfigurePage,
    area: Rect,
    theme: &TuiTheme,
    footer_status: &str,
) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(0), Constraint::Length(4)])
        .split(area);
    frame.render_widget(
        Paragraph::new(wizard_text(page))
            .wrap(Wrap { trim: false })
            .block(
                Block::default()
                    .title(crate::t!("tui.configure.wizard.title"))
                    .borders(Borders::ALL),
            ),
        chunks[0],
    );
    frame.render_widget(
        Paragraph::new(status_lines(page, theme, footer_status))
            .wrap(Wrap { trim: false })
            .block(
                Block::default()
                    .title(crate::t!("tui.configure.status"))
                    .borders(Borders::ALL),
            ),
        chunks[1],
    );
}

fn render_overview(
    frame: &mut Frame,
    page: &ConfigurePage,
    area: Rect,
    theme: &TuiTheme,
    footer_status: &str,
) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(9), Constraint::Min(0)])
        .split(area);
    frame.render_widget(
        Paragraph::new(overview_lines(page, theme, footer_status))
            .wrap(Wrap { trim: false })
            .block(
                Block::default()
                    .title(crate::t!("tui.configure.main"))
                    .borders(Borders::ALL),
            ),
        chunks[0],
    );
    frame.render_widget(
        Paragraph::new(main_grouped_lines(page, theme))
            .wrap(Wrap { trim: false })
            .block(
                block_for_focus(page, ConfigureFocus::Items, theme)
                    .title(ConfigureModule::Main.title())
                    .borders(Borders::ALL),
            ),
        chunks[1],
    );
}

fn module_nav_lines(page: &ConfigurePage, theme: &TuiTheme) -> Vec<Line<'static>> {
    all_modules()
        .into_iter()
        .map(|module| {
            let selected = module == page.module;
            let count = module_entry_count(page, module);
            let marker = if selected {
                if page.focus == ConfigureFocus::Modules {
                    "> "
                } else {
                    "* "
                }
            } else {
                "  "
            };
            let style = if selected {
                Style::default()
                    .fg(ui::accent(theme))
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(ui::segment(theme))
            };
            Line::from(vec![
                Span::styled(marker, style),
                Span::styled(module.title(), style),
                Span::raw(" "),
                Span::styled(format!("{count:>2}"), Style::default().fg(ui::muted(theme))),
            ])
        })
        .collect()
}

fn focused_title(page: &ConfigurePage, focus: ConfigureFocus, title: String) -> String {
    if page.focus == focus {
        format!("> {title}")
    } else {
        title
    }
}

fn block_for_focus(
    page: &ConfigurePage,
    focus: ConfigureFocus,
    theme: &TuiTheme,
) -> Block<'static> {
    if page.focus == focus {
        Block::default().border_style(Style::default().fg(ui::accent(theme)))
    } else {
        Block::default()
            .border_style(Style::default().fg(ui::muted(theme)))
            .title_style(Style::default().fg(ui::muted(theme)))
    }
}

fn all_modules() -> Vec<ConfigureModule> {
    vec![
        ConfigureModule::Overview,
        ConfigureModule::Profile,
        ConfigureModule::AsrProvider,
        ConfigureModule::PostProcessor,
    ]
}

fn module_entry_count(page: &ConfigurePage, module: ConfigureModule) -> usize {
    let label = if module == ConfigureModule::Overview {
        ConfigureModule::Main.inventory_module().label()
    } else {
        module.inventory_module().label()
    };
    page.rows.iter().filter(|row| row.group == label).count()
}

fn wizard_text(page: &ConfigurePage) -> String {
    let Some(wizard) = &page.llm_wizard else {
        return String::new();
    };
    let template_lines = wizard
        .templates
        .iter()
        .enumerate()
        .map(|(idx, id)| {
            let marker =
                if wizard.step == LlmWizardStep::Template && idx == wizard.selected_template {
                    ">"
                } else {
                    " "
                };
            format!("{marker} {id}")
        })
        .collect::<Vec<_>>()
        .join("\n");
    format!(
        "{}\n{}\n\n{}\n{}\n{}\n{}\n{}\n{}\n\n{}",
        crate::t!("tui.configure.wizard.title"),
        wizard_step_label(wizard.step),
        template_lines,
        wizard_field_line(
            LlmWizardStep::FileId,
            wizard.step,
            crate::t!("tui.configure.wizard.file_id"),
            &wizard.draft.file_id
        ),
        wizard_field_line(
            LlmWizardStep::ProviderName,
            wizard.step,
            crate::t!("tui.configure.wizard.provider_name"),
            &wizard.draft.provider_name
        ),
        wizard_field_line(
            LlmWizardStep::Format,
            wizard.step,
            crate::t!("tui.configure.wizard.format"),
            &wizard.draft.format
        ),
        wizard_field_line(
            LlmWizardStep::BaseUrl,
            wizard.step,
            crate::t!("tui.configure.wizard.base_url"),
            &wizard.draft.base_url
        ),
        wizard_field_line(
            LlmWizardStep::Model,
            wizard.step,
            crate::t!("tui.configure.wizard.model"),
            &wizard.draft.model
        ),
        crate::t!("tui.configure.wizard.no_profile_attach")
    )
}

fn wizard_step_label(step: LlmWizardStep) -> String {
    let key = match step {
        LlmWizardStep::Template => "tui.configure.wizard.step_template",
        LlmWizardStep::FileId => "tui.configure.wizard.step_file_id",
        LlmWizardStep::ProviderName => "tui.configure.wizard.step_provider_name",
        LlmWizardStep::Format => "tui.configure.wizard.step_format",
        LlmWizardStep::BaseUrl => "tui.configure.wizard.step_base_url",
        LlmWizardStep::Model => "tui.configure.wizard.step_model",
    };
    crate::i18n::tr(key, &[])
}

fn wizard_field_line(
    field: LlmWizardStep,
    current: LlmWizardStep,
    label: String,
    value: &str,
) -> String {
    let marker = if field == current { ">" } else { " " };
    format!("{marker} {label}: {value}")
}

fn item_lines(page: &ConfigurePage, theme: &TuiTheme) -> Vec<Line<'static>> {
    if matches!(
        page.module,
        ConfigureModule::Profile | ConfigureModule::PostProcessor | ConfigureModule::AsrProvider
    ) {
        return source_lines(page, theme);
    }
    if page.module == ConfigureModule::Main {
        return main_grouped_lines(page, theme);
    }
    field_lines(page.rows_for_current_module(), None, theme)
}

fn main_grouped_lines(page: &ConfigurePage, theme: &TuiTheme) -> Vec<Line<'static>> {
    let rows = page
        .rows
        .iter()
        .filter(|row| row.group == ConfigureModule::Main.inventory_module().label())
        .collect::<Vec<_>>();
    if rows.is_empty() {
        return vec![Line::from(crate::t!("tui.configure.no_entries"))];
    }
    let mut lines = Vec::new();
    let mut current_section = String::new();
    for row in rows {
        let (section, item_key) = split_main_display_key(&row.display_key);
        if section != current_section {
            if !lines.is_empty() {
                lines.push(Line::from(""));
            }
            current_section = section.clone();
            lines.push(Line::styled(
                section,
                Style::default()
                    .fg(ui::accent(theme))
                    .add_modifier(Modifier::BOLD),
            ));
        }
        lines.push(field_line(row, Some(&item_key), false, theme));
    }
    lines
}

fn split_main_display_key(key: &str) -> (String, String) {
    key.split_once('.')
        .map(|(section, rest)| (section.to_string(), rest.to_string()))
        .unwrap_or_else(|| ("root".to_string(), key.to_string()))
}

fn source_lines(page: &ConfigurePage, theme: &TuiTheme) -> Vec<Line<'static>> {
    let sources = page.sources_for_current_module();
    if sources.is_empty() {
        return vec![Line::from(crate::t!("tui.configure.no_entries"))];
    }
    sources
        .iter()
        .enumerate()
        .map(|(idx, source)| {
            let selected = idx == page.selected;
            let row_count = page
                .rows_for_current_module()
                .into_iter()
                .filter(|row| std::path::Path::new(&row.source) == source)
                .count();
            let marker_style = if selected {
                Style::default()
                    .fg(ui::accent(theme))
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(ui::muted(theme))
            };
            Line::from(vec![
                Span::styled(if selected { "> " } else { "  " }, marker_style),
                Span::styled(
                    format!("{row_count:>2}"),
                    Style::default().fg(ui::success(theme)),
                ),
                Span::raw(" "),
                Span::styled(
                    source_name(source),
                    if selected {
                        Style::default()
                            .fg(ui::accent(theme))
                            .add_modifier(Modifier::BOLD)
                    } else {
                        Style::default().fg(ui::fg(theme))
                    },
                ),
            ])
        })
        .collect()
}

fn field_lines(
    rows: Vec<&SettingsRow>,
    selected: Option<usize>,
    theme: &TuiTheme,
) -> Vec<Line<'static>> {
    if rows.is_empty() {
        return vec![Line::from(crate::t!("tui.configure.no_entries"))];
    }
    rows.iter()
        .enumerate()
        .map(|(idx, row)| {
            let is_selected = selected.is_some_and(|s| s == idx);
            field_line(row, None, is_selected, theme)
        })
        .collect()
}

fn field_line(
    row: &SettingsRow,
    key_override: Option<&str>,
    selected: bool,
    theme: &TuiTheme,
) -> Line<'static> {
    let display_key = key_override.unwrap_or(&row.display_key);
    Line::from(vec![
        Span::styled(
            if selected { "> " } else { "" }.to_string(),
            Style::default().fg(if selected {
                ui::accent(theme)
            } else {
                ui::muted(theme)
            }),
        ),
        Span::styled(
            status_glyph(row.status),
            Style::default().fg(status_color(row.status, theme)),
        ),
        Span::raw(" "),
        Span::styled(
            format!("{:<24}", truncate_display(display_key, 24)),
            Style::default().fg(if selected {
                ui::accent(theme)
            } else {
                ui::fg(theme)
            }),
        ),
        Span::raw(" "),
        Span::styled(
            format!("{:<24}", truncate_display(&compact_value(&row.value), 24)),
            Style::default().fg(ui::segment(theme)),
        ),
        Span::raw("  "),
        Span::styled(
            row.description_key
                .map(|key| crate::i18n::tr(key, &[]))
                .unwrap_or_default(),
            Style::default().fg(ui::muted(theme)),
        ),
    ])
}

fn compact_value(value: &str) -> String {
    value.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn detail_lines(page: &ConfigurePage, theme: &TuiTheme) -> Vec<Line<'static>> {
    let rows = match page.module {
        ConfigureModule::Profile
        | ConfigureModule::PostProcessor
        | ConfigureModule::AsrProvider => selected_source_rows(page),
        _ => selected_row(page).map(|row| vec![row]).unwrap_or_default(),
    };
    if rows.is_empty() {
        return vec![Line::from(crate::t!("tui.configure.no_config_selected"))];
    }
    let source = rows
        .first()
        .map(|row| row.source.clone())
        .unwrap_or_else(|| "-".to_string());
    let mut lines = vec![
        kv_line("source path", source, ui::warning(theme)),
        Line::from(""),
    ];
    lines.extend(detail_field_lines(rows, theme));
    lines
}

fn detail_field_lines(rows: Vec<&SettingsRow>, theme: &TuiTheme) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    for row in rows {
        let description = row
            .description_key
            .map(|key| crate::i18n::tr(key, &[]))
            .unwrap_or_default();
        if row.value.contains('\n') || display_width(&row.value) > 56 {
            lines.push(Line::from(vec![
                Span::styled(
                    row.display_key.clone(),
                    Style::default()
                        .fg(ui::accent(theme))
                        .add_modifier(Modifier::BOLD),
                ),
                Span::raw("  "),
                Span::styled(description, Style::default().fg(ui::muted(theme))),
            ]));
            lines.extend(text_lines(row.value.clone()));
        } else {
            lines.push(Line::from(vec![
                Span::styled(
                    format!("{:<24}", truncate_display(&row.display_key, 24)),
                    Style::default().fg(ui::accent(theme)),
                ),
                Span::styled(
                    format!("{:<24}", row.value.clone()),
                    Style::default().fg(ui::segment(theme)),
                ),
                Span::raw("  "),
                Span::styled(description, Style::default().fg(ui::muted(theme))),
            ]));
        }
    }
    lines
}

fn selected_source_rows(page: &ConfigurePage) -> Vec<&SettingsRow> {
    let Some(source) = page.selected_config_source() else {
        return Vec::new();
    };
    let module = page.module.inventory_module();
    page.rows
        .iter()
        .filter(|row| row.group == module.label() && std::path::Path::new(&row.source) == source)
        .collect()
}

fn selected_row(page: &ConfigurePage) -> Option<&SettingsRow> {
    let module = page.module.inventory_module();
    page.rows
        .iter()
        .filter(|row| row.group == module.label())
        .nth(page.selected)
}

fn overview_lines(
    page: &ConfigurePage,
    theme: &TuiTheme,
    footer_status: &str,
) -> Vec<Line<'static>> {
    let config_path = crate::config::default_path().display().to_string();
    let mut lines = vec![
        kv_line("config root", config_path, ui::warning(theme)),
        Line::from(""),
        Line::from(vec![
            label_span("module", theme),
            Span::raw("        "),
            label_span("items", theme),
            Span::raw("  "),
            label_span("errors", theme),
            Span::raw("  "),
            label_span("missing", theme),
        ]),
    ];
    for module in all_modules()
        .into_iter()
        .filter(|module| *module != ConfigureModule::Overview)
    {
        let label = module.inventory_module().label();
        let rows = page
            .rows
            .iter()
            .filter(|row| row.group == label)
            .collect::<Vec<_>>();
        let errors = rows
            .iter()
            .filter(|row| row.status == InventoryStatus::Error)
            .count();
        let missing = rows
            .iter()
            .filter(|row| row.status == InventoryStatus::Missing)
            .count();
        lines.push(Line::from(vec![
            Span::styled(
                format!("{:<13}", module.title()),
                Style::default().fg(ui::accent(theme)),
            ),
            Span::styled(
                format!("{:>5}", rows.len()),
                Style::default().fg(ui::fg(theme)),
            ),
            Span::styled(
                format!("{:>8}", errors),
                Style::default().fg(status_count_color(errors, theme)),
            ),
            Span::styled(
                format!("{:>9}", missing),
                Style::default().fg(status_count_color(missing, theme)),
            ),
        ]));
    }
    lines.push(Line::from(""));
    lines.extend(status_lines(page, theme, footer_status));
    lines
}

fn status_lines(page: &ConfigurePage, theme: &TuiTheme, footer_status: &str) -> Vec<Line<'static>> {
    vec![
        kv_line("doctor", doctor_status_value(page), ui::success(theme)),
        kv_line("reload/status", footer_status.to_string(), ui::fg(theme)),
        kv_line("actions", hint_text(page), ui::muted(theme)),
    ]
}

fn doctor_status_value(page: &ConfigurePage) -> String {
    match &page.doctor.status {
        Some(status) => status.clone(),
        None => crate::t!("tui.configure.doctor_not_run"),
    }
}

fn hint_text(page: &ConfigurePage) -> String {
    if page.llm_wizard.is_some() {
        crate::t!("tui.configure.wizard.hint")
    } else if page.module == ConfigureModule::PostProcessor {
        crate::t!("tui.configure.refresh_hint_post")
    } else {
        crate::t!("tui.configure.refresh_hint")
    }
}

fn status_glyph(status: InventoryStatus) -> &'static str {
    match status {
        InventoryStatus::Ok => "ok",
        InventoryStatus::Warning => "!!",
        InventoryStatus::Error => "xx",
        InventoryStatus::Missing => "--",
    }
}

fn status_color(status: InventoryStatus, theme: &TuiTheme) -> Color {
    match status {
        InventoryStatus::Ok => ui::success(theme),
        InventoryStatus::Warning => ui::warning(theme),
        InventoryStatus::Error => ui::error(theme),
        InventoryStatus::Missing => ui::muted(theme),
    }
}

fn status_count_color(count: usize, theme: &TuiTheme) -> Color {
    if count == 0 {
        ui::muted(theme)
    } else {
        ui::error(theme)
    }
}

fn source_name(path: &std::path::Path) -> String {
    path.file_stem()
        .and_then(|s| s.to_str())
        .map(ToOwned::to_owned)
        .unwrap_or_else(|| path.display().to_string())
}

fn run_doctor() -> DoctorState {
    let output = ProcessCommand::new(std::env::current_exe().unwrap_or_else(|_| "shuo".into()))
        .arg("doctor")
        .output();
    match output {
        Ok(output) => {
            let mut text = String::new();
            text.push_str(&String::from_utf8_lossy(&output.stdout));
            if !output.stderr.is_empty() {
                if !text.is_empty() && !text.ends_with('\n') {
                    text.push('\n');
                }
                text.push_str(&String::from_utf8_lossy(&output.stderr));
            }
            DoctorState {
                ran_once: true,
                status: Some(if output.status.success() {
                    "ok".to_string()
                } else {
                    format!("exit {}", output.status)
                }),
                output: text,
            }
        }
        Err(e) => DoctorState {
            ran_once: true,
            status: Some("error".to_string()),
            output: format!("failed to run doctor: {e}"),
        },
    }
}

// ---- shared UI helpers (mirrored from panes.rs; will dedupe after all pages split) ----

mod ui {
    use ratatui::style::Color;

    use crate::config::theme::TuiTheme;

    fn rgb(value: u32) -> Color {
        Color::Rgb(
            ((value >> 16) & 0xff) as u8,
            ((value >> 8) & 0xff) as u8,
            (value & 0xff) as u8,
        )
    }

    pub fn fg(theme: &TuiTheme) -> Color {
        rgb(theme.foreground)
    }
    pub fn muted(theme: &TuiTheme) -> Color {
        rgb(theme.muted)
    }
    pub fn accent(theme: &TuiTheme) -> Color {
        rgb(theme.accent)
    }
    pub fn success(theme: &TuiTheme) -> Color {
        rgb(theme.success)
    }
    pub fn warning(theme: &TuiTheme) -> Color {
        rgb(theme.warning)
    }
    pub fn error(theme: &TuiTheme) -> Color {
        rgb(theme.error)
    }
    pub fn segment(theme: &TuiTheme) -> Color {
        rgb(theme.segment)
    }
}

fn char_display_width(ch: char) -> usize {
    if ch.is_ascii() {
        1
    } else {
        2
    }
}

fn display_width(value: &str) -> usize {
    value.chars().map(char_display_width).sum()
}

fn truncate_display(value: &str, max_chars: usize) -> String {
    let mut out = value.chars().take(max_chars).collect::<String>();
    if value.chars().count() > max_chars && max_chars > 0 {
        out.pop();
        out.push('…');
    }
    out
}

fn text_lines(text: String) -> Vec<Line<'static>> {
    text.lines()
        .map(|line| Line::from(line.to_string()))
        .collect()
}

fn kv_line(label: impl Into<String>, value: impl Into<String>, color: Color) -> Line<'static> {
    Line::from(vec![
        Span::styled(
            format!("{}: ", label.into()),
            Style::default().fg(Color::DarkGray),
        ),
        value_span(value.into(), color),
    ])
}

fn label_span(text: impl Into<String>, theme: &TuiTheme) -> Span<'static> {
    Span::styled(text.into(), Style::default().fg(ui::muted(theme)))
}

fn value_span(text: impl Into<String>, color: Color) -> Span<'static> {
    Span::styled(text.into(), Style::default().fg(color))
}

// ---- tests ----

#[cfg(test)]
mod tests {
    use super::*;

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
}
