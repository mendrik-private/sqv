use std::cmp::Reverse;

use fuzzy_matcher::{skim::SkimMatcherV2, FuzzyMatcher};
use ratatui::{
    layout::{Constraint, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{block::BorderType, Block, Borders, Paragraph},
    Frame,
};

use crate::{symbols::Symbols, theme::Theme};

#[derive(Debug, Clone, PartialEq)]
pub enum PaletteCommand {
    SwitchTable(String),
    ExportCsv,
    ExportJson,
    ExportSql,
    CopyCell,
    CopyRowJson,
    ReloadSchema,
    ToggleSidebar,
    ToggleReadonly,
    ResetColumnWidths,
    ClearFilters,
    Quit,
}

impl PaletteCommand {
    pub fn label(&self) -> &'static str {
        match self {
            PaletteCommand::SwitchTable(_) => "Switch Table",
            PaletteCommand::ExportCsv => "Export CSV",
            PaletteCommand::ExportJson => "Export JSON",
            PaletteCommand::ExportSql => "Export SQL",
            PaletteCommand::CopyCell => "Copy cell",
            PaletteCommand::CopyRowJson => "Copy row(s) as JSON",
            PaletteCommand::ReloadSchema => "Reload schema",
            PaletteCommand::ToggleSidebar => "Toggle sidebar",
            PaletteCommand::ToggleReadonly => "Toggle read-only",
            PaletteCommand::ResetColumnWidths => "Reset column widths",
            PaletteCommand::ClearFilters => "Clear filters",
            PaletteCommand::Quit => "Quit",
        }
    }
}

pub struct CommandPaletteState {
    pub query: String,
    pub commands: Vec<PaletteCommand>,
    pub selected: usize,
}

impl CommandPaletteState {
    pub fn new(table_names: Vec<String>) -> Self {
        let mut commands = vec![
            PaletteCommand::ExportCsv,
            PaletteCommand::ExportJson,
            PaletteCommand::ExportSql,
            PaletteCommand::CopyCell,
            PaletteCommand::CopyRowJson,
            PaletteCommand::ReloadSchema,
            PaletteCommand::ToggleSidebar,
            PaletteCommand::ToggleReadonly,
            PaletteCommand::ResetColumnWidths,
            PaletteCommand::ClearFilters,
            PaletteCommand::Quit,
        ];
        for name in table_names {
            commands.push(PaletteCommand::SwitchTable(name));
        }
        Self {
            query: String::new(),
            commands,
            selected: 0,
        }
    }

    pub fn filtered(&self) -> Vec<(usize, &PaletteCommand, Vec<usize>)> {
        if self.query.is_empty() {
            return self
                .commands
                .iter()
                .enumerate()
                .map(|(i, c)| (i, c, vec![]))
                .collect();
        }
        let matcher = SkimMatcherV2::default();
        let mut results: Vec<(usize, &PaletteCommand, i64, Vec<usize>)> = self
            .commands
            .iter()
            .enumerate()
            .filter_map(|(i, c)| {
                let label = match c {
                    PaletteCommand::SwitchTable(name) => format!("Switch Table: {}", name),
                    _ => c.label().to_string(),
                };
                matcher
                    .fuzzy_indices(&label, &self.query)
                    .map(|(score, indices)| (i, c, score, indices))
            })
            .collect();
        results.sort_by_key(|result| Reverse(result.2));
        results
            .into_iter()
            .map(|(i, c, _, idx)| (i, c, idx))
            .collect()
    }

    pub fn move_up(&mut self) {
        self.selected = self.selected.saturating_sub(1);
    }

    pub fn move_down(&mut self) {
        let n = self.filtered().len();
        if n > 0 && self.selected + 1 < n {
            self.selected += 1;
        }
    }

    pub fn selected_command(&self) -> Option<PaletteCommand> {
        self.filtered()
            .get(self.selected)
            .map(|(_, c, _)| (*c).clone())
    }

    pub fn push_char(&mut self, ch: char) {
        self.query.push(ch);
        self.selected = 0;
    }

    pub fn pop_char(&mut self) {
        self.query.pop();
        self.selected = 0;
    }
}

pub fn render(
    frame: &mut Frame,
    area: Rect,
    state: &CommandPaletteState,
    theme: &Theme,
    symbols: &Symbols,
) {
    let popup_w = (area.width * 6 / 10).max(50).min(area.width);
    let popup_h = (area.height / 2).max(12).min(area.height);
    let x = area.x + (area.width.saturating_sub(popup_w)) / 2;
    let y = area.y + area.height / 4;
    let popup_area = Rect {
        x,
        y,
        width: popup_w,
        height: popup_h,
    };

    super::paint_popup_surface(frame, popup_area, theme);

    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(theme.accent))
        .title(" Command Palette ")
        .style(Style::default().bg(theme.bg_raised));
    let inner = block.inner(popup_area);
    frame.render_widget(block, popup_area);

    let chunks = Layout::vertical([Constraint::Length(1), Constraint::Min(0)]).split(inner);

    let query_line = Line::from(vec![
        Span::styled(" > ", Style::default().fg(theme.accent)),
        Span::styled(
            format!("{}{}", state.query, symbols.cursor),
            Style::default().fg(theme.fg),
        ),
    ]);
    frame.render_widget(
        Paragraph::new(query_line).style(Style::default().bg(theme.bg_raised)),
        chunks[0],
    );

    let filtered = state.filtered();
    let list_area = chunks[1];
    let visible = list_area.height as usize;
    let start = if state.selected >= visible {
        state.selected - visible + 1
    } else {
        0
    };

    for (view_i, (_, cmd, matched_chars)) in filtered.iter().skip(start).take(visible).enumerate() {
        let row_y = list_area.y + view_i as u16;
        let abs_i = view_i + start;
        let is_sel = abs_i == state.selected;

        let bg = if is_sel {
            theme.bg_soft
        } else {
            theme.bg_raised
        };

        let label = match cmd {
            PaletteCommand::SwitchTable(name) => format!("Switch Table: {}", name),
            _ => cmd.label().to_string(),
        };

        let buf = frame.buffer_mut();
        let line = format!("  {}  ", label);
        let line_chars: Vec<char> = line.chars().collect();
        let label_start = 2usize;

        for (x, (i, &ch)) in (list_area.x..).zip(line_chars.iter().enumerate()) {
            if x >= list_area.x + list_area.width {
                break;
            }
            let is_match = i >= label_start
                && i < label_start + label.len()
                && matched_chars.contains(&(i - label_start));
            let style = if is_match {
                Style::default()
                    .fg(theme.accent)
                    .bg(bg)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default()
                    .fg(if is_sel { theme.fg } else { theme.fg_dim })
                    .bg(bg)
            };
            buf.set_string(x, row_y, ch.to_string(), style);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{CommandPaletteState, PaletteCommand};

    #[test]
    fn command_palette_omits_open_db_command() {
        let state = CommandPaletteState::new(vec!["users".to_string()]);

        assert!(state.commands.iter().all(|cmd| cmd.label() != "Open DB…"));
        assert!(state
            .commands
            .contains(&PaletteCommand::SwitchTable("users".to_string())));
    }
}
