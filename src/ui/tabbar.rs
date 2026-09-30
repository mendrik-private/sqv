use ratatui::{
    layout::Rect,
    style::{Modifier, Style},
    Frame,
};

use super::widgets::text::{put, sanitize, text_width};
use crate::app::{App, FocusPane};

pub enum TabMouseAction {
    Activate(usize),
    Close(usize),
}

/// Where one tab is drawn, laid out as `│ name[sup] × │`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct TabSlot {
    index: usize,
    x: u16,
    width: u16,
    /// Column of the close glyph.
    close_x: u16,
}

/// The tabs that fit, scrolled so the active tab is always visible, and
/// whether tabs are hidden to the left or right. Shared by drawing and
/// hit-testing so the two cannot disagree.
struct TabLayout {
    slots: Vec<TabSlot>,
    more_left: bool,
    more_right: bool,
}

/// Label width (name plus shortcut) and whole tab width of each tab.
fn tab_widths(app: &App) -> Vec<(u16, u16)> {
    app.open_tabs
        .iter()
        .enumerate()
        .map(|(index, tab)| {
            let name = text_width(&sanitize(&tab.table_name));
            let shortcut = app.symbols.tab_shortcut(index).map_or(0, text_width);
            let label = (name + shortcut) as u16;
            (label, label + 6)
        })
        .collect()
}

fn layout(area: Rect, app: &App) -> TabLayout {
    let widths = tab_widths(app);
    let active = app
        .active_tab
        .unwrap_or(0)
        .min(widths.len().saturating_sub(1));
    // Scroll right until the active tab fits, leaving room for the left marker.
    let mut first = 0;
    while first < active {
        let marker = u16::from(first > 0);
        let used: u16 = widths[first..=active].iter().map(|(_, w)| *w).sum();
        if marker + used <= area.width.saturating_sub(1) {
            break;
        }
        first += 1;
    }
    let more_left = first > 0;
    let mut x = area.x + u16::from(more_left);
    let right = area.right();
    let mut slots = Vec::new();
    let mut more_right = false;
    for (index, &(label, width)) in widths.iter().enumerate().skip(first) {
        let reserve = u16::from(index + 1 < widths.len());
        if x + width + reserve > right && !slots.is_empty() {
            more_right = true;
            break;
        }
        slots.push(TabSlot {
            index,
            x,
            width: width.min(right.saturating_sub(x)),
            close_x: x + 3 + label,
        });
        x += width;
    }
    TabLayout {
        slots,
        more_left,
        more_right,
    }
}

pub fn render_tabbar(frame: &mut Frame, area: Rect, app: &App) {
    if area.width == 0 || area.height == 0 || app.open_tabs.is_empty() {
        return;
    }
    let theme = &app.theme;
    let symbols = &app.symbols;
    let buf = frame.buffer_mut();
    buf.set_style(area, Style::default().bg(theme.bg));

    let top_y = area.y;
    let label_y = area.y + u16::from(area.height > 1);
    let join_y = area.bottom() - 1;
    let has_roof = label_y > top_y;
    let has_join = area.height >= 3;
    let right = area.right();
    let tabs = layout(area, app);
    let marker_style = Style::default().fg(theme.fg_mute).bg(theme.bg);
    if tabs.more_left {
        put(buf, area.x, label_y, right, "‹", marker_style);
    }
    if tabs.more_right {
        put(buf, right - 1, label_y, right, "›", marker_style);
    }

    for slot in &tabs.slots {
        let tab = &app.open_tabs[slot.index];
        let is_active = app.active_tab == Some(slot.index);
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
        let close_style = base.fg(if is_focused_active {
            theme.accent
        } else {
            theme.fg_mute
        });
        let num_style = Style::default().fg(theme.fg_mute).bg(if is_active {
            theme.bg_raised
        } else {
            theme.bg_soft
        });
        let end = (slot.x + slot.width).min(right);
        let inner = slot.width.saturating_sub(2) as usize;

        if has_roof {
            let roof = format!(
                "{}{}{}",
                symbols.tab_top_left,
                symbols.box_horizontal.to_string().repeat(inner),
                symbols.tab_top_right
            );
            put(buf, slot.x, top_y, end, &roof, border_style);
        }

        let mut x = put(
            buf,
            slot.x,
            label_y,
            end,
            &symbols.box_vertical.to_string(),
            border_style,
        );
        x = put(buf, x, label_y, end, " ", base);
        x = put(buf, x, label_y, end, &sanitize(&tab.table_name), base);
        if let Some(shortcut) = symbols.tab_shortcut(slot.index) {
            x = put(buf, x, label_y, end, shortcut, num_style);
        }
        x = put(buf, x, label_y, end, " ", base);
        x = put(
            buf,
            x,
            label_y,
            end,
            &symbols.tab_close.to_string(),
            close_style,
        );
        x = put(buf, x, label_y, end, " ", base);
        put(
            buf,
            x,
            label_y,
            end,
            &symbols.box_vertical.to_string(),
            border_style,
        );

        if is_active && has_join {
            let left_join = if slot.x == area.x {
                symbols.box_vertical
            } else {
                symbols.tab_join_left
            };
            let right_join = if slot.x + slot.width >= right {
                symbols.box_vertical
            } else {
                symbols.tab_join_right
            };
            let x = put(
                buf,
                slot.x,
                join_y,
                end,
                &left_join.to_string(),
                border_style,
            );
            let x = put(buf, x, join_y, end, &" ".repeat(inner), base);
            put(buf, x, join_y, end, &right_join.to_string(), border_style);
        }
    }
}

pub fn hit_test(
    area: Rect,
    app: &App,
    x: u16,
    y: u16,
    middle_click: bool,
) -> Option<TabMouseAction> {
    if !area.contains(ratatui::layout::Position { x, y }) {
        return None;
    }
    let slot = layout(area, app)
        .slots
        .into_iter()
        .find(|slot| x >= slot.x && x < slot.x + slot.width)?;
    if middle_click || x == slot.close_x {
        Some(TabMouseAction::Close(slot.index))
    } else {
        Some(TabMouseAction::Activate(slot.index))
    }
}
