use anyhow::{anyhow, Result};
use ratatui::{
    layout::{Constraint, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{block::BorderType, Block, Borders, Paragraph},
    Frame,
};

use crate::{
    db::{
        schema::Column,
        types::{affinity, ColAffinity, SqlValue},
    },
    symbols::Symbols,
    theme::Theme,
};

pub struct InsertFieldState {
    pub name: String,
    pub col_type: String,
    pub not_null: bool,
    pub default_value: Option<String>,
    pub is_pk: bool,
    pub input: String,
    pub touched: bool,
    pub cursor_pos: usize,
}

pub struct InsertRowState {
    pub table: String,
    pub fields: Vec<InsertFieldState>,
    pub selected: usize,
    pub editing: bool,
}

impl InsertRowState {
    pub fn new(table: String, columns: Vec<Column>) -> Self {
        let fields: Vec<InsertFieldState> = columns
            .into_iter()
            .map(|col| InsertFieldState {
                name: col.name,
                col_type: col.col_type,
                not_null: col.not_null,
                default_value: col.default_value,
                is_pk: col.is_pk,
                input: String::new(),
                touched: false,
                cursor_pos: 0,
            })
            .collect();
        let selected = fields
            .iter()
            .position(|field| !field.is_pk && field.not_null && field.default_value.is_none())
            .unwrap_or(0);
        Self {
            table,
            fields,
            selected,
            editing: false,
        }
    }

    pub fn move_up(&mut self) {
        self.selected = self.selected.saturating_sub(1);
    }

    pub fn move_down(&mut self) {
        if self.selected + 1 < self.fields.len() {
            self.selected += 1;
        }
    }

    pub fn start_editing(&mut self) {
        self.editing = true;
        if let Some(field) = self.selected_field_mut() {
            field.touched = true;
            field.cursor_pos = field.input.chars().count();
        }
    }

    pub fn stop_editing(&mut self) {
        self.editing = false;
    }

    pub fn insert_char(&mut self, ch: char) {
        let Some(field) = self.selected_field_mut() else {
            return;
        };
        field.touched = true;
        let byte_pos = field
            .input
            .char_indices()
            .nth(field.cursor_pos)
            .map_or(field.input.len(), |(i, _)| i);
        field.input.insert(byte_pos, ch);
        field.cursor_pos += 1;
    }

    pub fn delete_backward(&mut self) {
        let Some(field) = self.selected_field_mut() else {
            return;
        };
        if field.cursor_pos == 0 {
            return;
        }
        field.touched = true;
        let byte_pos = field
            .input
            .char_indices()
            .nth(field.cursor_pos - 1)
            .map(|(i, _)| i)
            .unwrap_or(0);
        let end_pos = field
            .input
            .char_indices()
            .nth(field.cursor_pos)
            .map_or(field.input.len(), |(i, _)| i);
        field.input.replace_range(byte_pos..end_pos, "");
        field.cursor_pos -= 1;
    }

    pub fn move_cursor_left(&mut self) {
        if let Some(field) = self.selected_field_mut() {
            field.cursor_pos = field.cursor_pos.saturating_sub(1);
        }
    }

    pub fn move_cursor_right(&mut self) {
        if let Some(field) = self.selected_field_mut() {
            let len = field.input.chars().count();
            if field.cursor_pos < len {
                field.cursor_pos += 1;
            }
        }
    }

    pub fn reset_selected(&mut self) {
        self.editing = false;
        if let Some(field) = self.selected_field_mut() {
            field.input.clear();
            field.touched = false;
            field.cursor_pos = 0;
        }
    }

    pub fn build_insert_values(&self) -> Result<Vec<(String, SqlValue)>> {
        let mut values = Vec::new();
        for field in &self.fields {
            match field.parsed_value()? {
                Some(value) => {
                    if value == SqlValue::Null && field.not_null && !field.is_pk {
                        return Err(anyhow!("{} is required", field.name));
                    }
                    values.push((field.name.clone(), value));
                }
                None => {
                    if field.not_null && field.default_value.is_none() && !field.is_pk {
                        return Err(anyhow!("{} is required", field.name));
                    }
                }
            }
        }
        Ok(values)
    }

    pub fn selected_field(&self) -> Option<&InsertFieldState> {
        self.fields.get(self.selected)
    }

    fn selected_field_mut(&mut self) -> Option<&mut InsertFieldState> {
        self.fields.get_mut(self.selected)
    }
}

impl InsertFieldState {
    fn parsed_value(&self) -> Result<Option<SqlValue>> {
        if !self.touched {
            return Ok(None);
        }
        if self.input.is_empty() {
            return Ok(Some(SqlValue::Null));
        }
        match affinity(&self.col_type) {
            ColAffinity::Integer => self
                .input
                .parse::<i64>()
                .map(SqlValue::Integer)
                .map(Some)
                .map_err(|_| anyhow!("{} expects an integer", self.name)),
            ColAffinity::Real | ColAffinity::Numeric => self
                .input
                .parse::<f64>()
                .map(SqlValue::Real)
                .map(Some)
                .map_err(|_| anyhow!("{} expects a number", self.name)),
            ColAffinity::Text | ColAffinity::Blob => Ok(Some(SqlValue::Text(self.input.clone()))),
        }
    }

    fn is_input_valid(&self) -> bool {
        self.parsed_value().is_ok()
    }

    fn display_value(&self) -> String {
        if self.touched {
            if self.input.is_empty() {
                "NULL".to_string()
            } else {
                self.input.clone()
            }
        } else if self.is_pk {
            "<auto>".to_string()
        } else if let Some(default_value) = &self.default_value {
            format!("<default: {}>", default_value)
        } else if self.not_null {
            "<required>".to_string()
        } else {
            "NULL".to_string()
        }
    }

    fn display_editor_value(&self, editing: bool, cursor: char) -> String {
        if !editing {
            return self.display_value();
        }
        let before: String = self.input.chars().take(self.cursor_pos).collect();
        let after: String = self.input.chars().skip(self.cursor_pos).collect();
        format!("{before}{cursor}{after}")
    }
}

pub fn render(
    frame: &mut Frame,
    area: Rect,
    state: &InsertRowState,
    theme: &Theme,
    symbols: &Symbols,
) {
    let popup_width = area.width.saturating_sub(6).max(48).min(area.width);
    let popup_height = area.height.saturating_sub(4).max(12).min(area.height);
    let popup_area = Rect {
        x: area.x + (area.width.saturating_sub(popup_width)) / 2,
        y: area.y + (area.height.saturating_sub(popup_height)) / 2,
        width: popup_width,
        height: popup_height,
    };

    super::paint_popup_surface(frame, popup_area, theme);

    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(theme.accent))
        .title(format!(" Insert row: {} ", state.table))
        .style(Style::default().bg(theme.bg_raised));
    let inner = block.inner(popup_area);
    frame.render_widget(block, popup_area);

    let chunks = Layout::vertical([
        Constraint::Length(2),
        Constraint::Min(4),
        Constraint::Length(3),
        Constraint::Length(2),
    ])
    .split(inner);

    frame.render_widget(
        Paragraph::new(vec![
            Line::from(Span::styled(
                " Values are staged locally until you save the row.",
                Style::default().fg(theme.fg),
            )),
            Line::from(Span::styled(
                " Required fields are checked when you submit.",
                Style::default().fg(theme.fg_faint),
            )),
        ])
        .style(Style::default().bg(theme.bg_raised)),
        chunks[0],
    );

    let list_height = chunks[1].height as usize;
    let start = if state.selected >= list_height {
        state.selected - list_height + 1
    } else {
        0
    };
    let mut lines = Vec::new();
    for (idx, field) in state
        .fields
        .iter()
        .enumerate()
        .skip(start)
        .take(list_height)
    {
        let selected = idx == state.selected;
        let bg = if selected {
            theme.bg_soft
        } else {
            theme.bg_raised
        };
        let marker = if selected {
            format!(" {} ", symbols.selection)
        } else {
            "   ".to_string()
        };
        let label = if field.is_pk {
            format!("{} [pk]", field.name)
        } else if field.not_null {
            format!("{} [req]", field.name)
        } else {
            field.name.clone()
        };
        let value_style = if field.is_input_valid() {
            Style::default().fg(theme.fg_dim).bg(bg)
        } else {
            Style::default().fg(theme.red).bg(bg)
        };
        lines.push(Line::from(vec![
            Span::styled(
                marker,
                Style::default()
                    .fg(if selected {
                        theme.accent
                    } else {
                        theme.bg_raised
                    })
                    .bg(bg),
            ),
            Span::styled(
                format!("{label:<20}"),
                Style::default()
                    .fg(if selected { theme.fg } else { theme.fg_dim })
                    .bg(bg)
                    .add_modifier(if selected {
                        Modifier::BOLD
                    } else {
                        Modifier::empty()
                    }),
            ),
            Span::styled(" ", Style::default().bg(bg)),
            Span::styled(field.display_value(), value_style),
        ]));
    }
    frame.render_widget(
        Paragraph::new(lines).style(Style::default().bg(theme.bg_raised)),
        chunks[1],
    );

    let editor_border = if state.editing {
        theme.accent
    } else {
        theme.line
    };
    let editor_block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(editor_border).bg(theme.bg_soft))
        .title(
            state
                .selected_field()
                .map(|field| format!(" {} ", field.name))
                .unwrap_or_else(|| " value ".to_string()),
        )
        .style(Style::default().bg(theme.bg_soft));
    let editor_inner = editor_block.inner(chunks[2]);
    frame.render_widget(editor_block, chunks[2]);
    if let Some(field) = state.selected_field() {
        frame.render_widget(
            Paragraph::new(field.display_editor_value(state.editing, symbols.cursor))
                .style(Style::default().fg(theme.fg).bg(theme.bg_soft)),
            editor_inner,
        );
    }

    frame.render_widget(
        Paragraph::new(vec![
            Line::from(vec![
                Span::styled("Enter", Style::default().fg(theme.green)),
                Span::styled(
                    if state.editing { " done" } else { " edit" },
                    Style::default().fg(theme.green),
                ),
                Span::styled(
                    symbols.padded_separator(),
                    Style::default().fg(theme.fg_faint),
                ),
                Span::styled("Ctrl-Enter save row", Style::default().fg(theme.accent)),
                Span::styled(
                    symbols.padded_separator(),
                    Style::default().fg(theme.fg_faint),
                ),
                Span::styled("Del reset field", Style::default().fg(theme.yellow)),
                Span::styled(
                    symbols.padded_separator(),
                    Style::default().fg(theme.fg_faint),
                ),
                Span::styled("Esc cancel", Style::default().fg(theme.fg)),
            ]),
            Line::from(Span::styled(
                "Up/Down moves fields when not editing.",
                Style::default().fg(theme.fg_dim),
            )),
        ])
        .style(Style::default().bg(theme.bg_raised)),
        chunks[3],
    );
}

#[cfg(test)]
mod tests {
    use super::InsertRowState;
    use crate::db::schema::Column;

    fn column(name: &str, col_type: &str, not_null: bool, default_value: Option<&str>) -> Column {
        Column {
            cid: 0,
            name: name.to_string(),
            col_type: col_type.to_string(),
            not_null,
            default_value: default_value.map(str::to_string),
            is_pk: false,
        }
    }

    #[test]
    fn build_insert_values_requires_missing_required_fields() {
        let state = InsertRowState::new(
            "users".to_string(),
            vec![column("name", "TEXT", true, None)],
        );

        let err = state
            .build_insert_values()
            .expect_err("missing required field");

        assert!(err.to_string().contains("name is required"));
    }

    #[test]
    fn build_insert_values_omits_untouched_defaults() {
        let mut state = InsertRowState::new(
            "users".to_string(),
            vec![
                column("name", "TEXT", true, None),
                column("age", "INTEGER", false, Some("18")),
            ],
        );
        state.start_editing();
        state.insert_char('A');
        state.insert_char('l');
        state.insert_char('i');
        state.insert_char('c');
        state.insert_char('e');
        state.stop_editing();

        let values = state.build_insert_values().expect("build insert values");

        assert_eq!(values.len(), 1);
        assert_eq!(values[0].0, "name");
    }
}
