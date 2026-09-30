use crossterm::event::{KeyCode, KeyEvent, MouseEventKind};
use ratatui::{
    layout::Rect,
    style::{Modifier, Style},
    Frame,
};

use super::PopupAction;
use crate::{
    symbols::Symbols,
    theme::Theme,
    ui::widgets::{
        frame::PopupFrame,
        hints::{hint, render_hints},
        scrollbar::Scrollbar,
        text::{put, sanitize},
    },
};

/// A line of the schema view: a section heading or plain text.
#[derive(Debug, Clone, PartialEq)]
pub enum SchemaLine {
    Heading(String),
    Text(String),
}

/// The definition of a table, view or index: its DDL, indexes and keys.
pub struct SchemaViewState {
    pub name: String,
    pub ddl: String,
    pub lines: Vec<SchemaLine>,
    scroll: usize,
    viewport: usize,
}

impl SchemaViewState {
    pub fn new(name: String, ddl: String, lines: Vec<SchemaLine>) -> Self {
        Self {
            name,
            ddl,
            lines,
            scroll: 0,
            viewport: 0,
        }
    }

    fn scroll_by(&mut self, delta: isize) {
        let max = self.lines.len().saturating_sub(self.viewport) as isize;
        self.scroll = (self.scroll as isize + delta).clamp(0, max.max(0)) as usize;
    }

    pub fn handle_key(&mut self, key: &KeyEvent) -> PopupAction {
        let page = self.viewport.saturating_sub(1).max(1) as isize;
        match key.code {
            KeyCode::Esc => return PopupAction::Close,
            KeyCode::Char('y') => return PopupAction::Copy,
            KeyCode::Up | KeyCode::Char('k') => self.scroll_by(-1),
            KeyCode::Down | KeyCode::Char('j') => self.scroll_by(1),
            KeyCode::PageUp => self.scroll_by(-page),
            KeyCode::PageDown => self.scroll_by(page),
            KeyCode::Home => self.scroll = 0,
            KeyCode::End => self.scroll_by(isize::MAX / 2),
            _ => return PopupAction::Ignored,
        }
        PopupAction::Handled
    }

    pub fn handle_mouse(&mut self, kind: MouseEventKind) -> PopupAction {
        match kind {
            MouseEventKind::ScrollDown => self.scroll_by(3),
            MouseEventKind::ScrollUp => self.scroll_by(-3),
            _ => return PopupAction::Ignored,
        }
        PopupAction::Handled
    }
}

pub fn render(
    frame: &mut Frame,
    area: Rect,
    state: &mut SchemaViewState,
    theme: &Theme,
    symbols: &Symbols,
) {
    let height = (state.lines.len() as u16 + 3).max(8);
    let inner = PopupFrame::new(
        "Schema",
        Some(&state.name),
        (area.width * 7 / 10).max(50),
        height,
    )
    .render(frame, area, theme);
    if inner.height < 2 {
        return;
    }
    let bg = theme.bg_raised;
    let body = Rect::new(
        inner.x + 1,
        inner.y,
        inner.width.saturating_sub(2),
        inner.height - 1,
    );
    state.viewport = body.height as usize;
    state.scroll_by(0);
    let buf = frame.buffer_mut();
    for (row, line) in state
        .lines
        .iter()
        .skip(state.scroll)
        .take(body.height as usize)
        .enumerate()
    {
        let y = body.y + row as u16;
        match line {
            SchemaLine::Heading(text) => {
                put(
                    buf,
                    body.x,
                    y,
                    body.right(),
                    text,
                    Style::default()
                        .fg(theme.accent)
                        .bg(bg)
                        .add_modifier(Modifier::BOLD),
                );
            }
            SchemaLine::Text(text) => {
                put(
                    buf,
                    body.x,
                    y,
                    body.right(),
                    &sanitize(text),
                    Style::default().fg(theme.fg).bg(bg),
                );
            }
        }
    }
    Scrollbar {
        offset: state.scroll,
        total: state.lines.len(),
        viewport: state.viewport,
    }
    .render(
        buf,
        Rect::new(inner.right() - 1, body.y, 1, body.height),
        bg,
        theme,
        symbols,
    );
    render_hints(
        buf,
        Rect::new(
            inner.x + 1,
            inner.bottom() - 1,
            inner.width.saturating_sub(1),
            1,
        ),
        &[
            hint("↑↓", "scroll"),
            hint("y", "copy definition"),
            hint("Esc", "close"),
        ],
        theme,
        bg,
    );
}
