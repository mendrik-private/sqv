//! Placeholder content for panels that have nothing to show yet.

use ratatui::{buffer::Buffer, layout::Rect, style::Style};

use super::text::{put, sanitize};
use crate::{symbols::Symbols, theme::Theme};

pub enum StateView<'a> {
    Loading(&'a str),
    /// A message and an optional hint on how to get content.
    Empty(&'a str, Option<&'a str>),
    Error(&'a str),
}

/// Draws the state on the first rows of `area`, indented one cell.
pub fn render_state(
    buf: &mut Buffer,
    area: Rect,
    state: StateView<'_>,
    bg: ratatui::style::Color,
    theme: &Theme,
    symbols: &Symbols,
) {
    if area.height == 0 || area.width < 2 {
        return;
    }
    let right = area.right();
    let x = area.x + 1;
    match state {
        StateView::Loading(label) => {
            put(
                buf,
                x,
                area.y,
                right,
                &format!("{label}{}", symbols.ellipsis),
                Style::default().fg(theme.fg_dim).bg(bg),
            );
        }
        StateView::Empty(message, hint) => {
            put(
                buf,
                x,
                area.y,
                right,
                message,
                Style::default().fg(theme.fg_mute).bg(bg),
            );
            if let Some(hint) = hint.filter(|_| area.height > 1) {
                put(
                    buf,
                    x,
                    area.y + 1,
                    right,
                    hint,
                    Style::default().fg(theme.fg_faint).bg(bg),
                );
            }
        }
        StateView::Error(message) => {
            put(
                buf,
                x,
                area.y,
                right,
                &sanitize(message),
                Style::default().fg(theme.red).bg(bg),
            );
        }
    }
}
