use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEventKind};
use ratatui::{
    layout::{Position, Rect},
    style::Style,
    Frame,
};
use unicode_width::UnicodeWidthChar;

use super::PopupAction;
use crate::{
    db::types::{affinity, expects_number, parse_input, ColAffinity, SqlValue},
    symbols::Symbols,
    theme::Theme,
    ui::widgets::{
        frame::PopupFrame,
        hints::{hint, render_hints},
        input::{delete_backward, insert_char, TextInput},
        scrollbar::Scrollbar,
        text::put,
    },
};

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
        insert_char(&mut self.current, &mut self.cursor_pos, ch);
        self.dirty = true;
        self.follow_cursor = true;
        self.validate();
    }

    pub fn delete_backward(&mut self) {
        if self.readonly || !delete_backward(&mut self.current, &mut self.cursor_pos) {
            return;
        }
        self.dirty = true;
        self.follow_cursor = true;
        self.validate();
    }

    pub fn move_cursor_left(&mut self) {
        self.cursor_pos = self.cursor_pos.saturating_sub(1);
        self.follow_cursor = true;
    }

    pub fn move_cursor_right(&mut self) {
        self.cursor_pos = (self.cursor_pos + 1).min(self.current.chars().count());
        self.follow_cursor = true;
    }

    fn delete_forward(&mut self) {
        if self.readonly || self.cursor_pos >= self.current.chars().count() {
            return;
        }
        self.cursor_pos += 1;
        self.delete_backward();
    }

    /// Moves to the start or end of the current hard line.
    fn move_to_line_edge(&mut self, end: bool) {
        let chars: Vec<char> = self.current.chars().collect();
        let mut pos = self.cursor_pos;
        if end {
            while pos < chars.len() && chars[pos] != '\n' {
                pos += 1;
            }
        } else {
            while pos > 0 && chars[pos - 1] != '\n' {
                pos -= 1;
            }
        }
        self.cursor_pos = pos;
        self.follow_cursor = true;
    }

    fn page(&self) -> u16 {
        usize_to_u16(self.viewport_lines.saturating_sub(1).max(1))
    }

    pub fn handle_key(&mut self, key: &KeyEvent) -> PopupAction {
        let alt = key.modifiers.contains(KeyModifiers::ALT);
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        match key.code {
            KeyCode::Esc => return PopupAction::Close,
            KeyCode::Enter if alt && self.is_multiline => self.insert_char('\n'),
            KeyCode::Enter if !alt && !ctrl => return PopupAction::Submit,
            KeyCode::Tab if self.is_multiline => {
                self.insert_char(' ');
                self.insert_char(' ');
            }
            KeyCode::Char('a') if ctrl => self.move_to_line_edge(false),
            KeyCode::Char('e') if ctrl => self.move_to_line_edge(true),
            KeyCode::Char(ch) if !ctrl && !alt => self.insert_char(ch),
            KeyCode::Backspace => self.delete_backward(),
            KeyCode::Delete => self.delete_forward(),
            KeyCode::Left => self.move_cursor_left(),
            KeyCode::Right => self.move_cursor_right(),
            KeyCode::Home => self.move_to_line_edge(false),
            KeyCode::End => self.move_to_line_edge(true),
            KeyCode::Up if self.is_multiline => self.move_cursor_up(),
            KeyCode::Down if self.is_multiline => self.move_cursor_down(),
            KeyCode::PageUp if self.is_multiline => self.scroll_up(self.page()),
            KeyCode::PageDown if self.is_multiline => self.scroll_down(self.page()),
            _ => return PopupAction::Ignored,
        }
        PopupAction::Handled
    }

    pub fn handle_mouse(&mut self, kind: MouseEventKind, x: u16, y: u16) -> PopupAction {
        let handled = match kind {
            MouseEventKind::ScrollDown if self.mouse_scroll_area_contains(x, y) => {
                self.scroll_down(3);
                true
            }
            MouseEventKind::ScrollUp if self.mouse_scroll_area_contains(x, y) => {
                self.scroll_up(3);
                true
            }
            MouseEventKind::Down(MouseButton::Left) => {
                self.end_scrollbar_drag();
                self.begin_scrollbar_drag(x, y)
            }
            MouseEventKind::Drag(MouseButton::Left) => self.drag_scrollbar(y),
            MouseEventKind::Up(MouseButton::Left) => {
                self.end_scrollbar_drag();
                true
            }
            _ => false,
        };
        if handled {
            PopupAction::Handled
        } else {
            PopupAction::Ignored
        }
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
        self.is_multiline && self.scroll_area.contains(Position { x, y })
    }

    fn scrollbar(&self) -> Scrollbar {
        Scrollbar {
            offset: self.scroll_y as usize,
            total: self.visual_line_count,
            viewport: self.viewport_lines,
        }
    }

    pub(crate) fn begin_scrollbar_drag(&mut self, x: u16, y: u16) -> bool {
        if !self.scrollbar_area.contains(Position { x, y }) {
            return false;
        }
        let track = self.scrollbar_area.height;
        let Some(thumb) = self.scrollbar().thumb(track) else {
            return false;
        };
        let cell = y - self.scrollbar_area.y;
        let grab = if cell >= thumb.start && cell < thumb.start + thumb.len {
            cell - thumb.start
        } else {
            thumb.len / 2
        };
        self.scrollbar_drag_grab = Some(grab as usize);
        self.drag_scrollbar(y)
    }

    pub(crate) fn drag_scrollbar(&mut self, y: u16) -> bool {
        let Some(grab) = self.scrollbar_drag_grab else {
            return false;
        };
        let cell = y.saturating_sub(self.scrollbar_area.y);
        let offset = self
            .scrollbar()
            .offset_at(self.scrollbar_area.height, cell, grab as u16);
        self.follow_cursor = false;
        self.scroll_y = usize_to_u16(offset).min(self.max_scroll_y);
        true
    }

    pub(crate) fn end_scrollbar_drag(&mut self) {
        self.scrollbar_drag_grab = None;
    }

    fn validate(&mut self) {
        self.valid = if matches!(self.original, SqlValue::Blob(_)) {
            self.current.len().is_multiple_of(2)
                && self.current.bytes().all(|byte| byte.is_ascii_hexdigit())
        } else if self.current.trim().is_empty() {
            true
        } else if self.json_mode && !expects_number(&self.col_type) {
            serde_json::from_str::<serde_json::Value>(&self.current).is_ok()
        } else {
            parse_input(&self.col_type, &self.current).is_ok()
        };
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
        parse_input(&self.col_type, &self.current)
            .map_err(|error| anyhow::anyhow!("{} {error}", self.col_name))
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

/// Control characters are shown as visible glyphs so they can never reach the
/// terminal; each still occupies one position so cursor maths stays per char.
fn display_char(ch: char) -> char {
    match ch {
        '\t' => '→',
        ch if ch.is_control() => '�',
        ch => ch,
    }
}

fn push_wrapped_char(lines: &mut Vec<String>, line_width: &mut usize, ch: char, width: usize) {
    let ch = display_char(ch);
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
        let char_width = UnicodeWidthChar::width(display_char(ch)).unwrap_or(0);
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
    let verb = if state.json_mode { "Edit JSON" } else { "Edit" };
    let height = if state.is_multiline { 16 } else { 5 };
    let inner = PopupFrame::new(
        verb,
        Some(&state.col_name),
        (area.width * 6 / 10).max(40),
        height,
    )
    .render(frame, area, theme);
    if inner.height < 3 {
        return;
    }
    let bg = theme.bg_raised;
    let buf = frame.buffer_mut();

    // Status line: validity and read-only state.
    let mut x = inner.x + 1;
    if state.json_mode
        || expects_number(&state.col_type)
        || matches!(state.original, SqlValue::Blob(_))
    {
        let (glyph, color, label) = if state.valid {
            (symbols.valid, theme.green, "valid")
        } else {
            (symbols.invalid, theme.red, "invalid")
        };
        x = put(
            buf,
            x,
            inner.y,
            inner.right(),
            &format!("{glyph} {label}  "),
            Style::default().fg(color).bg(bg),
        );
    }
    if state.readonly {
        put(
            buf,
            x,
            inner.y,
            inner.right(),
            &format!("{} read-only", symbols.readonly),
            Style::default().fg(theme.red).bg(bg),
        );
    } else if x == inner.x + 1 {
        put(
            buf,
            x,
            inner.y,
            inner.right(),
            &state.col_type,
            Style::default().fg(theme.fg_mute).bg(bg),
        );
    }

    let editor = Rect::new(
        inner.x + 1,
        inner.y + 1,
        inner.width.saturating_sub(2),
        inner.height - 2,
    );
    let input_style = Style::default().fg(theme.fg).bg(bg);
    if state.is_multiline {
        state.scroll_area = editor;
        state.scrollbar_area = Rect::new(inner.right() - 1, editor.y, 1, editor.height);
        let text_width = editor.width.saturating_sub(1) as usize;
        let display = state.wrapped_display(symbols.cursor, text_width);
        state.update_viewport(text_width, editor.height as usize, &display);
        for (row, line) in display
            .lines
            .iter()
            .skip(state.scroll_y as usize)
            .take(editor.height as usize)
            .enumerate()
        {
            put(
                buf,
                editor.x,
                editor.y + row as u16,
                editor.x + text_width as u16,
                line,
                input_style,
            );
        }
        state.scrollbar().render(
            buf,
            Rect::new(inner.right() - 1, editor.y, 1, editor.height),
            bg,
            theme,
            symbols,
        );
    } else {
        state.scroll_area = Rect::default();
        state.scrollbar_area = Rect::default();
        state.end_scrollbar_drag();
        let mut input = TextInput::new(state.current.clone());
        for _ in state.cursor_pos..state.current.chars().count() {
            input.move_left();
        }
        input.render(
            buf,
            Rect::new(editor.x, editor.y, editor.width, 1),
            input_style,
            Some((symbols.cursor, Style::default().fg(theme.accent).bg(bg))),
            symbols.ellipsis,
        );
    }

    let mut hints = vec![hint("Enter", "save")];
    if state.is_multiline {
        hints.push(hint("Alt-Enter", "new line"));
    }
    hints.push(hint("Esc", "cancel"));
    render_hints(
        buf,
        Rect::new(
            inner.x + 1,
            inner.bottom() - 1,
            inner.width.saturating_sub(1),
            1,
        ),
        &hints,
        theme,
        bg,
    );
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
