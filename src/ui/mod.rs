pub mod popup;
pub mod sidebar;
pub mod statusbar;
pub mod tabbar;
pub mod toast;

use crate::app::{App, FocusPane};
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Modifier, Style},
    text::Span,
    widgets::{block::BorderType, Block, Paragraph},
    Frame,
};

pub fn render(frame: &mut Frame, app: &mut App) {
    let area = frame.area();
    app.screen_area = area;
    app.tabbar_area = Rect::default();
    app.sidebar_area = None;
    app.grid_outer_area = None;
    app.grid_inner_area = None;

    let vertical = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(0), Constraint::Length(1)])
        .split(area);

    statusbar::render_statusbar(frame, vertical[1], app);

    let main_area = vertical[0];

    let content_area = if app.sidebar_visible {
        let sidebar_width = (main_area.width / 3).clamp(20, 40);
        let horizontal = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Length(sidebar_width), Constraint::Min(0)])
            .split(main_area);

        let focused = matches!(app.focus, FocusPane::Sidebar);
        sidebar::render_sidebar(
            frame,
            horizontal[0],
            &app.schema,
            &mut app.sidebar,
            &app.theme,
            &app.symbols,
            focused,
        );
        app.sidebar_area = Some(horizontal[0]);
        horizontal[1]
    } else {
        main_area
    };

    let tabbar_height = if app.open_tabs.is_empty() {
        0
    } else {
        content_area.height.min(3)
    };
    let tabbar_area = Rect {
        x: content_area.x,
        y: content_area.y,
        width: content_area.width,
        height: tabbar_height,
    };
    let body_area = Rect {
        x: content_area.x,
        y: content_area.y + tabbar_height.saturating_sub(1),
        width: content_area.width,
        height: content_area
            .height
            .saturating_sub(tabbar_height.saturating_sub(1)),
    };
    app.tabbar_area = tabbar_area;

    frame.render_widget(
        Paragraph::new("").style(Style::default().bg(app.theme.bg)),
        content_area,
    );

    if let Some(ref mut grid) = app.grid {
        let border_color = if matches!(app.focus, FocusPane::Grid) {
            app.theme.accent
        } else {
            app.theme.line
        };
        let filter_count = grid
            .filter
            .columns
            .values()
            .map(|cf| cf.rules.iter().filter(|r| r.enabled).count())
            .sum::<usize>();
        let meta = {
            let mut parts = vec![format!("{} rows", fmt_count(grid.window.total_rows))];
            if filter_count > 0 {
                parts.push(format!("{} filters", filter_count));
            }
            if let Some(sort) = &grid.sort {
                if let Some(col) = grid.columns.get(sort.col_idx) {
                    let arrow = if sort.direction == crate::grid::SortDir::Asc {
                        app.symbols.sort_asc.to_string()
                    } else {
                        app.symbols.sort_desc.to_string()
                    };
                    parts.push(format!("sort: {} {}", col.name, arrow));
                }
            }
            parts.join(&app.symbols.inline_separator())
        };

        let block = Block::bordered()
            .style(Style::default().bg(app.theme.bg))
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(border_color))
            .title_bottom(Span::styled(meta, Style::default().fg(app.theme.fg_mute)));
        let inner = block.inner(body_area);
        app.grid_outer_area = Some(body_area);
        app.grid_inner_area = Some(inner);
        frame.render_widget(block, body_area);
        render_right_frame_label(
            frame,
            body_area,
            &format!(" {} ", grid.table_name),
            &app.theme,
        );
        crate::grid::render_grid(frame, inner, grid, &app.theme, &app.symbols);
    } else if let Some(active_idx) = app.active_tab {
        let tab = &app.open_tabs[active_idx];
        let block = Block::bordered()
            .style(Style::default().bg(app.theme.bg))
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(app.theme.line));
        let inner = block.inner(body_area);
        app.grid_outer_area = Some(body_area);
        app.grid_inner_area = Some(inner);
        frame.render_widget(block, body_area);
        render_right_frame_label(
            frame,
            body_area,
            &format!(" {} ", tab.table_name),
            &app.theme,
        );
        let msg = format!(" Loading {}...", tab.table_name);
        frame.render_widget(
            ratatui::widgets::Paragraph::new(msg)
                .style(ratatui::style::Style::default().fg(app.theme.fg_dim)),
            inner,
        );
    } else {
        let block = Block::bordered()
            .style(Style::default().bg(app.theme.bg))
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(app.theme.line));
        let inner = block.inner(body_area);
        app.grid_outer_area = Some(body_area);
        app.grid_inner_area = Some(inner);
        frame.render_widget(block, body_area);
    }

    if tabbar_area.height > 0 {
        tabbar::render_tabbar(frame, tabbar_area, app);
    }

    if let Some(ref mut popup) = app.popup {
        crate::ui::popup::render_popup(frame, area, popup, &app.theme, &app.symbols);
    }
    crate::ui::toast::render_toasts(frame, area, &app.toast, &app.theme);
    if let Some(ref confirm) = app.pending_confirm {
        crate::ui::toast::render_confirm(frame, area, &confirm.message, &app.theme);
    }
}

fn fmt_count(n: i64) -> String {
    let s = n.abs().to_string();
    let chars: Vec<char> = s.chars().collect();
    let grouped = chars
        .rchunks(3)
        .rev()
        .map(|chunk| chunk.iter().collect::<String>())
        .collect::<Vec<_>>()
        .join("\u{202F}");
    if n < 0 {
        format!("-{}", grouped)
    } else {
        grouped
    }
}

fn render_right_frame_label(
    frame: &mut Frame,
    area: Rect,
    label: &str,
    theme: &crate::theme::Theme,
) {
    if area.width <= 2 || area.height == 0 {
        return;
    }
    let max_width = area.width.saturating_sub(4) as usize;
    let trimmed: String = label.chars().take(max_width).collect();
    let label_width = trimmed.chars().count() as u16;
    if label_width == 0 {
        return;
    }
    let x = area.x + area.width.saturating_sub(label_width + 2);
    frame.buffer_mut().set_string(
        x,
        area.y,
        trimmed,
        Style::default()
            .fg(theme.accent)
            .bg(theme.bg)
            .add_modifier(Modifier::BOLD),
    );
}
