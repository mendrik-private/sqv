//! The key reference, generated from [`crate::keymap`].

use crossterm::event::{KeyCode, KeyEvent, MouseEventKind};
use ratatui::{
    layout::Rect,
    style::{Modifier, Style},
    Frame,
};

use super::PopupAction;
use crate::{
    keymap::{Section, SECTIONS},
    symbols::Symbols,
    theme::Theme,
    ui::widgets::{
        frame::PopupFrame,
        hints::{hint, render_hints},
        scrollbar::Scrollbar,
        text::{put, text_width, truncate_with_ellipsis},
    },
};

const KEYS_WIDTH: usize = 24;
const COLUMN_WIDTH: usize = 64;

#[derive(Debug, Default)]
pub struct HelpState {
    pub scroll: usize,
    viewport: usize,
    total: usize,
}

enum HelpLine {
    Title(&'static str),
    Binding(&'static str, &'static str),
    Blank,
}

fn section_lines(section: &Section) -> Vec<HelpLine> {
    let mut lines = vec![HelpLine::Title(section.title)];
    lines.extend(
        section
            .bindings
            .iter()
            .map(|binding| HelpLine::Binding(binding.keys, binding.action)),
    );
    lines.push(HelpLine::Blank);
    lines
}

/// The help as rows of one or two columns, balancing sections by length.
fn layout(columns: usize) -> Vec<Vec<HelpLine>> {
    let sections: Vec<Vec<HelpLine>> = SECTIONS.iter().map(section_lines).collect();
    if columns < 2 {
        return sections
            .into_iter()
            .flatten()
            .map(|line| vec![line])
            .collect();
    }
    let total: usize = sections.iter().map(Vec::len).sum();
    let mut left = Vec::new();
    let mut right = Vec::new();
    for section in sections {
        if left.len() < total.div_ceil(2) {
            left.extend(section);
        } else {
            right.extend(section);
        }
    }
    let rows = left.len().max(right.len());
    let mut left = left.into_iter();
    let mut right = right.into_iter();
    (0..rows)
        .map(|_| {
            vec![
                left.next().unwrap_or(HelpLine::Blank),
                right.next().unwrap_or(HelpLine::Blank),
            ]
        })
        .collect()
}

impl HelpState {
    pub fn new() -> Self {
        Self::default()
    }

    fn max_scroll(&self) -> usize {
        self.total.saturating_sub(self.viewport)
    }

    pub fn scroll_by(&mut self, delta: isize) {
        let target = self.scroll as isize + delta;
        self.scroll = target.clamp(0, self.max_scroll() as isize) as usize;
    }

    pub fn handle_key(&mut self, key: &KeyEvent) -> PopupAction {
        let page = self.viewport.saturating_sub(1).max(1) as isize;
        match key.code {
            KeyCode::Esc => return PopupAction::Close,
            KeyCode::Up | KeyCode::Char('k') => self.scroll_by(-1),
            KeyCode::Down | KeyCode::Char('j') => self.scroll_by(1),
            KeyCode::PageUp => self.scroll_by(-page),
            KeyCode::PageDown | KeyCode::Char(' ') => self.scroll_by(page),
            KeyCode::Home => self.scroll = 0,
            KeyCode::End => self.scroll = self.max_scroll(),
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
    state: &mut HelpState,
    theme: &Theme,
    symbols: &Symbols,
) {
    let columns = if area.width as usize >= COLUMN_WIDTH * 2 + 6 {
        2
    } else {
        1
    };
    let rows = layout(columns);
    let width = (COLUMN_WIDTH * columns + 4) as u16;
    let height = rows.len() as u16 + 3;
    let inner = PopupFrame::new(&format!("{} Help", symbols.help_icon), None, width, height)
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
    state.total = rows.len();
    state.scroll = state.scroll.min(state.max_scroll());
    let column_width = body.width as usize / columns;
    let buf = frame.buffer_mut();
    for (row_in_view, row) in rows
        .iter()
        .skip(state.scroll)
        .take(body.height as usize)
        .enumerate()
    {
        let y = body.y + row_in_view as u16;
        for (column, line) in row.iter().enumerate() {
            let x = body.x + (column * column_width) as u16;
            let right = x + column_width as u16 - 1;
            match line {
                HelpLine::Title(title) => {
                    put(
                        buf,
                        x,
                        y,
                        right,
                        title,
                        Style::default()
                            .fg(theme.accent)
                            .bg(bg)
                            .add_modifier(Modifier::BOLD),
                    );
                }
                HelpLine::Binding(keys, action) => {
                    let keys = truncate_with_ellipsis(keys, KEYS_WIDTH - 1, symbols.ellipsis);
                    put(
                        buf,
                        x + 1,
                        y,
                        right,
                        &keys,
                        Style::default().fg(theme.fg).bg(bg),
                    );
                    let action_x = x + 1 + KEYS_WIDTH.max(text_width(&keys) + 1) as u16;
                    let width = right.saturating_sub(action_x) as usize;
                    let action = truncate_with_ellipsis(action, width, symbols.ellipsis);
                    put(
                        buf,
                        action_x,
                        y,
                        right,
                        &action,
                        Style::default().fg(theme.fg_dim).bg(bg),
                    );
                }
                HelpLine::Blank => {}
            }
        }
    }
    Scrollbar {
        offset: state.scroll,
        total: state.total,
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
        &[hint("↑↓ PgUp PgDn", "scroll"), hint("Esc", "close")],
        theme,
        bg,
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_binding_appears_once_in_either_layout() {
        let bindings: usize = SECTIONS.iter().map(|s| s.bindings.len()).sum();
        for columns in [1, 2] {
            let shown = layout(columns)
                .iter()
                .flatten()
                .filter(|line| matches!(line, HelpLine::Binding(..)))
                .count();
            assert_eq!(shown, bindings);
        }
    }

    #[test]
    fn scrolling_stops_at_the_last_page() {
        let mut state = HelpState {
            scroll: 0,
            viewport: 10,
            total: 25,
        };
        state.handle_key(&KeyEvent::from(KeyCode::End));
        assert_eq!(state.scroll, 15);
        state.handle_key(&KeyEvent::from(KeyCode::PageDown));
        assert_eq!(state.scroll, 15);
    }
}
