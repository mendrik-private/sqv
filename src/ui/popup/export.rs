use crossterm::event::{KeyCode, KeyEvent};
use ratatui::{layout::Rect, style::Style, Frame};

use super::PopupAction;
use crate::{
    export::ExportFormat,
    symbols::Symbols,
    theme::Theme,
    ui::widgets::{
        frame::PopupFrame,
        hints::{hint, render_hints},
        input::{render_labeled, TextInput},
        text::{group_thousands, put},
    },
};

/// Where and what to export: the target file and, when rows are selected,
/// whether to export only those.
pub struct ExportState {
    pub format: ExportFormat,
    pub path: TextInput,
    pub selected_rows: usize,
    pub only_selection: bool,
}

impl ExportState {
    pub fn new(format: ExportFormat, default_path: String, selected_rows: usize) -> Self {
        Self {
            format,
            path: TextInput::new(default_path),
            selected_rows,
            only_selection: selected_rows > 0,
        }
    }

    /// Tab switches between the whole view and the selected rows.
    pub fn handle_key(&mut self, key: &KeyEvent) -> PopupAction {
        match key.code {
            KeyCode::Esc => PopupAction::Close,
            KeyCode::Enter if !self.path.value().trim().is_empty() => PopupAction::Submit,
            KeyCode::Tab | KeyCode::BackTab if self.selected_rows > 0 => {
                self.only_selection = !self.only_selection;
                PopupAction::Handled
            }
            _ if self.path.handle_key(key).consumed() => PopupAction::Handled,
            _ => PopupAction::Ignored,
        }
    }
}

pub fn render(
    frame: &mut Frame,
    area: Rect,
    state: &mut ExportState,
    theme: &Theme,
    symbols: &Symbols,
) {
    let verb = format!("Export {}", state.format.extension().to_uppercase());
    let inner =
        PopupFrame::new(&verb, None, (area.width * 6 / 10).max(50), 7).render(frame, area, theme);
    if inner.height < 4 {
        return;
    }
    let bg = theme.bg_raised;
    let buf = frame.buffer_mut();
    render_labeled(
        buf,
        Rect::new(inner.x, inner.y, inner.width, 1),
        " File: ",
        &state.path,
        theme,
        symbols,
    );
    let scope = if state.only_selection {
        format!(
            "Rows: the {} selected rows",
            group_thousands(state.selected_rows as i64)
        )
    } else {
        "Rows: every row of the current view (filters and sort apply)".to_string()
    };
    put(
        buf,
        inner.x + 1,
        inner.y + 2,
        inner.right(),
        &scope,
        Style::default().fg(theme.fg_dim).bg(bg),
    );
    let mut hints = vec![hint("Enter", "export")];
    if state.selected_rows > 0 {
        hints.push(hint("Tab", "view / selection"));
    }
    hints.push(hint("Esc", "cancel"));
    render_hints(
        buf,
        Rect::new(
            inner.x + 1,
            inner.bottom() - 1,
            inner.width.saturating_sub(1),
            1,
        ),
        &hints,
        theme,
        bg,
    );
}
