pub mod popup;
pub mod sidebar;
pub mod statusbar;
pub mod tabbar;
pub mod toast;
pub mod widgets;

use crate::app::{App, FocusPane};
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::Style,
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
        let meta = {
            let rows = widgets::text::group_thousands(grid.window.total_rows);
            let noun = if grid.window.total_rows == 1 {
                "row"
            } else {
                "rows"
            };
            let mut parts = vec![if grid.count_known {
                format!(" {rows} {noun} ")
            } else {
                format!(" {rows}{} {noun} ", app.symbols.ellipsis)
            }];
            if !grid.hidden.is_empty() {
                parts.push(format!(" {} hidden ", grid.hidden.len()));
            }
            if grid.frozen {
                parts.push(" first column frozen ".to_string());
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
        let insert_row = app.popup.as_ref().and_then(|popup| match popup {
            crate::ui::popup::PopupKind::InsertRow(state) => Some(state),
            _ => None,
        });
        crate::grid::render_grid(frame, inner, grid, insert_row, &app.theme, &app.symbols);
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
        let msg = format!(
            " Loading {}{}",
            widgets::text::sanitize(&tab.table_name),
            app.symbols.ellipsis
        );
        frame.render_widget(
            ratatui::widgets::Paragraph::new(msg).style(
                ratatui::style::Style::default()
                    .fg(app.theme.fg_dim)
                    .bg(app.theme.bg),
            ),
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
