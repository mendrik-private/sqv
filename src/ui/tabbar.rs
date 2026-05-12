use ratatui::{
    buffer::Buffer,
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
        let sup_w: u16 = sup
            .map(|value| UnicodeWidthStr::width(value) as u16)
            .unwrap_or(0);
        let name_w = tab.table_name.chars().count() as u16;
        // │ space name [sup] space × space │
        let content_width = 1 + name_w + sup_w + 1 + 1 + 1;
        let tab_width = content_width + 2;
        let tab_x = x;

        if has_roof {
            let mut roof_x = tab_x;
            roof_x = put(
                buf,
                roof_x,
                top_y,
                right,
                &app.symbols.tab_top_left.to_string(),
                border_style,
            );
            if tab_width > 2 {
                roof_x = put(
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
            put(
                buf,
                roof_x,
                top_y,
                right,
                &app.symbols.tab_top_right.to_string(),
                border_style,
            );
        }

        let mut label_x = tab_x;
        label_x = put(
            buf,
            label_x,
            label_y,
            right,
            &app.symbols.box_vertical.to_string(),
            border_style,
        );
        label_x = put(buf, label_x, label_y, right, " ", base);
        label_x = put(buf, label_x, label_y, right, &tab.table_name, base);
        if let Some(s) = sup {
            label_x = put(buf, label_x, label_y, right, s, num_style);
        }
        label_x = put(buf, label_x, label_y, right, " ", base);
        label_x = put(
            buf,
            label_x,
            label_y,
            right,
            &app.symbols.tab_close.to_string(),
            close_style,
        );
        label_x = put(buf, label_x, label_y, right, " ", base);
        put(
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
            join_x = put(buf, join_x, join_y, right, &left_join, border_style);
            if tab_width > 2 {
                join_x = put(
                    buf,
                    join_x,
                    join_y,
                    right,
                    &" ".repeat(tab_width.saturating_sub(2) as usize),
                    base,
                );
            }
            put(buf, join_x, join_y, right, &right_join, border_style);
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
    for (idx, tab) in app.open_tabs.iter().enumerate() {
        if cursor >= right {
            break;
        }
        let name_w = tab.table_name.chars().count() as u16;
        let sup_w: u16 = superscript_for_tab(app, idx)
            .map(|value| UnicodeWidthStr::width(value) as u16)
            .unwrap_or(0);
        let tab_width = name_w + sup_w + 6;
        let tab_end = cursor.saturating_add(tab_width).min(right);
        if x >= cursor && x < tab_end {
            // │(1) space(1) name(name_w) [sup(sup_w)] space(1) → × is at cursor+3+name_w+sup_w
            let close_x = cursor + 3 + name_w + sup_w;
            if middle_click || x == close_x {
                return Some(TabMouseAction::Close(idx));
            }
            return Some(TabMouseAction::Activate(idx));
        }
        cursor = tab_end;
    }

    None
}

fn put(buf: &mut Buffer, mut x: u16, y: u16, right: u16, text: &str, style: Style) -> u16 {
    for ch in text.chars() {
        if x >= right {
            break;
        }
        let s = ch.to_string();
        buf.set_string(x, y, s, style);
        x += 1;
    }
    x
}
