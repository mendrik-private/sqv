//! A proportional vertical scrollbar whose geometry is shared by drawing and
//! mouse dragging, so the two can never disagree.

use ratatui::{buffer::Buffer, layout::Rect, style::Style};

use super::text::put;
use crate::{symbols::Symbols, theme::Theme};

/// Position of `viewport` rows starting at `offset` within `total` rows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Scrollbar {
    pub offset: usize,
    pub total: usize,
    pub viewport: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Thumb {
    pub start: u16,
    pub len: u16,
}

impl Scrollbar {
    pub fn needed(&self) -> bool {
        self.total > self.viewport
    }

    /// Thumb position inside a track of `track` cells; `None` when nothing scrolls.
    pub fn thumb(&self, track: u16) -> Option<Thumb> {
        if !self.needed() || track == 0 {
            return None;
        }
        let track_len = track as usize;
        let len = (self.viewport * track_len / self.total).clamp(1, track_len);
        let max_offset = self.total - self.viewport;
        let travel = track_len - len;
        let start = (self.offset.min(max_offset) * travel + max_offset / 2) / max_offset;
        Some(Thumb {
            start: start as u16,
            len: len as u16,
        })
    }

    /// The offset that puts the thumb's grab point under track cell `cell`,
    /// where `grab` is how far into the thumb the drag started.
    pub fn offset_at(&self, track: u16, cell: u16, grab: u16) -> usize {
        let Some(thumb) = self.thumb(track) else {
            return 0;
        };
        let travel = (track - thumb.len) as usize;
        if travel == 0 {
            return 0;
        }
        let start = cell.saturating_sub(grab).min(track - thumb.len) as usize;
        let max_offset = self.total - self.viewport;
        (start * max_offset + travel / 2) / travel
    }

    /// Draws the track and thumb down the first column of `area`.
    pub fn render(
        &self,
        buf: &mut Buffer,
        area: Rect,
        bg: ratatui::style::Color,
        theme: &Theme,
        symbols: &Symbols,
    ) {
        let (x, y, track) = (area.x, area.y, area.height);
        let Some(thumb) = self.thumb(track) else {
            return;
        };
        for row in 0..track {
            let in_thumb = row >= thumb.start && row < thumb.start + thumb.len;
            let (glyph, fg) = if in_thumb {
                (symbols.scrollbar_thumb, theme.fg_mute)
            } else {
                (symbols.box_vertical, theme.line)
            };
            put(
                buf,
                x,
                y + row,
                x + 1,
                &glyph.to_string(),
                Style::default().fg(fg).bg(bg),
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn thumb_spans_the_track_proportionally() {
        let bar = Scrollbar {
            offset: 0,
            total: 100,
            viewport: 10,
        };
        assert_eq!(bar.thumb(20), Some(Thumb { start: 0, len: 2 }));
        let end = Scrollbar { offset: 90, ..bar };
        assert_eq!(end.thumb(20), Some(Thumb { start: 18, len: 2 }));
        assert_eq!(Scrollbar { total: 5, ..bar }.thumb(20), None);
    }

    #[test]
    fn dragging_maps_back_to_the_offset_it_draws() {
        let bar = Scrollbar {
            offset: 0,
            total: 1000,
            viewport: 50,
        };
        assert_eq!(bar.offset_at(20, 0, 0), 0);
        assert_eq!(bar.offset_at(20, 19, 0), 950);
        let mid = bar.offset_at(20, 9, 0);
        assert_eq!(
            Scrollbar { offset: mid, ..bar }.thumb(20).map(|t| t.start),
            Some(9)
        );
    }
}
