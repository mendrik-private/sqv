use ratatui::{
    layout::Rect,
    style::{Modifier, Style},
    Frame,
};

use crate::{
    db::types::{affinity, ColAffinity},
    grid::GridState,
    theme::Theme,
};

pub fn should_show_rail(state: &GridState) -> bool {
    if state.window.total_rows < 200 {
        return false;
    }
    if let Some(sort) = &state.sort {
        if let Some(col) = state.columns.get(sort.col_idx) {
            return matches!(affinity(&col.col_type), ColAffinity::Text);
        }
    }
    false
}

pub const RAIL_WIDTH: u16 = 2;

fn all_letters() -> Vec<char> {
    std::iter::once('#').chain('A'..='Z').collect()
}

fn letters_to_show(track_height: usize) -> Vec<char> {
    let letters = all_letters();
    if track_height >= letters.len() {
        letters
    } else {
        let step = ((letters.len() as f32 / track_height as f32).ceil() as usize).max(1);
        letters.iter().step_by(step).cloned().collect()
    }
}

fn normalize_letter(ch: char) -> char {
    let upper = ch.to_uppercase().next().unwrap_or(ch);
    if upper.is_ascii_alphabetic() {
        upper
    } else {
        '#'
    }
}

fn active_letter(state: &GridState) -> Option<char> {
    state
        .window
        .get_row(state.focused_row as i64)
        .or_else(|| state.window.get_row(state.viewport_start))
        .and_then(|row| state.sort.as_ref().and_then(|s| row.get(s.col_idx)))
        .and_then(|val| match val {
            crate::db::types::SqlValue::Text(s) => s.chars().next(),
            _ => None,
        })
        .map(normalize_letter)
}

pub fn hit_test(area: Rect, state: &GridState, x: u16, y: u16) -> Option<char> {
    if !should_show_rail(state) {
        return None;
    }

    let track_height = area.height.saturating_sub(3) as usize;
    if track_height == 0 {
        return None;
    }

    let rail_x = area.x + area.width.saturating_sub(RAIL_WIDTH + 1);
    let rail_y_start = area.y + 3;
    let letters = letters_to_show(track_height);
    let rail_y_end = rail_y_start + letters.len() as u16;
    if x < rail_x || x >= rail_x + RAIL_WIDTH || y < rail_y_start || y >= rail_y_end {
        return None;
    }

    letters.get((y - rail_y_start) as usize).copied()
}

pub fn render_rail(frame: &mut Frame, area: Rect, state: &GridState, theme: &Theme) {
    if !should_show_rail(state) {
        return;
    }

    let track_height = area.height.saturating_sub(3) as usize;
    if track_height == 0 {
        return;
    }

    // Rail is 2 chars wide, to the left of the scrollbar (1 col)
    let rail_x = area.x + area.width.saturating_sub(RAIL_WIDTH + 1);
    let rail_y_start = area.y + 3; // after header and divider

    let current_letter = active_letter(state);
    let letters_to_show = letters_to_show(track_height);

    let buf = frame.buffer_mut();
    for (i, &letter) in letters_to_show.iter().enumerate() {
        let y = rail_y_start + i as u16;
        if y >= area.y + area.height {
            break;
        }

        let is_current = current_letter == Some(letter);
        let style = if is_current {
            Style::default()
                .fg(theme.accent)
                .bg(theme.bg)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(theme.fg_dim).bg(theme.bg)
        };

        let label = format!("{} ", letter);
        buf.set_string(rail_x, y, &label, style);
    }
}

#[cfg(test)]
mod tests {
    use super::{active_letter, hit_test};
    use crate::{
        db::{schema::Column, types::SqlValue},
        grid::{GridInit, GridState, SortDir, SortSpec},
    };
    use ratatui::layout::Rect;

    fn text_col(name: &str) -> Column {
        Column {
            cid: 0,
            name: name.to_string(),
            col_type: "TEXT".to_string(),
            not_null: false,
            default_value: None,
            is_pk: false,
            pk_position: 0,
            writable: true,
        }
    }

    #[test]
    fn hit_test_returns_clicked_rail_letter() {
        let mut grid = GridState::new(GridInit {
            table_name: "people".to_string(),
            columns: vec![text_col("name")],
            fk_cols: vec![false],
            enumerated_values: vec![Vec::new()],
            rows: vec![vec![SqlValue::Text("Alice".to_string())]],
            width_sample_rows: vec![],
            total_rows: 300,
            area_width: 40,
        });
        grid.sort = Some(SortSpec {
            col_idx: 0,
            direction: SortDir::Asc,
        });

        let area = Rect {
            x: 0,
            y: 0,
            width: 40,
            height: 30,
        };

        assert_eq!(hit_test(area, &grid, 37, 4), Some('A'));
    }

    #[test]
    fn active_letter_tracks_focused_row_not_viewport_start() {
        let mut grid = GridState::new(GridInit {
            table_name: "people".to_string(),
            columns: vec![text_col("name")],
            fk_cols: vec![false],
            enumerated_values: vec![Vec::new()],
            rows: vec![
                vec![SqlValue::Text("Sally".to_string())],
                vec![SqlValue::Text("Tina".to_string())],
            ],
            width_sample_rows: vec![],
            total_rows: 300,
            area_width: 40,
        });
        grid.sort = Some(SortSpec {
            col_idx: 0,
            direction: SortDir::Asc,
        });
        grid.viewport_start = 0;
        grid.focused_row = 1;

        assert_eq!(active_letter(&grid), Some('T'));
    }
}
