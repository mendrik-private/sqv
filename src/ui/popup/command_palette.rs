use std::borrow::Cow;

use crossterm::event::{KeyCode, KeyEvent, MouseButton, MouseEventKind};
use ratatui::{
    layout::{Position, Rect},
    style::{Modifier, Style},
    Frame,
};

use super::PopupAction;
use crate::{
    export::ExportFormat,
    symbols::Symbols,
    theme::Theme,
    ui::widgets::{
        frame::{Anchor, PopupFrame},
        fuzzy_filter, highlighted_spans,
        hints::{hint, render_hints},
        input::TextInput,
        list::{paint_row, ListCursor},
        state::{render_state, StateView},
        text::put,
        FuzzyMatch,
    },
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CopyFormat {
    Json,
    Csv,
    Sql,
}

#[derive(Debug, Clone, PartialEq)]
pub enum PaletteCommand {
    Export(ExportFormat),
    CopyCell,
    CopyRows(CopyFormat),
    CopyColumn,
    Find,
    GoToRow,
    SqlConsole,
    SearchAllTables,
    ShowRecord,
    ShowSchema,
    ReferencingRows,
    FilterColumn,
    ClearFilters,
    SortColumn,
    ShowHiddenColumns,
    ToggleFreezeColumn,
    InsertRow,
    DeleteRows,
    Undo,
    NextTab,
    PrevTab,
    CloseTab,
    ReloadSchema,
    ToggleSidebar,
    ToggleReadonly,
    Help,
    Quit,
    SwitchTable(String),
}

impl PaletteCommand {
    pub fn label(&self) -> Cow<'static, str> {
        Cow::Borrowed(match self {
            PaletteCommand::Export(ExportFormat::Csv) => "Export as CSV…",
            PaletteCommand::Export(ExportFormat::Json) => "Export as JSON…",
            PaletteCommand::Export(ExportFormat::Sql) => "Export as SQL…",
            PaletteCommand::CopyCell => "Copy cell",
            PaletteCommand::CopyRows(CopyFormat::Json) => "Copy rows as JSON",
            PaletteCommand::CopyRows(CopyFormat::Csv) => "Copy rows as CSV",
            PaletteCommand::CopyRows(CopyFormat::Sql) => "Copy rows as SQL inserts",
            PaletteCommand::CopyColumn => "Copy column values",
            PaletteCommand::Find => "Find in table",
            PaletteCommand::GoToRow => "Go to row",
            PaletteCommand::SqlConsole => "SQL console",
            PaletteCommand::SearchAllTables => "Search all tables",
            PaletteCommand::ShowRecord => "Show row as record",
            PaletteCommand::ShowSchema => "Show schema",
            PaletteCommand::ReferencingRows => "Show referencing rows",
            PaletteCommand::FilterColumn => "Filter column",
            PaletteCommand::ClearFilters => "Clear filters",
            PaletteCommand::SortColumn => "Sort by column",
            PaletteCommand::ShowHiddenColumns => "Show hidden columns",
            PaletteCommand::ToggleFreezeColumn => "Freeze / unfreeze first column",
            PaletteCommand::InsertRow => "Insert row",
            PaletteCommand::DeleteRows => "Delete rows",
            PaletteCommand::Undo => "Undo last write",
            PaletteCommand::NextTab => "Next tab",
            PaletteCommand::PrevTab => "Previous tab",
            PaletteCommand::CloseTab => "Close tab",
            PaletteCommand::ReloadSchema => "Reload schema",
            PaletteCommand::ToggleSidebar => "Toggle sidebar",
            PaletteCommand::ToggleReadonly => "Toggle read-only",
            PaletteCommand::Help => "Help",
            PaletteCommand::Quit => "Quit",
            PaletteCommand::SwitchTable(name) => return format!("Open table: {name}").into(),
        })
    }

    /// The direct key for the command, as listed in the key map.
    pub fn shortcut(&self) -> Option<&'static str> {
        Some(match self {
            PaletteCommand::CopyCell => "y",
            PaletteCommand::CopyRows(CopyFormat::Json) => "Y",
            PaletteCommand::Find => "Ctrl-F",
            PaletteCommand::GoToRow => "Ctrl-G",
            PaletteCommand::SqlConsole => ":",
            PaletteCommand::ShowRecord => "v",
            PaletteCommand::ReferencingRows => "r",
            PaletteCommand::FilterColumn => "f",
            PaletteCommand::ClearFilters => "F",
            PaletteCommand::SortColumn => "s",
            PaletteCommand::InsertRow => "i",
            PaletteCommand::DeleteRows => "d",
            PaletteCommand::Undo => "Ctrl-Z",
            PaletteCommand::NextTab => "]",
            PaletteCommand::PrevTab => "[",
            PaletteCommand::CloseTab => "Ctrl-W",
            PaletteCommand::ToggleSidebar => "Ctrl-B",
            PaletteCommand::Help => "?",
            PaletteCommand::Quit => "Ctrl-Q",
            _ => return None,
        })
    }
}

pub struct CommandPaletteState {
    pub query: TextInput,
    pub commands: Vec<PaletteCommand>,
    pub list: ListCursor,
    list_area: Rect,
}

impl CommandPaletteState {
    pub fn new(table_names: Vec<String>) -> Self {
        let mut commands = vec![
            PaletteCommand::Find,
            PaletteCommand::GoToRow,
            PaletteCommand::SqlConsole,
            PaletteCommand::SearchAllTables,
            PaletteCommand::ShowRecord,
            PaletteCommand::ShowSchema,
            PaletteCommand::ReferencingRows,
            PaletteCommand::FilterColumn,
            PaletteCommand::ClearFilters,
            PaletteCommand::SortColumn,
            PaletteCommand::ShowHiddenColumns,
            PaletteCommand::ToggleFreezeColumn,
            PaletteCommand::CopyCell,
            PaletteCommand::CopyRows(CopyFormat::Json),
            PaletteCommand::CopyRows(CopyFormat::Csv),
            PaletteCommand::CopyRows(CopyFormat::Sql),
            PaletteCommand::CopyColumn,
            PaletteCommand::Export(ExportFormat::Csv),
            PaletteCommand::Export(ExportFormat::Json),
            PaletteCommand::Export(ExportFormat::Sql),
            PaletteCommand::InsertRow,
            PaletteCommand::DeleteRows,
            PaletteCommand::Undo,
            PaletteCommand::NextTab,
            PaletteCommand::PrevTab,
            PaletteCommand::CloseTab,
            PaletteCommand::ReloadSchema,
            PaletteCommand::ToggleSidebar,
            PaletteCommand::ToggleReadonly,
            PaletteCommand::Help,
            PaletteCommand::Quit,
        ];
        commands.extend(table_names.into_iter().map(PaletteCommand::SwitchTable));
        Self {
            query: TextInput::default(),
            commands,
            list: ListCursor::default(),
            list_area: Rect::default(),
        }
    }

    fn filtered(&self) -> Vec<FuzzyMatch<&PaletteCommand>> {
        filter_commands(&self.commands, &self.query)
    }

    pub fn selected_command(&self) -> Option<PaletteCommand> {
        self.filtered()
            .get(self.list.selected)
            .map(|(command, _, _)| (*command).clone())
    }

    pub fn handle_key(&mut self, key: &KeyEvent) -> PopupAction {
        match key.code {
            KeyCode::Esc => return PopupAction::Close,
            KeyCode::Enter if self.selected_command().is_some() => return PopupAction::Submit,
            KeyCode::Enter => return PopupAction::Handled,
            KeyCode::Home | KeyCode::End if !self.query.is_empty() => {}
            _ => {
                if self.list.handle_key(key, self.filtered().len()) {
                    return PopupAction::Handled;
                }
            }
        }
        if self.query.handle_key(key).consumed() {
            self.list.reset();
            PopupAction::Handled
        } else {
            PopupAction::Ignored
        }
    }

    pub fn handle_mouse(&mut self, kind: MouseEventKind, x: u16, y: u16) -> PopupAction {
        let len = self.filtered().len();
        match kind {
            MouseEventKind::ScrollDown | MouseEventKind::ScrollUp => {
                self.list.scroll(kind == MouseEventKind::ScrollDown, len);
                PopupAction::Handled
            }
            MouseEventKind::Down(MouseButton::Left)
                if self.list_area.contains(Position { x, y }) =>
            {
                match self.list.hit((y - self.list_area.y) as usize, len) {
                    Some(index) if index == self.list.selected => PopupAction::Submit,
                    Some(index) => {
                        self.list.select(index, len);
                        PopupAction::Handled
                    }
                    None => PopupAction::Handled,
                }
            }
            _ => PopupAction::Ignored,
        }
    }
}

fn filter_commands<'a>(
    commands: &'a [PaletteCommand],
    query: &TextInput,
) -> Vec<FuzzyMatch<&'a PaletteCommand>> {
    fuzzy_filter(
        commands
            .iter()
            .map(|command| (command, command.label().into_owned())),
        query.value(),
    )
}

pub fn render(
    frame: &mut Frame,
    area: Rect,
    state: &mut CommandPaletteState,
    theme: &Theme,
    symbols: &Symbols,
) {
    let width = (area.width * 6 / 10).max(50);
    let height = (area.height / 2).max(12);
    let inner = PopupFrame::new("Command palette", None, width, height)
        .anchor(Anchor::Upper)
        .render(frame, area, theme);
    if inner.height < 3 {
        return;
    }
    let buf = frame.buffer_mut();
    let bg = theme.bg_raised;
    put(
        buf,
        inner.x,
        inner.y,
        inner.right(),
        " > ",
        Style::default().fg(theme.accent).bg(bg),
    );
    state.query.render(
        buf,
        Rect::new(inner.x + 3, inner.y, inner.width.saturating_sub(3), 1),
        Style::default().fg(theme.fg).bg(bg),
        Some((symbols.cursor, Style::default().fg(theme.accent).bg(bg))),
        symbols.ellipsis,
    );

    let list_area = Rect::new(inner.x, inner.y + 1, inner.width, inner.height - 2);
    state.list_area = list_area;
    let filtered = filter_commands(&state.commands, &state.query);
    if filtered.is_empty() {
        render_state(
            buf,
            list_area,
            StateView::Empty("No matching command", None),
            bg,
            theme,
            symbols,
        );
    }
    for (row, index) in state
        .list
        .visible(filtered.len(), list_area.height as usize)
        .enumerate()
    {
        let (command, label, matched) = &filtered[index];
        let selected = index == state.list.selected;
        let row_rect = Rect::new(list_area.x, list_area.y + row as u16, list_area.width, 1);
        paint_row(buf, row_rect, selected, theme, symbols);
        let row_bg = if selected { theme.bg_soft } else { bg };
        let shortcut = command.shortcut().unwrap_or("");
        let label_width = (list_area.width as usize).saturating_sub(3 + shortcut.len() + 2);
        let spans = highlighted_spans(
            label,
            |i| matched.binary_search(&i).is_ok(),
            label_width,
            Style::default()
                .fg(if selected { theme.fg } else { theme.fg_dim })
                .bg(row_bg),
            Style::default()
                .fg(theme.accent)
                .bg(row_bg)
                .add_modifier(Modifier::BOLD),
        );
        let mut x = row_rect.x + 2;
        for span in spans {
            x = put(
                buf,
                x,
                row_rect.y,
                row_rect.right(),
                &span.content,
                span.style,
            );
        }
        if !shortcut.is_empty() {
            let sx = row_rect.right().saturating_sub(shortcut.len() as u16 + 1);
            put(
                buf,
                sx,
                row_rect.y,
                row_rect.right(),
                shortcut,
                Style::default().fg(theme.fg_mute).bg(row_bg),
            );
        }
    }
    render_hints(
        buf,
        Rect::new(
            inner.x + 1,
            inner.bottom() - 1,
            inner.width.saturating_sub(1),
            1,
        ),
        &[
            hint("Enter", "run"),
            hint("↑↓", "select"),
            hint("Esc", "close"),
        ],
        theme,
        bg,
    );
}

#[cfg(test)]
mod tests {
    use super::{CommandPaletteState, PaletteCommand};

    #[test]
    fn palette_lists_tables_and_actions_with_shortcuts() {
        let state = CommandPaletteState::new(vec!["users".to_string()]);
        assert!(state
            .commands
            .contains(&PaletteCommand::SwitchTable("users".to_string())));
        assert_eq!(PaletteCommand::Find.shortcut(), Some("Ctrl-F"));
    }
}
