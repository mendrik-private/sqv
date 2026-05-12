use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::{Color, Modifier, Style},
    Frame,
};

use crate::{
    app::{App, AppMode, FocusPane},
    db::types::SqlValue,
};

fn fmt_number(n: i64) -> String {
    let s = n.abs().to_string();
    let chars: Vec<char> = s.chars().collect();
    let grouped: String = chars
        .rchunks(3)
        .rev()
        .map(|c| c.iter().collect::<String>())
        .collect::<Vec<_>>()
        .join("\u{202F}");
    if n < 0 {
        format!("-{}", grouped)
    } else {
        grouped
    }
}

pub fn render_statusbar(frame: &mut Frame, area: Rect, app: &App) {
    if area.width == 0 || area.height == 0 {
        return;
    }

    let table_name: String = app
        .active_tab
        .and_then(|i| app.open_tabs.get(i))
        .map(|t| t.table_name.clone())
        .unwrap_or_else(|| app.symbols.empty_placeholder.to_string());

    let (row_num, total_rows, col_num) = app.grid.as_ref().map_or((0i64, 0i64, 0usize), |g| {
        (
            g.focused_row as i64 + 1,
            g.window.total_rows,
            g.focused_col + 1,
        )
    });

    let cell_preview: String = app
        .grid
        .as_ref()
        .and_then(|g| {
            let abs_row = g.focused_row as i64;
            let col_idx = g.focused_col;
            g.window
                .get_row(abs_row)
                .and_then(|row| row.get(col_idx))
                .map(|val| match val {
                    SqlValue::Null => "NULL".to_string(),
                    SqlValue::Integer(n) => n.to_string(),
                    SqlValue::Real(f) => format!("{}", f),
                    SqlValue::Text(s) => s.chars().take(50).collect(),
                    SqlValue::Blob(b) => format!("<blob {} bytes>", b.len()),
                })
        })
        .unwrap_or_default();

    let pos_str = if total_rows > 0 {
        format!(
            "r {}/{}{}col {}",
            fmt_number(row_num),
            fmt_number(total_rows),
            app.symbols.inline_separator(),
            col_num
        )
    } else {
        String::new()
    };

    let theme = &app.theme;

    let filter_count = app
        .grid
        .as_ref()
        .map(|g| {
            g.filter
                .columns
                .values()
                .map(|cf| cf.rules.iter().filter(|r| r.enabled).count())
                .sum::<usize>()
        })
        .unwrap_or(0);

    let sort_str = app
        .grid
        .as_ref()
        .and_then(|g| {
            g.sort.as_ref().and_then(|s| {
                let col_name = g.columns.get(s.col_idx).map(|c| c.name.as_str())?;
                let arrow = if s.direction == crate::grid::SortDir::Asc {
                    app.symbols.sort_asc.to_string()
                } else {
                    app.symbols.sort_desc.to_string()
                };
                Some(format!("{} {}", arrow, col_name))
            })
        })
        .unwrap_or_default();

    let mut segments: Vec<(String, Style)> = vec![
        (
            " BROWSE ".to_string(),
            Style::default()
                .fg(theme.bg)
                .bg(theme.accent)
                .add_modifier(Modifier::BOLD),
        ),
        (
            table_name,
            Style::default()
                .fg(theme.fg_dim)
                .bg(theme.bg)
                .add_modifier(Modifier::BOLD),
        ),
    ];

    if filter_count > 0 {
        segments.push((
            format!("{} {} filters", app.symbols.filter_icon, filter_count),
            Style::default().fg(theme.red).bg(theme.bg_soft),
        ));
    }

    if !sort_str.is_empty() {
        segments.push((
            sort_str,
            Style::default().fg(theme.accent).bg(theme.bg_soft),
        ));
    }

    if !pos_str.is_empty() {
        segments.push((
            pos_str,
            Style::default().fg(theme.fg_mute).bg(theme.bg_soft),
        ));
    }

    if !app.jump_stack.is_empty() {
        let current_table = app.grid.as_ref().map_or_else(
            || app.symbols.empty_placeholder.to_string(),
            |g| g.table_name.clone(),
        );
        let crumb: String = app
            .jump_stack
            .iter()
            .map(|f| f.table.as_str())
            .chain(std::iter::once(current_table.as_str()))
            .collect::<Vec<_>>()
            .join(&app.symbols.breadcrumb_separator);
        segments.push((
            format!("{} {}", app.symbols.breadcrumb_prefix, crumb),
            Style::default().fg(theme.accent).bg(theme.bg_soft),
        ));
    }

    if let Some(hints) = action_hint_text(app) {
        segments.push((hints, Style::default().fg(theme.accent).bg(theme.bg_soft)));
    }

    let buf = frame.buffer_mut();
    buf.set_style(area, Style::default().bg(theme.bg_soft));

    let preview = truncate_preview(&cell_preview, area.width as usize / 3, app.symbols.ellipsis);
    let preview_width = preview.chars().count() as u16;
    let (content_right, preview_x) = preview_layout(area, preview_width);

    let mut x = area.x;
    for (idx, (text, style)) in segments.iter().enumerate() {
        if x >= content_right {
            break;
        }
        x = put(buf, x, area.y, content_right, text, *style);
        if idx + 1 < segments.len() && x < content_right {
            x = put(
                buf,
                x,
                area.y,
                content_right,
                &app.symbols.segment_separator(),
                Style::default().fg(theme.line).bg(theme.bg_soft),
            );
        }
    }

    if !preview.is_empty() && preview_x < area.x + area.width {
        let _ = put(
            buf,
            preview_x,
            area.y,
            area.x + area.width,
            &preview,
            preview_style(theme),
        );
    }
}

fn truncate_preview(s: &str, max_chars: usize, ellipsis: char) -> String {
    if max_chars == 0 {
        return String::new();
    }
    let mut out: String = s.chars().take(max_chars).collect();
    if s.chars().count() > max_chars && max_chars > 1 {
        out.pop();
        out.push(ellipsis);
    }
    out
}

fn preview_layout(area: Rect, preview_width: u16) -> (u16, u16) {
    let right = area.x + area.width;
    if preview_width == 0 {
        return (right, right);
    }

    let preview_x = right.saturating_sub(preview_width + 1);
    let content_right = preview_x.saturating_sub(1);
    (content_right, preview_x)
}

fn preview_style(theme: &crate::theme::Theme) -> Style {
    Style::default().fg(Color::Black).bg(theme.accent)
}

fn action_hint_text(app: &App) -> Option<String> {
    if app.popup.is_some() || app.mode != AppMode::Browse {
        return None;
    }

    let mut hints = Vec::new();

    match app.focus {
        FocusPane::Sidebar => {
            hints.push("[enter] open".to_string());
            hints.push(format!(
                "[{}/{} h/l] fold",
                app.symbols.arrow_left, app.symbols.arrow_right
            ));
            if app.sidebar_visible {
                hints.push("[tab] panel".to_string());
            }
        }
        FocusPane::Grid => {
            if app.grid.is_some() && !app.readonly {
                hints.push("[enter] open".to_string());
                hints.push("[e] modify".to_string());
            }
            if app.grid.is_some() {
                hints.push("[s] sort".to_string());
                hints.push("[f] filter".to_string());
                hints.push("[ctrl-f] find".to_string());
            }
            if app.sidebar_visible {
                hints.push("[tab] panel".to_string());
            }
        }
    }

    hints.push("[ctrl-h] help".to_string());
    hints.push("[ctrl-q] quit".to_string());

    if hints.is_empty() {
        None
    } else {
        Some(hints.join("  "))
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use r2d2_sqlite::SqliteConnectionManager;
    use ratatui::style::Color;
    use tokio::sync::mpsc;

    use super::{action_hint_text, preview_layout, preview_style};
    use crate::{
        app::{App, FocusPane},
        config::Config,
        db::{self, schema::Column, types::SqlValue},
        grid::{GridInit, GridState},
    };

    fn make_test_app() -> App {
        let manager = SqliteConnectionManager::memory();
        let pool = Arc::new(
            r2d2::Pool::builder()
                .max_size(1)
                .build(manager)
                .expect("test pool"),
        );
        let conn = pool.get().expect("test conn");
        conn.execute_batch(
            "CREATE TABLE users (id INTEGER PRIMARY KEY, name TEXT, age INTEGER, email TEXT);",
        )
        .expect("seed schema");
        let schema = db::load_schema(&conn).expect("load schema");
        drop(conn);

        let (tx, _rx) = mpsc::unbounded_channel();
        App::new(
            schema,
            Config::default(),
            pool,
            tx,
            false,
            ":memory:".to_string(),
        )
    }

    fn make_grid() -> GridState {
        let columns = vec![
            Column {
                cid: 0,
                name: "id".to_string(),
                col_type: "INTEGER".to_string(),
                not_null: false,
                default_value: None,
                is_pk: true,
            },
            Column {
                cid: 1,
                name: "name".to_string(),
                col_type: "TEXT".to_string(),
                not_null: false,
                default_value: None,
                is_pk: false,
            },
        ];
        GridState::new(GridInit {
            table_name: "users".to_string(),
            columns,
            fk_cols: vec![false; 2],
            enumerated_values: vec![Vec::new(); 2],
            rows: vec![vec![
                SqlValue::Integer(1),
                SqlValue::Text("Alice".to_string()),
            ]],
            width_sample_rows: vec![],
            total_rows: 1,
            area_width: 40,
        })
    }

    #[test]
    fn grid_hints_focus_on_primary_actions() {
        let mut app = make_test_app();
        app.grid = Some(make_grid());
        app.focus = FocusPane::Grid;

        let hints = action_hint_text(&app).expect("grid hints");

        assert!(hints.contains("[enter] open"));
        assert!(hints.contains("[e] modify"));
        assert!(hints.contains("[f] filter"));
        assert!(hints.contains("[ctrl-f] find"));
        assert!(hints.contains("[ctrl-h] help"));
        assert!(!hints.contains("[n] set null"));
        assert!(!hints.contains("[i] add row"));
        assert!(!hints.contains("[d] delete row"));
    }

    #[test]
    fn grid_hints_leave_secondary_actions_in_help_dialog() {
        let mut app = make_test_app();
        let mut grid = make_grid();
        grid.window.rows[0][1] = SqlValue::Null;
        grid.focused_col = 1;
        app.grid = Some(grid);
        app.focus = FocusPane::Grid;

        let hints = action_hint_text(&app).expect("grid hints");

        assert!(!hints.contains("[y]"));
        assert!(!hints.contains("[Y]"));
        assert!(!hints.contains("[n] set null"));
    }

    #[test]
    fn sidebar_hints_include_ctrl_h_help() {
        let mut app = make_test_app();
        app.focus = FocusPane::Sidebar;

        let hints = action_hint_text(&app).expect("sidebar hints");

        assert!(hints.contains("[ctrl-h] help"));
        assert!(hints.contains("[enter] open"));
    }

    #[test]
    fn preview_layout_reserves_gap_before_preview_text() {
        let (content_right, preview_x) = preview_layout(
            ratatui::layout::Rect {
                x: 0,
                y: 0,
                width: 20,
                height: 1,
            },
            5,
        );

        assert_eq!(content_right, 13);
        assert_eq!(preview_x, 14);
    }

    #[test]
    fn preview_layout_uses_full_width_when_preview_is_empty() {
        let (content_right, preview_x) = preview_layout(
            ratatui::layout::Rect {
                x: 3,
                y: 0,
                width: 20,
                height: 1,
            },
            0,
        );

        assert_eq!(content_right, 23);
        assert_eq!(preview_x, 23);
    }

    #[test]
    fn preview_style_uses_black_text_on_accent_background() {
        let theme = crate::theme::Theme::default();
        let style = preview_style(&theme);

        assert_eq!(style.fg, Some(Color::Black));
        assert_eq!(style.bg, Some(theme.accent));
    }
}

fn put(buf: &mut Buffer, mut x: u16, y: u16, right: u16, text: &str, style: Style) -> u16 {
    for ch in text.chars() {
        if x >= right {
            break;
        }
        buf.set_string(x, y, ch.to_string(), style);
        x += 1;
    }
    x
}
