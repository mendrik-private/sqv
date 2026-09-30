use ratatui::{
    layout::Rect,
    style::{Modifier, Style},
    Frame,
};
use unicode_width::UnicodeWidthStr;

use crate::app::{App, FocusPane};

pub enum TabMouseAction {
    Activate(usize),
    Close(usize),
}

pub fn superscript_for_tab(app: &App, idx: usize) -> Option<&str> {
    app.symbols.tab_shortcut(idx)
}

/// The display width of a tab's name plus shortcut, and of the whole tab laid
/// out as `│ name[sup] × │`, whose close glyph sits at offset `3 + label`.
fn tab_widths(app: &App, idx: usize) -> (u16, u16) {
    let name = app
        .open_tabs
        .get(idx)
        .map_or(0, |tab| UnicodeWidthStr::width(tab.table_name.as_str()));
    let shortcut = superscript_for_tab(app, idx).map_or(0, UnicodeWidthStr::width);
    let label = (name + shortcut) as u16;
    (label, label + 6)
}

pub fn render_tabbar(frame: &mut Frame, area: Rect, app: &App) {
    if area.width == 0 || area.height == 0 {
        return;
    }

    let theme = &app.theme;
    let buf = frame.buffer_mut();
    buf.set_style(area, Style::default().bg(theme.bg));

    let top_y = area.y;
    let label_y = area.y + if area.height > 1 { 1 } else { 0 };
    let join_y = area.y + area.height.saturating_sub(1);
    let has_roof = label_y > top_y;
    let has_join = area.height >= 3;

    let mut x = area.x;
    let right = area.x + area.width;

    if app.open_tabs.is_empty() {
        buf.set_string(
            x,
            label_y,
            " sqview ",
            Style::default()
                .fg(theme.accent)
                .bg(theme.bg_soft)
                .add_modifier(Modifier::BOLD),
        );
        return;
    }

    for (idx, tab) in app.open_tabs.iter().enumerate() {
        if x >= right {
            break;
        }

        let is_active = app.active_tab == Some(idx);
        let is_focused_active = is_active && matches!(app.focus, FocusPane::Grid);
        let border_style = Style::default()
            .fg(if is_focused_active {
                theme.accent
            } else {
                theme.line
            })
            .bg(theme.bg);
        let base = if is_active {
            Style::default()
                .fg(theme.fg)
                .bg(theme.bg_raised)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(theme.fg_dim).bg(theme.bg_soft)
        };
        let close_style = if is_focused_active {
            base.fg(theme.accent)
        } else {
            base.fg(theme.fg_mute)
        };
        let num_style = Style::default().fg(theme.fg_mute).bg(if is_active {
            theme.bg_raised
        } else {
            theme.bg_soft
        });

        let sup = superscript_for_tab(app, idx);
        let (_, tab_width) = tab_widths(app, idx);
        let tab_x = x;

        if has_roof {
            let mut roof_x = tab_x;
            roof_x = super::put(
                buf,
                roof_x,
                top_y,
                right,
                &app.symbols.tab_top_left.to_string(),
                border_style,
            );
            if tab_width > 2 {
                roof_x = super::put(
                    buf,
                    roof_x,
                    top_y,
                    right,
                    &app.symbols
                        .box_horizontal
                        .to_string()
                        .repeat(tab_width.saturating_sub(2) as usize),
                    border_style,
                );
            }
            super::put(
                buf,
                roof_x,
                top_y,
                right,
                &app.symbols.tab_top_right.to_string(),
                border_style,
            );
        }

        let mut label_x = tab_x;
        label_x = super::put(
            buf,
            label_x,
            label_y,
            right,
            &app.symbols.box_vertical.to_string(),
            border_style,
        );
        label_x = super::put(buf, label_x, label_y, right, " ", base);
        label_x = super::put(buf, label_x, label_y, right, &tab.table_name, base);
        if let Some(s) = sup {
            label_x = super::put(buf, label_x, label_y, right, s, num_style);
        }
        label_x = super::put(buf, label_x, label_y, right, " ", base);
        label_x = super::put(
            buf,
            label_x,
            label_y,
            right,
            &app.symbols.tab_close.to_string(),
            close_style,
        );
        label_x = super::put(buf, label_x, label_y, right, " ", base);
        super::put(
            buf,
            label_x,
            label_y,
            right,
            &app.symbols.box_vertical.to_string(),
            border_style,
        );

        if is_active && has_join {
            let mut join_x = tab_x;
            let left_join = if tab_x == area.x {
                app.symbols.box_vertical.to_string()
            } else {
                app.symbols.tab_join_left.to_string()
            };
            let right_join = if tab_x.saturating_add(tab_width) >= right {
                app.symbols.box_vertical.to_string()
            } else {
                app.symbols.tab_join_right.to_string()
            };
            join_x = super::put(buf, join_x, join_y, right, &left_join, border_style);
            if tab_width > 2 {
                join_x = super::put(
                    buf,
                    join_x,
                    join_y,
                    right,
                    &" ".repeat(tab_width.saturating_sub(2) as usize),
                    base,
                );
            }
            super::put(buf, join_x, join_y, right, &right_join, border_style);
        }

        x = tab_x.saturating_add(tab_width).min(right);
    }
}

pub fn hit_test(
    area: Rect,
    app: &App,
    x: u16,
    y: u16,
    middle_click: bool,
) -> Option<TabMouseAction> {
    if area.width == 0
        || area.height == 0
        || y < area.y
        || y >= area.y + area.height
        || x < area.x
        || x >= area.x + area.width
    {
        return None;
    }

    let mut cursor = area.x;
    let right = area.x + area.width;
    for idx in 0..app.open_tabs.len() {
        if cursor >= right {
            break;
        }
        let (label_w, tab_width) = tab_widths(app, idx);
        let tab_end = cursor.saturating_add(tab_width).min(right);
        if x >= cursor && x < tab_end {
            let close_x = cursor + 3 + label_w;
            if middle_click || x == close_x {
                return Some(TabMouseAction::Close(idx));
            }
            return Some(TabMouseAction::Activate(idx));
        }
        cursor = tab_end;
    }

    None
}
