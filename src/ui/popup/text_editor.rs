use ratatui::{
    layout::{Constraint, Layout, Rect},
    style::Style,
    text::{Line, Span},
    widgets::{block::BorderType, Block, Borders, Paragraph},
    Frame,
};

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
    pub scroll_y: u16,
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
            SqlValue::Blob(_) => String::new(),
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
            scroll_y: 0,
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
        self.cursor_pos += 1;
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
        self.cursor_pos -= 1;
        self.validate();
    }

    pub fn move_cursor_left(&mut self) {
        self.cursor_pos = self.cursor_pos.saturating_sub(1);
    }

    pub fn move_cursor_right(&mut self) {
        let len = self.current.chars().count();
        if self.cursor_pos < len {
            self.cursor_pos += 1;
        }
    }

    pub fn move_cursor_up(&mut self) {
        let (line, col) = self.cursor_line_col();
        if line > 0 {
            self.cursor_pos = self.cursor_from_line_col(line - 1, col);
        }
    }

    pub fn move_cursor_down(&mut self) {
        let (line, col) = self.cursor_line_col();
        if line + 1 < self.line_count() {
            self.cursor_pos = self.cursor_from_line_col(line + 1, col);
        }
    }

    pub fn scroll_up(&mut self, lines: u16) {
        self.scroll_y = self.scroll_y.saturating_sub(lines);
    }

    pub fn scroll_down(&mut self, lines: u16) {
        let max_scroll = self.line_count().saturating_sub(1) as u16;
        self.scroll_y = self.scroll_y.saturating_add(lines).min(max_scroll);
    }

    fn validate(&mut self) {
        let upper = self.col_type.to_uppercase();
        if upper.contains("INT") {
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

    pub fn as_sql_value(&self) -> SqlValue {
        if self.current.is_empty() {
            return SqlValue::Null;
        }
        let upper = self.col_type.to_uppercase();
        if upper.contains("INT") {
            self.current
                .parse::<i64>()
                .ok()
                .map(SqlValue::Integer)
                .unwrap_or(SqlValue::Null)
        } else if upper.contains("REAL") || upper.contains("FLOAT") || upper.contains("DOUBLE") {
            self.current
                .parse::<f64>()
                .ok()
                .map(SqlValue::Real)
                .unwrap_or(SqlValue::Null)
        } else {
            SqlValue::Text(self.current.clone())
        }
    }

    fn line_count(&self) -> usize {
        self.current.split('\n').count().max(1)
    }

    fn cursor_line(&self) -> usize {
        self.current
            .chars()
            .take(self.cursor_pos)
            .filter(|ch| *ch == '\n')
            .count()
    }

    fn cursor_line_col(&self) -> (usize, usize) {
        let before: String = self.current.chars().take(self.cursor_pos).collect();
        let mut line = 0usize;
        let mut col = 0usize;
        for ch in before.chars() {
            if ch == '\n' {
                line += 1;
                col = 0;
            } else {
                col += 1;
            }
        }
        (line, col)
    }

    fn cursor_from_line_col(&self, target_line: usize, target_col: usize) -> usize {
        let lines: Vec<&str> = self.current.split('\n').collect();
        let capped_line = target_line.min(lines.len().saturating_sub(1));
        let mut pos = 0usize;
        for line in lines.iter().take(capped_line) {
            pos += line.chars().count() + 1;
        }
        let line_len = lines
            .get(capped_line)
            .map_or(0, |line| line.chars().count());
        pos + target_col.min(line_len)
    }

    fn effective_scroll_y(&self, viewport_lines: usize) -> u16 {
        let cursor_line = self.cursor_line();
        let max_scroll = self.line_count().saturating_sub(viewport_lines) as u16;
        let mut scroll_y = self.scroll_y.min(max_scroll);
        if cursor_line < scroll_y as usize {
            scroll_y = cursor_line as u16;
        } else if cursor_line >= scroll_y as usize + viewport_lines {
            scroll_y = (cursor_line + 1 - viewport_lines) as u16;
        }
        scroll_y
    }

    fn display_lines(&self, cursor: char) -> Vec<String> {
        let before: String = self.current.chars().take(self.cursor_pos).collect();
        let after: String = self.current.chars().skip(self.cursor_pos).collect();
        let display = format!("{before}{cursor}{after}");
        display
            .split('\n')
            .map(|line| {
                if line.is_empty() {
                    " ".to_string()
                } else {
                    line.to_string()
                }
            })
            .collect()
    }
}

pub fn render(
    frame: &mut Frame,
    area: Rect,
    state: &TextEditorState,
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
        let content_lines: Vec<Line<'static>> = state
            .display_lines(symbols.cursor)
            .into_iter()
            .map(|line| Line::from(Span::styled(line, input_style)))
            .collect();
        let viewport_lines = editor_chunks[1].height.max(1) as usize;
        let scroll_y = state.effective_scroll_y(viewport_lines);
        frame.render_widget(
            Paragraph::new(content_lines)
                .style(Style::default().bg(theme.bg_raised))
                .scroll((scroll_y, 0)),
            editor_chunks[1],
        );
        render_scrollbar(
            frame,
            editor_chunks[2],
            scroll_y as usize,
            state.line_count(),
            viewport_lines,
            theme,
            symbols,
        );
    } else {
        let editor_chunks =
            Layout::horizontal([Constraint::Length(1), Constraint::Min(1)]).split(sections[1]);
        let display_lines = state.display_lines(symbols.cursor);
        let line = display_lines
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
    if total <= viewport || viewport == 0 {
        return;
    }
    let thumb_height = ((viewport * track_height) / total).max(1).min(track_height);
    let max_offset = total.saturating_sub(viewport);
    let thumb_top = if max_offset == 0 {
        0
    } else {
        offset
            .min(max_offset)
            .checked_mul(track_height.saturating_sub(thumb_height))
            .and_then(|n| n.checked_div(max_offset))
            .unwrap_or(0)
            .min(track_height.saturating_sub(thumb_height))
    };
    for row in thumb_top..thumb_top + thumb_height {
        buf.set_string(
            area.x,
            area.y + row as u16,
            symbols.scrollbar_thumb.to_string(),
            Style::default().fg(theme.fg_mute).bg(theme.bg_raised),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::TextEditorState;
    use crate::db::types::SqlValue;

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
}
