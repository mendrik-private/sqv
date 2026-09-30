use super::text_cursor;
use anyhow::{anyhow, Result};

use crate::db::{
    schema::Column,
    types::{parse_input, SqlValue},
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
    pub insert_position: usize,
    pub editing: bool,
}

impl InsertRowState {
    pub fn new(table: String, columns: Vec<Column>, insert_position: usize) -> Self {
        let fields: Vec<InsertFieldState> = columns
            .into_iter()
            .filter(|column| column.writable)
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
            insert_position,
            editing: false,
        }
    }

    pub fn move_prev_field(&mut self) {
        self.selected = self.selected.saturating_sub(1);
    }

    pub fn move_next_field(&mut self) {
        if self.selected + 1 < self.fields.len() {
            self.selected += 1;
        }
    }

    pub fn start_editing(&mut self) {
        self.editing = true;
        if let Some(field) = self.selected_field_mut() {
            field.cursor_pos = field.input.chars().count();
        }
    }

    pub fn insert_char(&mut self, ch: char) {
        let Some(field) = self.selected_field_mut() else {
            return;
        };
        field.touched = true;
        text_cursor::insert(&mut field.input, &mut field.cursor_pos, ch);
    }

    pub fn delete_backward(&mut self) {
        let Some(field) = self.selected_field_mut() else {
            return;
        };
        if text_cursor::delete_backward(&mut field.input, &mut field.cursor_pos) {
            field.touched = true;
        }
    }

    pub fn move_cursor_left(&mut self) {
        if let Some(field) = self.selected_field_mut() {
            field.cursor_pos = field.cursor_pos.saturating_sub(1);
        }
    }

    pub fn move_cursor_right(&mut self) {
        if let Some(field) = self.selected_field_mut() {
            text_cursor::move_right(&field.input, &mut field.cursor_pos);
        }
    }

    pub fn reset_selected(&mut self) {
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
        parse_input(&self.col_type, &self.input)
            .map(Some)
            .map_err(|error| anyhow!("{} {error}", self.name))
    }

    pub fn is_valid(&self) -> bool {
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

    fn display_editor_value(&self, cursor: char) -> String {
        let before: String = self.input.chars().take(self.cursor_pos).collect();
        let after: String = self.input.chars().skip(self.cursor_pos).collect();
        format!("{before}{cursor}{after}")
    }

    pub fn grid_display_value(&self, selected: bool, cursor: char) -> String {
        if self.touched && selected {
            return self.display_editor_value(cursor);
        }
        if self.touched {
            return self.display_value();
        }
        if self.is_pk {
            "<auto>".to_string()
        } else if let Some(default_value) = &self.default_value {
            default_value.clone()
        } else if self.not_null {
            "<required>".to_string()
        } else {
            "NULL".to_string()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::InsertRowState;
    use crate::db::schema::Column;

    fn column(name: &str, col_type: &str, not_null: bool, default_value: Option<&str>) -> Column {
        Column {
            name: name.to_string(),
            col_type: col_type.to_string(),
            not_null,
            default_value: default_value.map(str::to_string),
            is_pk: false,
            pk_position: 0,
            writable: true,
        }
    }

    #[test]
    fn build_insert_values_requires_missing_required_fields() {
        let state = InsertRowState::new(
            "users".to_string(),
            vec![column("name", "TEXT", true, None)],
            0,
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
            0,
        );
        state.start_editing();
        state.insert_char('A');
        state.insert_char('l');
        state.insert_char('i');
        state.insert_char('c');
        state.insert_char('e');
        state.editing = false;

        let values = state.build_insert_values().expect("build insert values");

        assert_eq!(values.len(), 1);
        assert_eq!(values[0].0, "name");
    }
}
