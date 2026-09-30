use anyhow::{anyhow, Result};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use super::PopupAction;
use crate::{
    db::{
        schema::Column,
        types::{parse_input, SqlValue},
    },
    ui::widgets::input::TextInput,
};

pub struct InsertFieldState {
    pub name: String,
    pub col_type: String,
    pub not_null: bool,
    pub default_value: Option<String>,
    pub is_pk: bool,
    /// Generated columns are shown but never written.
    pub writable: bool,
    pub input: TextInput,
    pub touched: bool,
}

/// A new row staged inline in the grid. There is one field per column, in
/// column order, so field indexes are grid column indexes.
pub struct InsertRowState {
    pub table: String,
    pub fields: Vec<InsertFieldState>,
    pub selected: usize,
    pub insert_position: usize,
}

impl InsertRowState {
    pub fn new(table: String, columns: Vec<Column>, insert_position: usize) -> Self {
        let fields: Vec<InsertFieldState> = columns
            .into_iter()
            .map(|col| InsertFieldState {
                name: col.name,
                col_type: col.col_type,
                not_null: col.not_null,
                default_value: col.default_value,
                is_pk: col.is_pk,
                writable: col.writable,
                input: TextInput::default(),
                touched: false,
            })
            .collect();
        let selected = fields
            .iter()
            .position(|field| {
                field.writable && !field.is_pk && field.not_null && field.default_value.is_none()
            })
            .or_else(|| fields.iter().position(|field| field.writable))
            .unwrap_or(0);
        Self {
            table,
            fields,
            selected,
            insert_position,
        }
    }

    fn step_field(&mut self, forward: bool) {
        let mut index = self.selected;
        loop {
            let next = if forward {
                index + 1
            } else {
                match index.checked_sub(1) {
                    Some(prev) => prev,
                    None => return,
                }
            };
            let Some(field) = self.fields.get(next) else {
                return;
            };
            index = next;
            if field.writable {
                self.selected = index;
                return;
            }
        }
    }

    pub fn move_prev_field(&mut self) {
        self.step_field(false);
    }

    pub fn move_next_field(&mut self) {
        self.step_field(true);
    }

    pub fn reset_selected(&mut self) {
        if let Some(field) = self.fields.get_mut(self.selected) {
            field.input.clear();
            field.touched = false;
        }
    }

    /// Enter, Tab and ↓ move to the next field, Shift-Tab and ↑ to the previous;
    /// Alt-Enter commits the row (`Submit`), Esc discards it.
    pub fn handle_key(&mut self, key: &KeyEvent) -> PopupAction {
        match key.code {
            KeyCode::Esc => return PopupAction::Close,
            KeyCode::Enter if key.modifiers.contains(KeyModifiers::ALT) => {
                return PopupAction::Submit
            }
            KeyCode::Enter | KeyCode::Tab | KeyCode::Down => self.move_next_field(),
            KeyCode::BackTab | KeyCode::Up => self.move_prev_field(),
            KeyCode::Delete if key.modifiers.contains(KeyModifiers::SHIFT) => self.reset_selected(),
            _ => {
                let Some(field) = self.fields.get_mut(self.selected) else {
                    return PopupAction::Ignored;
                };
                match field.input.handle_key(key) {
                    crate::ui::widgets::input::InputOutcome::Changed => field.touched = true,
                    crate::ui::widgets::input::InputOutcome::Moved => {}
                    crate::ui::widgets::input::InputOutcome::Ignored => {
                        return PopupAction::Ignored
                    }
                }
            }
        }
        PopupAction::Handled
    }

    pub fn build_insert_values(&self) -> Result<Vec<(String, SqlValue)>> {
        let mut values = Vec::new();
        for field in self.fields.iter().filter(|field| field.writable) {
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
}

impl InsertFieldState {
    fn parsed_value(&self) -> Result<Option<SqlValue>> {
        if !self.touched {
            return Ok(None);
        }
        if self.input.is_empty() {
            return Ok(Some(SqlValue::Null));
        }
        parse_input(&self.col_type, self.input.value())
            .map(Some)
            .map_err(|error| anyhow!("{} {error}", self.name))
    }

    pub fn is_valid(&self) -> bool {
        self.parsed_value().is_ok()
    }

    /// The field's text when it is not being edited: the typed value, or what
    /// the database will fill in.
    pub fn placeholder(&self) -> String {
        if !self.writable {
            return "<generated>".to_string();
        }
        if self.touched {
            return if self.input.is_empty() {
                "NULL".to_string()
            } else {
                self.input.value().to_string()
            };
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
    fn generated_columns_keep_field_positions_and_are_skipped() {
        let mut generated = column("total", "INTEGER", false, None);
        generated.writable = false;
        let mut state = InsertRowState::new(
            "t".to_string(),
            vec![
                column("a", "TEXT", false, None),
                generated,
                column("b", "TEXT", false, None),
            ],
            0,
        );
        assert_eq!(state.fields.len(), 3);
        state.move_next_field();
        assert_eq!(state.selected, 2, "the generated column is skipped");
        assert_eq!(state.fields[1].placeholder(), "<generated>");
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
        for ch in "Alice".chars() {
            state.handle_key(&crossterm::event::KeyEvent::from(
                crossterm::event::KeyCode::Char(ch),
            ));
        }

        let values = state.build_insert_values().expect("build insert values");

        assert_eq!(values.len(), 1);
        assert_eq!(values[0].0, "name");
    }
}
