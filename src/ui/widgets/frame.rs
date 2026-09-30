//! The raised, bordered surface every popup is drawn on.

use ratatui::{
    layout::Rect,
    style::Style,
    widgets::{block::BorderType, Block, Borders, Clear},
    Frame,
};

use super::text::sanitize;
use crate::theme::Theme;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Anchor {
    Center,
    /// A third of the way down, leaving the context above visible.
    Upper,
}

/// A popup of the requested size, shrunk to fit the screen so it can never be
/// drawn outside the buffer.
pub struct PopupFrame {
    title: String,
    width: u16,
    height: u16,
    anchor: Anchor,
}

impl PopupFrame {
    /// `verb` names the popup, `subject` what it acts on: ` Filter · amount `.
    pub fn new(verb: &str, subject: Option<&str>, width: u16, height: u16) -> Self {
        let title = match subject {
            Some(subject) => format!(" {verb} · {} ", sanitize(subject)),
            None => format!(" {verb} "),
        };
        Self {
            title,
            width,
            height,
            anchor: Anchor::Center,
        }
    }

    pub fn anchor(mut self, anchor: Anchor) -> Self {
        self.anchor = anchor;
        self
    }

    /// The rectangle this frame occupies inside `area`.
    pub fn outer(&self, area: Rect) -> Rect {
        let width = self.width.min(area.width);
        let height = self.height.min(area.height);
        let free = area.height - height;
        let y_offset = match self.anchor {
            Anchor::Center => free / 2,
            Anchor::Upper => (area.height / 4).min(free),
        };
        Rect {
            x: area.x + (area.width - width) / 2,
            y: area.y + y_offset,
            width,
            height,
        }
    }

    /// Paints the surface, shadow and border and returns the content rect.
    pub fn render(self, frame: &mut Frame, area: Rect, theme: &Theme) -> Rect {
        let outer = self.outer(area);
        let screen = frame.area();
        let shadow = Rect {
            x: outer.x.saturating_add(1),
            y: outer.y.saturating_add(1),
            width: outer.width,
            height: outer.height,
        }
        .intersection(screen);
        if !shadow.is_empty() {
            frame.render_widget(
                Block::default().style(Style::default().bg(theme.line_soft)),
                shadow,
            );
        }
        frame.render_widget(Clear, outer);
        let block = Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(theme.accent))
            .title(self.title)
            .style(Style::default().bg(theme.bg_raised).fg(theme.fg));
        let inner = block.inner(outer);
        frame.render_widget(block, outer);
        inner
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frames_always_fit_the_area() {
        let area = Rect::new(0, 0, 30, 5);
        for anchor in [Anchor::Center, Anchor::Upper] {
            let rect = PopupFrame::new("Palette", None, 80, 20)
                .anchor(anchor)
                .outer(area);
            assert!(rect.bottom() <= area.bottom() && rect.right() <= area.right());
        }
    }

    #[test]
    fn titles_are_sanitised() {
        let frame = PopupFrame::new("Edit", Some("na\x1bme"), 10, 5);
        assert_eq!(frame.title, " Edit · na�me ");
    }
}
