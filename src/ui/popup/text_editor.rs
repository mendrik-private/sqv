use ratatui::{
    layout::{Constraint, Layout, Rect},
    style::Style,
    text::{Line, Span},
    widgets::{block::BorderType, Block, Borders, Paragraph},
    Frame,
};
use unicode_width::UnicodeWidthChar;

use crate::{
    db::types::{affinity, ColAffinity, SqlValue},
    symbols::Symbols,
    theme::Theme,
};

#[allow(dead_code)]
pub struct TextEditorState {
    pub table: String,
    pub rowid: i64,
    pub col_name: String,
    pub col_type: String,
    pub original: SqlValue,
    pub current: String,
    pub cursor_pos: usize,
    pub is_multiline: bool,
    pub json_mode: bool,
    pub valid: bool,
    pub readonly: bool,
    dirty: bool,
    pub scroll_y: u16,
    wrap_width: usize,
    max_scroll_y: u16,
    follow_cursor: bool,
    scroll_area: Rect,
    scrollbar_area: Rect,
    visual_line_count: usize,
    viewport_lines: usize,
    scrollbar_drag_grab: Option<usize>,
}

struct WrappedDisplay {
    lines: Vec<String>,
    cursor_line: usize,
}

#[derive(Clone, Copy)]
struct ScrollbarMetrics {
    track_height: usize,
    thumb_height: usize,
    thumb_top: usize,
    max_thumb_top: usize,
}

impl TextEditorState {
    pub fn new(
        table: String,
        rowid: i64,
        col_name: String,
        col_type: String,
        original: SqlValue,
        readonly: bool,
    ) -> Self {
        let mut current = match &original {
            SqlValue::Null => String::new(),
            SqlValue::Integer(n) => n.to_string(),
            SqlValue::Real(f) => f.to_string(),
            SqlValue::Text(s) => s.clone(),
            SqlValue::Blob(bytes) => bytes.iter().map(|byte| format!("{byte:02X}")).collect(),
        };
        let mut json_mode = false;
        if let SqlValue::Text(text) = &original {
            if let Ok(json) = serde_json::from_str::<serde_json::Value>(text) {
                if let Ok(pretty) = serde_json::to_string_pretty(&json) {
                    current = pretty;
                    json_mode = true;
                }
            }
        }
        let is_multiline = json_mode || matches!(affinity(&col_type), ColAffinity::Text);
        let cursor_pos = current.chars().count();
        let mut state = Self {
            table,
            rowid,
            col_name,
            col_type,
            original,
            current,
            cursor_pos,
            is_multiline,
            json_mode,
            valid: true,
            readonly,
            dirty: false,
            scroll_y: 0,
            wrap_width: 0,
            max_scroll_y: 0,
            follow_cursor: true,
            scroll_area: Rect::default(),
            scrollbar_area: Rect::default(),
            visual_line_count: 0,
            viewport_lines: 0,
            scrollbar_drag_grab: None,
        };
        state.validate();
        state
    }

    pub fn insert_char(&mut self, ch: char) {
        if self.readonly {
            return;
        }
        let byte_pos = self
            .current
            .char_indices()
            .nth(self.cursor_pos)
            .map_or(self.current.len(), |(i, _)| i);
        self.current.insert(byte_pos, ch);
        self.dirty = true;
        self.cursor_pos += 1;
        self.follow_cursor = true;
        self.validate();
    }

    pub fn delete_backward(&mut self) {
        if self.readonly || self.cursor_pos == 0 {
            return;
        }
        let byte_pos = self
            .current
            .char_indices()
            .nth(self.cursor_pos - 1)
            .map(|(i, _)| i)
            .unwrap_or(0);
        let end_pos = self
            .current
            .char_indices()
            .nth(self.cursor_pos)
            .map_or(self.current.len(), |(i, _)| i);
        self.current.replace_range(byte_pos..end_pos, "");
        self.dirty = true;
        self.cursor_pos -= 1;
        self.follow_cursor = true;
        self.validate();
    }

    pub fn move_cursor_left(&mut self) {
        self.cursor_pos = self.cursor_pos.saturating_sub(1);
        self.follow_cursor = true;
    }

    pub fn move_cursor_right(&mut self) {
        let len = self.current.chars().count();
        if self.cursor_pos < len {
            self.cursor_pos += 1;
        }
        self.follow_cursor = true;
    }

    pub fn move_cursor_up(&mut self) {
        self.move_cursor_vertically(-1);
    }

    pub fn move_cursor_down(&mut self) {
        self.move_cursor_vertically(1);
    }

    pub fn scroll_up(&mut self, lines: u16) {
        self.follow_cursor = false;
        self.scroll_y = self.scroll_y.saturating_sub(lines);
    }

    pub fn scroll_down(&mut self, lines: u16) {
        self.follow_cursor = false;
        self.scroll_y = self.scroll_y.saturating_add(lines).min(self.max_scroll_y);
    }

    pub(crate) fn mouse_scroll_area_contains(&self, x: u16, y: u16) -> bool {
        self.is_multiline && rect_contains(self.scroll_area, x, y)
    }

    pub(crate) fn begin_scrollbar_drag(&mut self, x: u16, y: u16) -> bool {
        if !rect_contains(self.scrollbar_area, x, y) {
            return false;
        }
        let Some(metrics) = scrollbar_metrics(
            self.scrollbar_area,
            self.scroll_y as usize,
            self.visual_line_count,
            self.viewport_lines,
        ) else {
            return false;
        };
        if metrics.max_thumb_top == 0 {
            return false;
        }

        let pointer_offset = usize::from(y.saturating_sub(self.scrollbar_area.y))
            .min(metrics.track_height.saturating_sub(1));
        self.scrollbar_drag_grab = Some(
            if pointer_offset >= metrics.thumb_top
                && pointer_offset < metrics.thumb_top.saturating_add(metrics.thumb_height)
            {
                pointer_offset.saturating_sub(metrics.thumb_top)
            } else {
                metrics.thumb_height / 2
            },
        );
        self.drag_scrollbar(y)
    }

    pub(crate) fn drag_scrollbar(&mut self, y: u16) -> bool {
        let Some(grab_offset) = self.scrollbar_drag_grab else {
            return false;
        };
        let Some(metrics) = scrollbar_metrics(
            self.scrollbar_area,
            self.scroll_y as usize,
            self.visual_line_count,
            self.viewport_lines,
        ) else {
            self.scrollbar_drag_grab = None;
            return false;
        };
        if metrics.max_thumb_top == 0 {
            return false;
        }

        let pointer_offset = usize::from(y.saturating_sub(self.scrollbar_area.y));
        let thumb_top = pointer_offset
            .saturating_sub(grab_offset)
            .min(metrics.max_thumb_top);
        let numerator = (thumb_top as u64)
            .saturating_mul(u64::from(self.max_scroll_y))
            .saturating_add((metrics.max_thumb_top / 2) as u64);
        self.follow_cursor = false;
        self.scroll_y = u16::try_from(numerator / metrics.max_thumb_top as u64)
            .unwrap_or(u16::MAX)
            .min(self.max_scroll_y);
        true
    }

    pub(crate) fn end_scrollbar_drag(&mut self) {
        self.scrollbar_drag_grab = None;
    }

    fn validate(&mut self) {
        let upper = self.col_type.to_uppercase();
        if matches!(self.original, SqlValue::Blob(_)) {
            self.valid = self.current.len().is_multiple_of(2)
                && self.current.bytes().all(|byte| byte.is_ascii_hexdigit());
        } else if upper.contains("INT") {
            self.valid = self.current.is_empty() || self.current.parse::<i64>().is_ok();
        } else if upper.contains("REAL") || upper.contains("FLOAT") || upper.contains("DOUBLE") {
            self.valid = self.current.is_empty() || self.current.parse::<f64>().is_ok();
        } else if self.json_mode {
            self.valid = self.current.trim().is_empty()
                || serde_json::from_str::<serde_json::Value>(&self.current).is_ok();
        } else {
            self.valid = true;
        }
    }

    pub fn as_sql_value(&self) -> anyhow::Result<SqlValue> {
        if !self.dirty {
            return Ok(self.original.clone());
        }
        if !self.valid {
            anyhow::bail!("value is not valid for column type {}", self.col_type);
        }
        if matches!(self.original, SqlValue::Blob(_)) {
            let bytes = self
                .current
                .as_bytes()
                .chunks(2)
                .map(|pair| {
                    let pair = std::str::from_utf8(pair)?;
                    Ok(u8::from_str_radix(pair, 16)?)
                })
                .collect::<anyhow::Result<Vec<_>>>()?;
            return Ok(SqlValue::Blob(bytes));
        }
        if self.current.is_empty() {
            return Ok(if matches!(affinity(&self.col_type), ColAffinity::Text) {
                SqlValue::Text(String::new())
            } else {
                SqlValue::Null
            });
        }
        let upper = self.col_type.to_uppercase();
        if upper.contains("INT") {
            Ok(self.current.parse::<i64>().map(SqlValue::Integer)?)
        } else if upper.contains("REAL") || upper.contains("FLOAT") || upper.contains("DOUBLE") {
            Ok(self.current.parse::<f64>().map(SqlValue::Real)?)
        } else {
            Ok(SqlValue::Text(self.current.clone()))
        }
    }

    fn move_cursor_vertically(&mut self, direction: isize) {
        let width = if self.wrap_width == 0 {
            usize::MAX
        } else {
            self.wrap_width
        };
        let positions = cursor_positions(&self.current, width);
        let Some(&(line, col)) = positions.get(self.cursor_pos) else {
            return;
        };
        let target_line = if direction < 0 {
            line.checked_sub(1)
        } else {
            line.checked_add(1)
        };
        let Some(target_line) = target_line else {
            self.follow_cursor = true;
            return;
        };

        if let Some((index, _)) = positions
            .iter()
            .enumerate()
            .filter(|(_, (candidate_line, _))| *candidate_line == target_line)
            .min_by_key(|(_, (_, candidate_col))| candidate_col.abs_diff(col))
        {
            self.cursor_pos = index;
        }
        self.follow_cursor = true;
    }

    fn wrapped_display(&self, cursor: char, width: usize) -> WrappedDisplay {
        let width = width.max(1);
        let mut lines = vec![String::new()];
        let mut line_width = 0usize;
        let mut cursor_line = 0usize;

        for (index, ch) in self.current.chars().enumerate() {
            if index == self.cursor_pos {
                push_display_char(&mut lines, &mut line_width, cursor, width, &mut cursor_line);
            }
            if ch == '\n' {
                lines.push(String::new());
                line_width = 0;
            } else {
                push_wrapped_char(&mut lines, &mut line_width, ch, width);
            }
        }
        if self.cursor_pos == self.current.chars().count() {
            push_display_char(&mut lines, &mut line_width, cursor, width, &mut cursor_line);
        }

        WrappedDisplay { lines, cursor_line }
    }

    fn update_viewport(&mut self, width: usize, viewport_lines: usize, display: &WrappedDisplay) {
        self.wrap_width = width.max(1);
        let viewport_lines = viewport_lines.max(1);
        self.visual_line_count = display.lines.len();
        self.viewport_lines = viewport_lines;
        self.max_scroll_y = usize_to_u16(display.lines.len().saturating_sub(viewport_lines));
        self.scroll_y = self.scroll_y.min(self.max_scroll_y);

        if self.follow_cursor {
            let cursor_line = usize_to_u16(display.cursor_line);
            if cursor_line < self.scroll_y {
                self.scroll_y = cursor_line;
            } else if display.cursor_line >= self.scroll_y as usize + viewport_lines {
                self.scroll_y = usize_to_u16(
                    display
                        .cursor_line
                        .saturating_add(1)
                        .saturating_sub(viewport_lines),
                )
                .min(self.max_scroll_y);
            }
        }
    }
}

fn rect_contains(area: Rect, x: u16, y: u16) -> bool {
    area.width > 0
        && area.height > 0
        && x >= area.x
        && x < area.x.saturating_add(area.width)
        && y >= area.y
        && y < area.y.saturating_add(area.height)
}

fn push_display_char(
    lines: &mut Vec<String>,
    line_width: &mut usize,
    cursor: char,
    width: usize,
    cursor_line: &mut usize,
) {
    push_wrapped_char(lines, line_width, cursor, width);
    *cursor_line = lines.len().saturating_sub(1);
}

fn push_wrapped_char(lines: &mut Vec<String>, line_width: &mut usize, ch: char, width: usize) {
    let char_width = UnicodeWidthChar::width(ch).unwrap_or(0);
    if *line_width > 0 && line_width.saturating_add(char_width) > width {
        lines.push(String::new());
        *line_width = 0;
    }
    if let Some(line) = lines.last_mut() {
        line.push(ch);
    }
    *line_width = line_width.saturating_add(char_width);
}

fn cursor_positions(text: &str, width: usize) -> Vec<(usize, usize)> {
    let width = width.max(1);
    let mut positions = Vec::with_capacity(text.chars().count().saturating_add(1));
    let mut line = 0usize;
    let mut col = 0usize;

    for ch in text.chars() {
        let char_width = UnicodeWidthChar::width(ch).unwrap_or(0);
        if ch != '\n' && col > 0 && col.saturating_add(char_width) > width {
            line = line.saturating_add(1);
            col = 0;
        }
        positions.push(if col >= width {
            (line.saturating_add(1), 0)
        } else {
            (line, col)
        });
        if ch == '\n' {
            line = line.saturating_add(1);
            col = 0;
        } else {
            col = col.saturating_add(char_width);
        }
    }
    positions.push(if col >= width {
        (line.saturating_add(1), 0)
    } else {
        (line, col)
    });
    positions
}

fn usize_to_u16(value: usize) -> u16 {
    u16::try_from(value).unwrap_or(u16::MAX)
}

pub fn render(
    frame: &mut Frame,
    area: Rect,
    state: &mut TextEditorState,
    theme: &Theme,
    symbols: &Symbols,
) {
    let popup_width = (area.width * 6 / 10).max(40).min(area.width);
    let popup_height = if state.is_multiline { 14u16 } else { 5u16 };
    let x = area.x + (area.width.saturating_sub(popup_width)) / 2;
    let y = area.y + area.height / 3;
    let popup_area = Rect {
        x,
        y,
        width: popup_width,
        height: popup_height.min(area.height),
    };

    super::paint_popup_surface(frame, popup_area, theme);

    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(theme.accent))
        .title(format!(
            " {}: {} ",
            if state.json_mode { "JSON" } else { "Edit" },
            state.col_name
        ))
        .style(Style::default().bg(theme.bg_raised));

    let inner = block.inner(popup_area);
    frame.render_widget(block, popup_area);

    let upper = state.col_type.to_uppercase();
    let show_validity = state.json_mode
        || upper.contains("INT")
        || upper.contains("REAL")
        || upper.contains("FLOAT")
        || upper.contains("DOUBLE");

    let sections = if state.is_multiline {
        Layout::vertical([
            Constraint::Length(if show_validity { 2 } else { 1 }),
            Constraint::Min(1),
            Constraint::Length(1),
        ])
        .split(inner)
    } else {
        Layout::vertical([
            Constraint::Length(if show_validity { 2 } else { 1 }),
            Constraint::Length(1),
            Constraint::Length(1),
        ])
        .split(inner)
    };

    let input_style = Style::default().fg(theme.fg).bg(theme.bg_raised);
    let hint_style = Style::default().fg(theme.fg_faint);

    let mut status_lines = Vec::new();
    if show_validity {
        let vi = if state.valid {
            symbols.valid
        } else {
            symbols.invalid
        };
        let vc = if state.valid { theme.green } else { theme.red };
        status_lines.push(Line::from(Span::styled(
            format!(" {} ", vi),
            Style::default().fg(vc).bg(theme.bg_raised),
        )));
    }

    if state.readonly {
        status_lines.push(Line::from(Span::styled(
            format!(" {} read-only", symbols.readonly),
            Style::default().fg(theme.red),
        )));
    }
    if status_lines.is_empty() {
        status_lines.push(Line::from(Span::styled(
            " editing value",
            Style::default().fg(theme.fg_faint).bg(theme.bg_raised),
        )));
    }
    frame.render_widget(
        Paragraph::new(status_lines).style(Style::default().bg(theme.bg_raised)),
        sections[0],
    );

    if state.is_multiline {
        let editor_chunks = Layout::horizontal([
            Constraint::Length(1),
            Constraint::Min(1),
            Constraint::Length(1),
        ])
        .split(sections[1]);
        state.scroll_area = sections[1];
        state.scrollbar_area = editor_chunks[2];
        let viewport_lines = editor_chunks[1].height.max(1) as usize;
        let display = state.wrapped_display(symbols.cursor, editor_chunks[1].width as usize);
        state.update_viewport(editor_chunks[1].width as usize, viewport_lines, &display);
        let total_lines = display.lines.len();
        let content_lines: Vec<Line<'static>> = display
            .lines
            .into_iter()
            .map(|line| Line::from(Span::styled(line, input_style)))
            .collect();
        frame.render_widget(
            Paragraph::new(content_lines)
                .style(Style::default().bg(theme.bg_raised))
                .scroll((state.scroll_y, 0)),
            editor_chunks[1],
        );
        render_scrollbar(
            frame,
            editor_chunks[2],
            state.scroll_y as usize,
            total_lines,
            viewport_lines,
            theme,
            symbols,
        );
    } else {
        state.scroll_area = Rect::default();
        state.scrollbar_area = Rect::default();
        state.end_scrollbar_drag();
        let editor_chunks =
            Layout::horizontal([Constraint::Length(1), Constraint::Min(1)]).split(sections[1]);
        let display = state.wrapped_display(symbols.cursor, usize::MAX);
        let line = display
            .lines
            .into_iter()
            .next()
            .unwrap_or_else(|| symbols.cursor.to_string());
        frame.render_widget(
            Paragraph::new(vec![Line::from(Span::styled(line, input_style))])
                .style(Style::default().bg(theme.bg_raised)),
            editor_chunks[1],
        );
    }

    frame.render_widget(
        Paragraph::new(vec![Line::from(Span::styled(
            if state.is_multiline {
                format!(
                    " Enter save {} Alt-Enter newline {} Esc cancel",
                    symbols.separator, symbols.separator
                )
            } else {
                format!(" Enter save {} Esc cancel", symbols.separator)
            },
            hint_style,
        ))])
        .style(Style::default().bg(theme.bg_raised)),
        sections[2],
    );
}

fn render_scrollbar(
    frame: &mut Frame,
    area: Rect,
    offset: usize,
    total: usize,
    viewport: usize,
    theme: &Theme,
    symbols: &Symbols,
) {
    if area.width == 0 || area.height == 0 {
        return;
    }
    let buf = frame.buffer_mut();
    let track_height = area.height as usize;
    for row in 0..track_height {
        buf.set_string(
            area.x,
            area.y + row as u16,
            symbols.box_vertical.to_string(),
            Style::default().fg(theme.line).bg(theme.bg_raised),
        );
    }
    let Some(metrics) = scrollbar_metrics(area, offset, total, viewport) else {
        return;
    };
    for row in metrics.thumb_top..metrics.thumb_top + metrics.thumb_height {
        buf.set_string(
            area.x,
            area.y + row as u16,
            symbols.scrollbar_thumb.to_string(),
            Style::default().fg(theme.fg_mute).bg(theme.bg_raised),
        );
    }
}

fn scrollbar_metrics(
    area: Rect,
    offset: usize,
    total: usize,
    viewport: usize,
) -> Option<ScrollbarMetrics> {
    let track_height = area.height as usize;
    if area.width == 0 || track_height == 0 || viewport == 0 || total <= viewport {
        return None;
    }

    let thumb_height = ((viewport * track_height) / total).max(1).min(track_height);
    let max_offset = total.saturating_sub(viewport);
    let max_thumb_top = track_height.saturating_sub(thumb_height);
    let thumb_top = offset
        .min(max_offset)
        .checked_mul(max_thumb_top)
        .and_then(|value| value.checked_div(max_offset))
        .unwrap_or(0)
        .min(max_thumb_top);

    Some(ScrollbarMetrics {
        track_height,
        thumb_height,
        thumb_top,
        max_thumb_top,
    })
}

#[cfg(test)]
mod tests {
    use super::TextEditorState;
    use crate::db::types::SqlValue;
    use ratatui::layout::Rect;

    fn scrollable_editor() -> TextEditorState {
        let mut state = TextEditorState::new(
            "users".to_string(),
            1,
            "note".to_string(),
            "TEXT".to_string(),
            SqlValue::Text("x".repeat(100)),
            false,
        );
        let display = state.wrapped_display('|', 1);
        state.scroll_area = Rect::new(2, 3, 10, 10);
        state.scrollbar_area = Rect::new(11, 3, 1, 10);
        state.update_viewport(1, 10, &display);
        state.scroll_up(u16::MAX);
        state
    }

    #[test]
    fn varchar_columns_are_multiline_text_editors() {
        let state = TextEditorState::new(
            "users".to_string(),
            1,
            "note".to_string(),
            "VARCHAR(255)".to_string(),
            SqlValue::Text("hello".to_string()),
            false,
        );

        assert!(state.is_multiline);
    }

    #[test]
    fn long_text_soft_wraps_at_the_editor_width() {
        let state = TextEditorState::new(
            "users".to_string(),
            1,
            "note".to_string(),
            "TEXT".to_string(),
            SqlValue::Text("abcdefghij".to_string()),
            false,
        );

        let display = state.wrapped_display('|', 4);

        assert_eq!(display.lines, ["abcd", "efgh", "ij|"]);
        assert_eq!(display.cursor_line, 2);
    }

    #[test]
    fn soft_wrap_uses_terminal_width_for_wide_characters() {
        let state = TextEditorState::new(
            "users".to_string(),
            1,
            "note".to_string(),
            "TEXT".to_string(),
            SqlValue::Text("界界a".to_string()),
            false,
        );

        let display = state.wrapped_display('|', 4);

        assert_eq!(display.lines, ["界界", "a|"]);
    }

    #[test]
    fn wrapped_cursor_is_kept_inside_the_viewport() {
        let mut state = TextEditorState::new(
            "users".to_string(),
            1,
            "note".to_string(),
            "TEXT".to_string(),
            SqlValue::Text("abcdefghijklmnopqrst".to_string()),
            false,
        );
        let display = state.wrapped_display('|', 5);

        state.update_viewport(5, 2, &display);

        assert_eq!(display.lines.len(), 5);
        assert_eq!(state.scroll_y, 3);
    }

    #[test]
    fn page_scroll_is_not_undone_by_cursor_following() {
        let mut state = TextEditorState::new(
            "users".to_string(),
            1,
            "note".to_string(),
            "TEXT".to_string(),
            SqlValue::Text("abcdefghijklmnopqrst".to_string()),
            false,
        );
        let display = state.wrapped_display('|', 5);
        state.update_viewport(5, 2, &display);

        state.scroll_up(2);
        state.update_viewport(5, 2, &display);

        assert_eq!(state.scroll_y, 1);
    }

    #[test]
    fn vertical_cursor_movement_follows_soft_wrapped_rows() {
        let mut state = TextEditorState::new(
            "users".to_string(),
            1,
            "note".to_string(),
            "TEXT".to_string(),
            SqlValue::Text("abcdefghijkl".to_string()),
            false,
        );
        let display = state.wrapped_display('|', 5);
        state.update_viewport(5, 2, &display);

        state.move_cursor_up();
        assert_eq!(state.cursor_pos, 7);

        state.move_cursor_up();
        assert_eq!(state.cursor_pos, 2);
    }

    #[test]
    fn vertical_cursor_movement_uses_hard_lines_before_first_render() {
        let mut state = TextEditorState::new(
            "users".to_string(),
            1,
            "note".to_string(),
            "TEXT".to_string(),
            SqlValue::Text("ab\ncd".to_string()),
            false,
        );

        state.move_cursor_up();

        assert_eq!(state.cursor_pos, 2);
    }

    #[test]
    fn mouse_scroll_area_uses_the_rendered_editor_bounds() {
        let state = scrollable_editor();

        assert!(state.mouse_scroll_area_contains(2, 3));
        assert!(state.mouse_scroll_area_contains(11, 12));
        assert!(!state.mouse_scroll_area_contains(1, 3));
        assert!(!state.mouse_scroll_area_contains(2, 13));
    }

    #[test]
    fn scrollbar_drag_maps_the_full_track_to_the_scroll_range() {
        let mut state = scrollable_editor();
        let max_scroll = state.max_scroll_y;

        assert!(state.begin_scrollbar_drag(11, 3));
        assert_eq!(state.scroll_y, 0);
        assert!(state.drag_scrollbar(12));
        assert_eq!(state.scroll_y, max_scroll);

        state.end_scrollbar_drag();
        assert!(!state.drag_scrollbar(3));
        assert_eq!(state.scroll_y, max_scroll);
    }

    #[test]
    fn clicking_the_scrollbar_track_jumps_toward_the_pointer() {
        let mut state = scrollable_editor();

        assert!(state.begin_scrollbar_drag(11, 8));

        assert!(state.scroll_y > 0);
        assert!(state.scroll_y < state.max_scroll_y);
    }

    #[test]
    fn invalid_numeric_text_cannot_turn_into_null() {
        let mut state = TextEditorState::new(
            "items".to_string(),
            1,
            "amount".to_string(),
            "INTEGER".to_string(),
            SqlValue::Integer(7),
            false,
        );
        state.insert_char('x');
        assert!(!state.valid);
        assert!(state.as_sql_value().is_err());
    }

    #[test]
    fn clearing_text_produces_an_empty_string_without_changing_an_untouched_null() {
        let mut text = TextEditorState::new(
            "items".to_string(),
            1,
            "label".to_string(),
            "TEXT".to_string(),
            SqlValue::Text("x".to_string()),
            false,
        );
        text.delete_backward();
        assert_eq!(
            text.as_sql_value().expect("cleared text"),
            SqlValue::Text(String::new())
        );

        let null = TextEditorState::new(
            "items".to_string(),
            1,
            "label".to_string(),
            "TEXT".to_string(),
            SqlValue::Null,
            false,
        );
        assert_eq!(null.as_sql_value().expect("unchanged null"), SqlValue::Null);
    }

    #[test]
    fn blob_editor_round_trips_and_parses_hex() {
        let original = SqlValue::Blob(vec![0x00, 0xff]);
        let mut state = TextEditorState::new(
            "items".to_string(),
            1,
            "payload".to_string(),
            "BLOB".to_string(),
            original.clone(),
            false,
        );
        assert_eq!(state.current, "00FF");
        assert_eq!(state.as_sql_value().expect("unchanged blob"), original);
        state.delete_backward();
        state.delete_backward();
        state.insert_char('A');
        state.insert_char('A');
        assert_eq!(
            state.as_sql_value().expect("edited blob"),
            SqlValue::Blob(vec![0x00, 0xaa])
        );
    }
}
