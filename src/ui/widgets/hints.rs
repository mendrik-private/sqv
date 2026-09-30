//! Key hints: one vocabulary (`Enter`, `Esc`, `Ctrl-F`, `↑↓`) on every surface.

use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::{Modifier, Style},
};

use super::text::{put, text_width};
use crate::theme::Theme;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Hint {
    pub keys: &'static str,
    pub label: &'static str,
}

pub const fn hint(keys: &'static str, label: &'static str) -> Hint {
    Hint { keys, label }
}

const GAP: &str = "  ";

impl Hint {
    fn width(&self) -> usize {
        text_width(self.keys) + 1 + text_width(self.label)
    }
}

/// Draws hints left to right on one row, dropping whole hints that do not fit
/// rather than cutting one in half. Returns the column after the last hint.
pub fn render_hints(
    buf: &mut Buffer,
    area: Rect,
    hints: &[Hint],
    theme: &Theme,
    bg: ratatui::style::Color,
) -> u16 {
    let key_style = Style::default()
        .fg(theme.fg_dim)
        .bg(bg)
        .add_modifier(Modifier::BOLD);
    let label_style = Style::default().fg(theme.fg_mute).bg(bg);
    let right = area.right();
    let mut x = area.x;
    for (index, hint) in hints.iter().enumerate() {
        let gap = if index == 0 { 0 } else { GAP.len() };
        if x as usize + gap + hint.width() > right as usize {
            break;
        }
        if gap > 0 {
            x = put(buf, x, area.y, right, GAP, label_style);
        }
        x = put(buf, x, area.y, right, hint.keys, key_style);
        x = put(buf, x, area.y, right, " ", label_style);
        x = put(buf, x, area.y, right, hint.label, label_style);
    }
    x
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hints_that_do_not_fit_are_dropped_whole() {
        let theme = Theme::default();
        let mut buf = Buffer::empty(Rect::new(0, 0, 20, 1));
        let hints = [hint("Enter", "select"), hint("Esc", "cancel")];
        let area = buf.area;
        render_hints(&mut buf, area, &hints, &theme, theme.bg_raised);
        let text: String = buf.content.iter().map(|cell| cell.symbol()).collect();
        assert_eq!(text.trim_end(), "Enter select");
    }
}
