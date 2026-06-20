use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph, Wrap};
use ratatui::Frame;

use crate::config::inventory::InventoryStatus;
use crate::config::theme::TuiTheme;
use crate::tui::configure::{ConfigureFocus, ConfigureModule, ConfigurePage, LlmWizardStep};
use crate::tui::settings::SettingsRow;
use crate::tui::ui;

pub(super) fn render_page(
    frame: &mut Frame,
    page: &ConfigurePage,
    area: Rect,
    theme: &TuiTheme,
    footer_status: &str,
) {
    if page.llm_wizard.is_some() {
        render_wizard(frame, page, area, theme, footer_status);
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
        Paragraph::new(module_nav_lines(page, theme)).block(
            block_for_focus(page, ConfigureFocus::Modules, theme)
                .title(crate::t!("tui.configure.modules"))
                .borders(Borders::ALL),
        ),
        body[0],
    );

    if page.module == ConfigureModule::Overview {
        render_overview(
            frame,
            page,
            Rect::new(
                body[1].x,
                body[1].y,
                body[1].width + body[2].width,
                body[1].height,
            ),
            theme,
            footer_status,
        );
    } else if page.module == ConfigureModule::Main {
        frame.render_widget(
            Paragraph::new(item_lines(page, theme))
                .wrap(Wrap { trim: false })
                .block(
                    block_for_focus(page, ConfigureFocus::Items, theme)
                        .title(focused_title(
                            page,
                            ConfigureFocus::Items,
                            page.module.title(),
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
            Paragraph::new(item_lines(page, theme))
                .wrap(Wrap { trim: false })
                .block(
                    block_for_focus(page, ConfigureFocus::Items, theme)
                        .title(focused_title(
                            page,
                            ConfigureFocus::Items,
                            page.module.title(),
                        ))
                        .borders(Borders::ALL),
                ),
            body[1],
        );
        frame.render_widget(
            Paragraph::new(detail_lines(page, theme))
                .wrap(Wrap { trim: false })
                .block(
                    block_for_focus(page, ConfigureFocus::Items, theme)
                        .title(crate::t!("tui.configure.detail"))
                        .borders(Borders::ALL),
                ),
            body[2],
        );
    }
}

pub(super) fn render_wizard(
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

pub(super) fn render_overview(
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

pub(super) fn module_nav_lines(page: &ConfigurePage, theme: &TuiTheme) -> Vec<Line<'static>> {
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

pub(super) fn item_lines(page: &ConfigurePage, theme: &TuiTheme) -> Vec<Line<'static>> {
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

pub(super) fn main_grouped_lines(page: &ConfigurePage, theme: &TuiTheme) -> Vec<Line<'static>> {
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

pub(super) fn split_main_display_key(key: &str) -> (String, String) {
    key.split_once('.')
        .map(|(section, rest)| (section.to_string(), rest.to_string()))
        .unwrap_or_else(|| ("root".to_string(), key.to_string()))
}

pub(super) fn source_lines(page: &ConfigurePage, theme: &TuiTheme) -> Vec<Line<'static>> {
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

pub(super) fn field_lines(
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

pub(super) fn detail_lines(page: &ConfigurePage, theme: &TuiTheme) -> Vec<Line<'static>> {
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

pub(super) fn selected_source_rows(page: &ConfigurePage) -> Vec<&SettingsRow> {
    let Some(source) = page.selected_config_source() else {
        return Vec::new();
    };
    let module = page.module.inventory_module();
    page.rows
        .iter()
        .filter(|row| row.group == module.label() && std::path::Path::new(&row.source) == source)
        .collect()
}

pub(super) fn selected_row(page: &ConfigurePage) -> Option<&SettingsRow> {
    let module = page.module.inventory_module();
    page.rows
        .iter()
        .filter(|row| row.group == module.label())
        .nth(page.selected)
}

pub(super) fn overview_lines(
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

// ---- shared UI helpers ----

fn char_display_width(ch: char) -> usize {
    if ch.is_ascii() {
        1
    } else {
        2
    }
}

pub(super) fn display_width(value: &str) -> usize {
    value.chars().map(char_display_width).sum()
}

pub(super) fn truncate_display(value: &str, max_chars: usize) -> String {
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
