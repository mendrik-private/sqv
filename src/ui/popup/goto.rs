use crossterm::event::{KeyCode, KeyEvent};
use ratatui::{layout::Rect, style::Style, Frame};

use super::PopupAction;
use crate::{
    symbols::Symbols,
    theme::Theme,
    ui::widgets::{
        frame::PopupFrame,
        hints::{hint, render_hints},
        input::{render_labeled, TextInput},
        text::{group_thousands, put},
    },
};

/// Asks for a row number to jump to.
pub struct GoToRowState {
    pub input: TextInput,
    pub total_rows: i64,
    pub error: Option<String>,
}

impl GoToRowState {
    pub fn new(total_rows: i64) -> Self {
        Self {
            input: TextInput::default(),
            total_rows,
            error: None,
        }
    }

    /// The zero-based row to go to, or why the input is not a row.
    pub fn target(&self) -> Result<i64, String> {
        let number: i64 = self
            .input
            .value()
            .trim()
            .replace(['_', ' ', '\u{202F}'], "")
            .parse()
            .map_err(|_| "Type a row number".to_string())?;
        if number < 1 || number > self.total_rows {
            return Err(format!(
                "Rows go from 1 to {}",
                group_thousands(self.total_rows)
            ));
        }
        Ok(number - 1)
    }

    pub fn handle_key(&mut self, key: &KeyEvent) -> PopupAction {
        match key.code {
            KeyCode::Esc => PopupAction::Close,
            KeyCode::Enter => match self.target() {
                Ok(_) => PopupAction::Submit,
                Err(error) => {
                    self.error = Some(error);
                    PopupAction::Handled
                }
            },
            _ if self.input.handle_key(key).consumed() => {
                self.error = None;
                PopupAction::Handled
            }
            _ => PopupAction::Ignored,
        }
    }
}

pub fn render(
    frame: &mut Frame,
    area: Rect,
    state: &mut GoToRowState,
    theme: &Theme,
    symbols: &Symbols,
) {
    let inner = PopupFrame::new("Go to row", None, 44, 6).render(frame, area, theme);
    if inner.height < 3 {
        return;
    }
    let buf = frame.buffer_mut();
    let label = format!(" Row (1-{}): ", group_thousands(state.total_rows));
    render_labeled(
        buf,
        Rect::new(inner.x, inner.y, inner.width, 1),
        &label,
        &state.input,
        theme,
        symbols,
    );
    if let Some(error) = &state.error {
        put(
            buf,
            inner.x + 1,
            inner.y + 1,
            inner.right(),
            error,
            Style::default().fg(theme.red).bg(theme.bg_raised),
        );
    }
    render_hints(
        buf,
        Rect::new(
            inner.x + 1,
            inner.bottom() - 1,
            inner.width.saturating_sub(1),
            1,
        ),
        &[hint("Enter", "go"), hint("Esc", "cancel")],
        theme,
        theme.bg_raised,
    );
}
