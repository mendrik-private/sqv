use ratatui::{
    layout::Rect,
    style::{Modifier, Style},
    Frame,
};

use super::{
    popup::{relative_time::relative_label, PopupKind},
    widgets::{
        hints::{hint, render_hints, Hint},
        text::{group_thousands, put, sanitize, text_width, truncate_with_ellipsis},
    },
};
use crate::{
    app::{App, FocusPane},
    db::types::SqlValue,
    grid::SortDir,
};

/// The mode the badge names: what keys currently do.
pub(crate) fn mode_label(app: &App) -> &'static str {
    if app.pending_confirm.is_some() {
        return "CONFIRM";
    }
    match &app.popup {
        None => "BROWSE",
        Some(popup) => match popup {
            PopupKind::TextEditor(_)
            | PopupKind::ValuePicker(_)
            | PopupKind::DatePicker(_)
            | PopupKind::FkPicker(_) => "EDIT",
            PopupKind::InsertRow(_) => "INSERT",
            PopupKind::FilterPopup(_) => "FILTER",
            PopupKind::Find(_) => "FIND",
            PopupKind::CommandPalette(_) => "COMMAND",
            PopupKind::Help(_) => "HELP",
            PopupKind::GoToRow(_) => "GO TO",
            PopupKind::Record(_) => "RECORD",
            PopupKind::Schema(_) => "SCHEMA",
            PopupKind::SqlConsole(_) => "SQL",
            PopupKind::GlobalSearch(_) => "SEARCH",
            PopupKind::Export(_) => "EXPORT",
            PopupKind::References(_) => "REFERENCES",
            PopupKind::Json(_) => "JSON",
        },
    }
}

pub fn render_statusbar(frame: &mut Frame, area: Rect, app: &App) {
    if area.width == 0 || area.height == 0 {
        return;
    }
    let theme = &app.theme;
    let symbols = &app.symbols;
    let bar_bg = theme.bg_soft;
    let grid = app.grid.as_ref();

    let mut segments: Vec<(String, Style)> = vec![(
        format!(" {} ", mode_label(app)),
        Style::default()
            .fg(theme.bg)
            .bg(theme.accent)
            .add_modifier(Modifier::BOLD),
    )];
    if app.is_readonly_view() {
        segments.push((
            " READ-ONLY ".to_string(),
            Style::default()
                .fg(theme.bg)
                .bg(theme.yellow)
                .add_modifier(Modifier::BOLD),
        ));
    }
    let table_name = app.active_table_name().map_or_else(
        || symbols.empty_placeholder.to_string(),
        |name| sanitize(&name).into_owned(),
    );
    segments.push((
        table_name,
        Style::default()
            .fg(theme.fg_dim)
            .bg(bar_bg)
            .add_modifier(Modifier::BOLD),
    ));

    if let Some(grid) = grid {
        let filters = grid.filter.active_count();
        if filters > 0 {
            let noun = if filters == 1 { "filter" } else { "filters" };
            segments.push((
                format!("{} {filters} {noun}", symbols.filter_icon),
                Style::default().fg(theme.fg_dim).bg(bar_bg),
            ));
        }
        let sort = grid
            .sort
            .iter()
            .filter_map(|spec| {
                let name = grid.columns.get(spec.col_idx)?;
                let arrow = match spec.direction {
                    SortDir::Asc => symbols.sort_asc,
                    SortDir::Desc => symbols.sort_desc,
                };
                Some(format!("{arrow} {}", sanitize(&name.name)))
            })
            .collect::<Vec<_>>()
            .join(", ");
        if !sort.is_empty() {
            segments.push((sort, Style::default().fg(theme.accent).bg(bar_bg)));
        }
        let selected = grid.selected_row_count();
        if selected > 0 {
            let text = if selected == grid.window.total_rows.max(0) as usize {
                "sel all".to_string()
            } else {
                format!("sel {}", group_thousands(selected as i64))
            };
            segments.push((text, Style::default().fg(theme.yellow).bg(bar_bg)));
        }
        if grid.window.total_rows > 0 {
            let total = if grid.count_known {
                group_thousands(grid.window.total_rows)
            } else {
                format!(
                    "{}{}",
                    group_thousands(grid.window.total_rows),
                    symbols.ellipsis
                )
            };
            segments.push((
                format!(
                    "r {}/{}{}col {}",
                    group_thousands(grid.focused_row as i64 + 1),
                    total,
                    symbols.inline_separator(),
                    grid.display_columns()
                        .iter()
                        .position(|&col| col == grid.focused_col)
                        .map_or(0, |position| position + 1)
                ),
                Style::default().fg(theme.fg_mute).bg(bar_bg),
            ));
        }
    }

    if !app.jump_stack.is_empty() {
        let current = grid.map_or_else(
            || symbols.empty_placeholder.to_string(),
            |g| g.table_name.clone(),
        );
        let crumb = app
            .jump_stack
            .iter()
            .map(|frame| frame.table.as_str())
            .chain(std::iter::once(current.as_str()))
            .collect::<Vec<_>>()
            .join(&symbols.breadcrumb_separator);
        segments.push((
            format!("{} {}", symbols.breadcrumb_prefix, sanitize(&crumb)),
            Style::default().fg(theme.accent).bg(bar_bg),
        ));
    }

    let buf = frame.buffer_mut();
    buf.set_style(area, Style::default().bg(bar_bg));

    let preview = truncate_with_ellipsis(
        &cell_preview(app),
        area.width as usize / 3,
        symbols.ellipsis,
    );
    let (content_right, preview_x) = preview_layout(area, text_width(&preview) as u16);

    let mut x = area.x;
    for (index, (text, style)) in segments.iter().enumerate() {
        if x >= content_right {
            break;
        }
        if index > 0 {
            x = put(
                buf,
                x,
                area.y,
                content_right,
                &symbols.segment_separator(),
                Style::default().fg(theme.line).bg(bar_bg),
            );
        }
        x = put(buf, x, area.y, content_right, text, *style);
    }
    let hints = action_hints(app);
    if !hints.is_empty() && x + 2 < content_right {
        render_hints(
            buf,
            Rect::new(x + 2, area.y, content_right - x - 2, 1),
            &hints,
            theme,
            bar_bg,
        );
    }
    if !preview.is_empty() && preview_x < area.right() {
        put(
            buf,
            preview_x,
            area.y,
            area.right(),
            &preview,
            preview_style(theme),
        );
    }
}

/// The focused value, with a relative age for dates.
fn cell_preview(app: &App) -> String {
    let Some(grid) = app.grid.as_ref() else {
        return String::new();
    };
    let Some(value) = grid
        .window
        .get_row(grid.focused_row as i64)
        .and_then(|row| row.get(grid.focused_col))
    else {
        return String::new();
    };
    let text: String = sanitize(&value.to_text()).chars().take(80).collect();
    let temporal = grid
        .kinds
        .get(grid.focused_col)
        .is_some_and(|kind| kind.is_temporal());
    match value {
        SqlValue::Text(raw) if temporal => match relative_label(raw) {
            Some(label) => format!("{text} ({label})"),
            None => text,
        },
        _ => text,
    }
}

fn preview_layout(area: Rect, preview_width: u16) -> (u16, u16) {
    let right = area.right();
    if preview_width == 0 {
        return (right, right);
    }
    let preview_x = right.saturating_sub(preview_width + 1);
    (preview_x.saturating_sub(1), preview_x)
}

fn preview_style(theme: &crate::theme::Theme) -> Style {
    Style::default().fg(theme.bg).bg(theme.accent)
}

/// Keys that act right now, most useful first; hints that do not fit are
/// dropped from the end.
pub(crate) fn action_hints(app: &App) -> Vec<Hint> {
    if app.pending_confirm.is_some() {
        return vec![hint("y", "delete"), hint("n / Esc", "keep")];
    }
    match &app.popup {
        // The staged row is edited inline in the grid, which has no footer.
        Some(PopupKind::InsertRow(_)) => {
            return vec![
                hint("Alt-Enter", "save row"),
                hint("Tab / Shift-Tab", "next / previous field"),
                hint("Shift-Del", "reset field"),
                hint("Esc", "discard"),
            ];
        }
        Some(_) => return Vec::new(),
        None => {}
    }
    let mut hints = Vec::new();
    match app.focus {
        FocusPane::Sidebar => {
            let selected = app.sidebar.selected_name(&app.schema);
            let is_index = selected
                .as_ref()
                .is_some_and(|name| app.schema.indexes.contains(name));
            match selected {
                None => hints.push(hint("Enter", "fold")),
                Some(_) if is_index => hints.push(hint("Enter", "schema")),
                Some(_) => {
                    hints.push(hint("Enter", "open"));
                    hints.push(hint("i", "schema"));
                }
            }
            if app.active_tab.is_some() {
                hints.push(hint("Tab", "table"));
            }
        }
        FocusPane::Grid => {
            if let Some(grid) = app.grid.as_ref() {
                if !app.is_readonly_view() {
                    hints.push(hint("Enter", "edit"));
                }
                if grid.fk_cols.get(grid.focused_col).copied().unwrap_or(false) {
                    hints.push(hint("j", "follow link"));
                }
                if !app.jump_stack.is_empty() {
                    hints.push(hint("Backspace", "back"));
                }
                hints.push(hint("v", "record"));
                hints.push(hint("f", "filter"));
                hints.push(hint("s", "sort"));
                hints.push(hint("Ctrl-F", "find"));
            }
        }
    }
    hints.push(hint("?", "help"));
    hints.push(hint("Ctrl-P", "commands"));
    hints
}

#[cfg(test)]
mod tests {
    use super::*;

    fn labels(hints: &[Hint]) -> Vec<String> {
        hints
            .iter()
            .map(|h| format!("{} {}", h.keys, h.label))
            .collect()
    }

    fn test_app() -> App {
        let pool = std::sync::Arc::new(
            r2d2::Pool::builder()
                .max_size(1)
                .build(r2d2_sqlite::SqliteConnectionManager::memory())
                .expect("pool"),
        );
        let conn = pool.get().expect("conn");
        conn.execute_batch("CREATE TABLE users (id INTEGER PRIMARY KEY, name TEXT);")
            .expect("schema");
        let schema = crate::db::load_schema(&conn).expect("load");
        drop(conn);
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        App::new(
            schema,
            crate::config::Config::default(),
            pool,
            tx,
            false,
            ":memory:".into(),
        )
    }

    #[test]
    fn sidebar_hints_follow_the_selected_row() {
        let mut app = test_app();
        assert!(labels(&action_hints(&app)).contains(&"Enter fold".to_string()));
        app.sidebar.move_down(&app.schema);
        let hints = labels(&action_hints(&app));
        assert!(hints.contains(&"Enter open".to_string()));
        assert!(hints.contains(&"i schema".to_string()));
        assert!(hints.contains(&"? help".to_string()));
    }

    #[test]
    fn read_only_mode_hides_edit_hints_and_shows_the_badge_mode() {
        let mut app = test_app();
        app.readonly = true;
        app.focus = FocusPane::Grid;
        assert!(!labels(&action_hints(&app))
            .iter()
            .any(|h| h.starts_with("Enter")));
        assert!(app.is_readonly_view());
        assert_eq!(mode_label(&app), "BROWSE");
    }

    #[test]
    fn preview_layout_reserves_gap_before_preview_text() {
        assert_eq!(preview_layout(Rect::new(0, 0, 20, 1), 5), (13, 14));
    }

    #[test]
    fn preview_layout_uses_full_width_when_preview_is_empty() {
        assert_eq!(preview_layout(Rect::new(3, 0, 20, 1), 0), (23, 23));
    }

    #[test]
    fn preview_text_uses_the_canvas_colour_on_accent() {
        let theme = crate::theme::Theme::default();
        let style = preview_style(&theme);
        assert_eq!(style.fg, Some(theme.bg));
        assert_eq!(style.bg, Some(theme.accent));
    }
}
