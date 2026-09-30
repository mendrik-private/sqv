//! Display-safe text: sanitising untrusted strings and fitting them to cells.

use std::borrow::Cow;

use ratatui::{buffer::Buffer, style::Style};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

/// Makes text from the database or the filesystem safe to draw: control
/// characters would otherwise reach the terminal as escape sequences or shift
/// the layout, so newlines, tabs and other controls become visible glyphs.
pub fn sanitize(text: &str) -> Cow<'_, str> {
    if !text.chars().any(char::is_control) {
        return Cow::Borrowed(text);
    }
    Cow::Owned(
        text.chars()
            .map(|ch| match ch {
                '\n' => '↵',
                '\t' => '→',
                '\r' => '␍',
                ch if ch.is_control() => '�',
                ch => ch,
            })
            .collect(),
    )
}

pub fn char_width(ch: char) -> usize {
    UnicodeWidthChar::width(ch).unwrap_or(0)
}

pub fn text_width(text: &str) -> usize {
    UnicodeWidthStr::width(text)
}

/// The longest prefix of `text` that fits in `max_width` terminal cells.
pub fn truncate_to_width(text: &str, max_width: usize) -> String {
    let mut used = 0usize;
    text.chars()
        .take_while(|ch| {
            used += char_width(*ch);
            used <= max_width
        })
        .collect()
}

/// Like [`truncate_to_width`], ending in `ellipsis` when text was cut off.
pub fn truncate_with_ellipsis(text: &str, max_width: usize, ellipsis: char) -> String {
    if text_width(text) <= max_width {
        return text.to_string();
    }
    if max_width <= 1 {
        return truncate_to_width(text, max_width);
    }
    let mut out = truncate_to_width(text, max_width - 1);
    out.push(ellipsis);
    out
}

/// Writes `text` from `x` without passing `right` or leaving the buffer, and
/// returns the column where it ended. Rows outside the buffer are skipped.
pub fn put(buf: &mut Buffer, x: u16, y: u16, right: u16, text: &str, style: Style) -> u16 {
    let area = buf.area;
    if y < area.y || y >= area.bottom() || x >= area.right() {
        return x;
    }
    let right = right.min(area.right());
    buf.set_stringn(x, y, text, right.saturating_sub(x) as usize, style)
        .0
}

/// Groups digits in threes with narrow no-break spaces.
pub fn group_thousands(n: i64) -> String {
    let digits = n.unsigned_abs().to_string();
    let chars: Vec<char> = digits.chars().collect();
    let grouped = chars
        .rchunks(3)
        .rev()
        .map(|chunk| chunk.iter().collect::<String>())
        .collect::<Vec<_>>()
        .join("\u{202F}");
    if n < 0 {
        format!("-{grouped}")
    } else {
        grouped
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitize_replaces_controls_and_keeps_plain_text_borrowed() {
        assert!(matches!(sanitize("plain"), Cow::Borrowed("plain")));
        assert_eq!(sanitize("a\nb\tc\x1b]2;x\x07"), "a↵b→c�]2;x�");
    }

    #[test]
    fn put_ignores_rows_outside_the_buffer() {
        let mut buf = Buffer::empty(ratatui::layout::Rect::new(0, 0, 4, 1));
        assert_eq!(put(&mut buf, 0, 5, 4, "x", Style::default()), 0);
        assert_eq!(put(&mut buf, 1, 0, 3, "abcd", Style::default()), 3);
    }

    #[test]
    fn ellipsis_respects_wide_characters() {
        assert_eq!(truncate_with_ellipsis("日本語テキスト", 5, '…'), "日本…");
        assert_eq!(truncate_with_ellipsis("short", 5, '…'), "short");
    }
}
