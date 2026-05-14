pub mod alphabet_rail;
pub mod layout;
pub mod virtual_scroll;

use std::collections::HashMap;

use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::{Color, Modifier, Style},
    Frame,
};
use unicode_width::UnicodeWidthStr;

use crate::{
    db::{
        schema::Column,
        types::{affinity, ColAffinity, SqlValue},
    },
    symbols::Symbols,
    theme::Theme,
    ui::popup::InsertRowState,
};

#[derive(Debug, Clone, PartialEq)]
pub enum SortDir {
    Asc,
    Desc,
}

#[derive(Debug, Clone)]
pub struct SortSpec {
    pub col_idx: usize,
    pub direction: SortDir,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RowSelection {
    None,
    Range { anchor: usize, head: usize },
    All,
}

pub struct GridInit {
    pub table_name: String,
    pub columns: Vec<Column>,
    pub fk_cols: Vec<bool>,
    pub enumerated_values: Vec<Vec<String>>,
    pub rows: Vec<Vec<SqlValue>>,
    pub width_sample_rows: Vec<Vec<SqlValue>>,
    pub total_rows: i64,
    pub area_width: u16,
}

#[allow(dead_code)]
pub struct GridState {
    pub table_name: String,
    pub columns: Vec<Column>,
    pub window: virtual_scroll::VirtualWindow,
    pub width_sample_rows: Vec<Vec<SqlValue>>,
    pub focused_row: usize,
    pub focused_col: usize,
    pub col_widths: Vec<u16>,
    pub h_scroll: usize,
    pub fk_cols: Vec<bool>,
    pub enumerated_values: Vec<Vec<String>>,
    pub manual_widths: HashMap<usize, u16>,
    pub needs_fetch: bool,
    pub viewport_start: i64,
    pub avail_col_width: u16,
    pub sort: Option<SortSpec>,
    pub filter: crate::filter::FilterSet,
    pub row_selection: RowSelection,
}

const HEADER_ROWS: u16 = 3;
const VIEWPORT_SCROLL_MARGIN_ROWS: usize = 5;

impl GridState {
    pub fn new(init: GridInit) -> Self {
        let GridInit {
            table_name,
            columns,
            fk_cols,
            enumerated_values,
            rows,
            width_sample_rows,
            total_rows,
            area_width,
        } = init;
        let col_count = columns.len();
        let fk_cols_safe = if fk_cols.len() == col_count {
            fk_cols
        } else {
            vec![false; col_count]
        };
        let enumerated_values_safe = if enumerated_values.len() == col_count {
            enumerated_values
        } else {
            vec![Vec::new(); col_count]
        };
        let mut state = Self {
            table_name,
            columns,
            window: virtual_scroll::VirtualWindow::new(0, rows, total_rows),
            width_sample_rows,
            focused_row: 0,
            focused_col: 0,
            col_widths: Vec::new(),
            h_scroll: 0,
            fk_cols: fk_cols_safe,
            enumerated_values: enumerated_values_safe,
            manual_widths: HashMap::new(),
            needs_fetch: false,
            viewport_start: 0,
            avail_col_width: area_width,
            sort: None,
            filter: crate::filter::FilterSet::default(),
            row_selection: RowSelection::None,
        };
        state.recompute_col_widths(area_width);
        state
    }

    pub fn recompute_col_widths(&mut self, avail_width: u16) {
        let sizing_rows = if self.width_sample_rows.is_empty() {
            &self.window.rows
        } else {
            &self.width_sample_rows
        };
        self.col_widths = layout::compute_col_widths(
            &self.columns,
            sizing_rows,
            avail_width,
            &self.manual_widths,
            &self.fk_cols,
        );
        self.avail_col_width = avail_width;
        self.adjust_h_scroll();
    }

    pub fn scroll_down(&mut self, n: usize) {
        let max_row = (self.window.total_rows - 1).max(0);
        let new_focused = (self.focused_row as i64 + n as i64).min(max_row);
        self.focused_row = new_focused as usize;
        self.adjust_viewport();
        self.check_needs_fetch();
    }

    pub fn scroll_up(&mut self, n: usize) {
        self.focused_row = self.focused_row.saturating_sub(n);
        self.adjust_viewport();
        self.check_needs_fetch();
    }

    pub fn scroll_to_row(&mut self, abs_row: i64) {
        let max_row = (self.window.total_rows - 1).max(0);
        self.focused_row = abs_row.clamp(0, max_row) as usize;
        self.adjust_viewport();
        self.check_needs_fetch();
    }

    pub fn scroll_to_end(&mut self) {
        let max_row = (self.window.total_rows - 1).max(0) as usize;
        self.focused_row = max_row;
        self.adjust_viewport();
        self.check_needs_fetch();
    }

    fn adjust_viewport(&mut self) {
        let vp = self.window.viewport_rows.max(1);
        let fr = self.focused_row as i64;
        let total = self.window.total_rows;
        let margin = VIEWPORT_SCROLL_MARGIN_ROWS.min(vp.saturating_sub(1)) as i64;

        if fr < self.viewport_start + margin {
            self.viewport_start = fr - margin;
        } else if fr >= self.viewport_start + vp as i64 - margin {
            self.viewport_start = fr - vp as i64 + margin + 1;
        }

        self.viewport_start = self.viewport_start.max(0);
        let max_start = (total - vp as i64).max(0);
        self.viewport_start = self.viewport_start.min(max_start);
    }

    fn check_needs_fetch(&mut self) {
        if !self.window.fetch_in_flight && self.window.needs_prefetch(self.focused_row as i64) {
            self.needs_fetch = true;
        }
    }

    pub fn move_col_right(&mut self) {
        if self.focused_col + 1 < self.columns.len() {
            self.focused_col += 1;
            self.adjust_h_scroll();
        }
    }

    pub fn move_col_left(&mut self) {
        if self.focused_col > 0 {
            self.focused_col -= 1;
            self.adjust_h_scroll();
        }
    }

    pub fn move_col_first(&mut self) {
        self.focused_col = 0;
        self.h_scroll = 0;
    }

    pub fn move_col_last(&mut self) {
        if !self.columns.is_empty() {
            self.focused_col = self.columns.len() - 1;
            self.adjust_h_scroll();
        }
    }

    pub fn clear_row_selection(&mut self) {
        self.row_selection = RowSelection::None;
    }

    pub fn extend_row_selection_down(&mut self, n: usize) {
        if self.window.total_rows <= 0 {
            return;
        }
        if matches!(self.row_selection, RowSelection::All) {
            self.scroll_down(n);
            return;
        }
        let anchor = match self.row_selection {
            RowSelection::Range { anchor, .. } => anchor,
            RowSelection::None => self.focused_row,
            RowSelection::All => self.focused_row,
        };
        self.scroll_down(n);
        self.row_selection = RowSelection::Range {
            anchor,
            head: self.focused_row,
        };
    }

    pub fn extend_row_selection_up(&mut self, n: usize) {
        if self.window.total_rows <= 0 {
            return;
        }
        if matches!(self.row_selection, RowSelection::All) {
            self.scroll_up(n);
            return;
        }
        let anchor = match self.row_selection {
            RowSelection::Range { anchor, .. } => anchor,
            RowSelection::None => self.focused_row,
            RowSelection::All => self.focused_row,
        };
        self.scroll_up(n);
        self.row_selection = RowSelection::Range {
            anchor,
            head: self.focused_row,
        };
    }

    pub fn select_all_rows(&mut self) {
        self.row_selection = if self.window.total_rows > 0 {
            RowSelection::All
        } else {
            RowSelection::None
        };
    }

    pub fn has_row_selection(&self) -> bool {
        !matches!(self.row_selection, RowSelection::None)
    }

    pub fn selected_row_range(&self) -> Option<(usize, usize)> {
        match self.row_selection {
            RowSelection::Range { anchor, head } => {
                let start = anchor.min(head);
                let end = anchor.max(head);
                Some((start, end))
            }
            RowSelection::None | RowSelection::All => None,
        }
    }

    pub fn is_row_selected(&self, abs_row: i64) -> bool {
        if abs_row < 0 {
            return false;
        }
        let abs_row = abs_row as usize;
        match self.row_selection {
            RowSelection::None => false,
            RowSelection::Range { .. } => self
                .selected_row_range()
                .is_some_and(|(start, end)| abs_row >= start && abs_row <= end),
            RowSelection::All => abs_row < self.window.total_rows.max(0) as usize,
        }
    }

    pub fn focus_cell(&mut self, row: usize, col: usize) {
        self.clear_row_selection();
        if self.columns.is_empty() {
            self.focused_row = row.min(self.window.total_rows.saturating_sub(1) as usize);
            self.focused_col = 0;
            return;
        }
        self.focused_row = row.min(self.window.total_rows.saturating_sub(1) as usize);
        self.focused_col = col.min(self.columns.len() - 1);
        self.adjust_viewport();
        self.adjust_h_scroll();
        self.check_needs_fetch();
    }

    fn adjust_h_scroll(&mut self) {
        if self.focused_col < self.h_scroll {
            self.h_scroll = self.focused_col;
            return;
        }
        let avail = self.avail_col_width as usize;
        if avail == 0 {
            return;
        }
        let mut cumul = 0usize;
        let mut visible_end = self.h_scroll;
        for col_idx in self.h_scroll..self.col_widths.len() {
            let w = self.col_widths[col_idx] as usize;
            if cumul + w > avail {
                break;
            }
            cumul += w;
            visible_end = col_idx + 1;
        }
        while self.focused_col >= visible_end && self.h_scroll < self.focused_col {
            self.h_scroll += 1;
            cumul = 0;
            visible_end = self.h_scroll;
            for col_idx in self.h_scroll..self.col_widths.len() {
                let w = self.col_widths[col_idx] as usize;
                if cumul + w > avail {
                    break;
                }
                cumul += w;
                visible_end = col_idx + 1;
            }
        }
    }
}

// ── rendering helpers ────────────────────────────────────────────────────────

fn digits(n: i64) -> usize {
    if n == 0 {
        1
    } else {
        n.unsigned_abs().to_string().len()
    }
}

fn format_thousands(n: i64) -> String {
    n.to_string()
}

fn truncate_to_display_width(s: &str, max_w: usize) -> String {
    if max_w == 0 {
        return String::new();
    }
    let mut result = String::new();
    let mut cur_w = 0usize;
    for c in s.chars() {
        let cw = unicode_width::UnicodeWidthChar::width(c).unwrap_or(1);
        if cur_w + cw > max_w {
            break;
        }
        result.push(c);
        cur_w += cw;
    }
    result
}

fn col_badge(col: &Column) -> &'static str {
    let upper = col.col_type.to_uppercase();
    if upper.contains("DATETIME") || upper.contains("TIMESTAMP") {
        return "DT ";
    }
    if upper.contains("DATE") {
        return "DAT";
    }
    match affinity(&col.col_type) {
        ColAffinity::Integer => "INT",
        ColAffinity::Real => "REA",
        ColAffinity::Text => "TXT",
        ColAffinity::Blob => "BLB",
        ColAffinity::Numeric => "NUM",
    }
}

fn badge_color(col: &Column, theme: &Theme) -> Color {
    let upper = col.col_type.to_uppercase();
    if upper.contains("DATETIME") || upper.contains("TIMESTAMP") || upper.contains("DATE") {
        return theme.pink;
    }
    match affinity(&col.col_type) {
        ColAffinity::Integer => theme.yellow,
        ColAffinity::Real => theme.blue,
        ColAffinity::Text => theme.teal,
        ColAffinity::Blob => theme.purple,
        ColAffinity::Numeric => theme.yellow,
    }
}

fn enum_value_color(value: &str, enum_values: &[String], theme: &Theme) -> Color {
    if let Some(index) = enum_values.iter().position(|candidate| candidate == value) {
        return indexed_enum_color(index, theme);
    }

    let hash = value.bytes().fold(0u64, |acc, byte| {
        acc.wrapping_mul(131).wrapping_add(byte as u64)
    });
    indexed_enum_color(hash as usize, theme)
}

fn indexed_enum_color(index: usize, theme: &Theme) -> Color {
    let base_palette = [
        theme.teal,
        theme.blue,
        theme.green,
        theme.yellow,
        theme.purple,
        theme.pink,
        theme.accent,
        theme.red,
    ];
    let base = base_palette[index % base_palette.len()];
    let variant = index / base_palette.len();
    match variant {
        0 => base,
        1 => mix_color(base, theme.fg, 0.24),
        2 => mix_color(base, theme.fg_dim, 0.12),
        _ => mix_color(base, theme.bg_raised, 0.08),
    }
}

fn mix_color(base: Color, target: Color, ratio: f32) -> Color {
    let (br, bg, bb) = color_rgb(base);
    let (tr, tg, tb) = color_rgb(target);
    let mix = |from: u8, to: u8| -> u8 {
        let from = from as f32;
        let to = to as f32;
        ((from + (to - from) * ratio).round()).clamp(0.0, 255.0) as u8
    };
    Color::Rgb(mix(br, tr), mix(bg, tg), mix(bb, tb))
}

fn color_rgb(color: Color) -> (u8, u8, u8) {
    match color {
        Color::Rgb(r, g, b) => (r, g, b),
        Color::Indexed(v) => (v, v, v),
        Color::Reset => (0, 0, 0),
        Color::Black => (0, 0, 0),
        Color::Red => (255, 0, 0),
        Color::Green => (0, 255, 0),
        Color::Yellow => (255, 255, 0),
        Color::Blue => (0, 0, 255),
        Color::Magenta => (255, 0, 255),
        Color::Cyan => (0, 255, 255),
        Color::Gray => (128, 128, 128),
        Color::DarkGray => (64, 64, 64),
        Color::LightRed => (255, 102, 102),
        Color::LightGreen => (102, 255, 102),
        Color::LightYellow => (255, 255, 153),
        Color::LightBlue => (102, 178, 255),
        Color::LightMagenta => (255, 102, 255),
        Color::LightCyan => (102, 255, 255),
        Color::White => (255, 255, 255),
    }
}

#[derive(Clone, Copy)]
enum CellAlign {
    Left,
    Right,
    Center,
}

fn format_cell_content(
    val: &SqlValue,
    col: &Column,
    inner_w: usize,
    symbols: &Symbols,
) -> (String, CellAlign) {
    let col_upper = col.col_type.to_uppercase();
    match val {
        SqlValue::Null => (truncate_to_display_width("NULL", inner_w), CellAlign::Left),
        SqlValue::Integer(n) => {
            if col_upper.contains("BOOL") {
                let s = if *n != 0 {
                    symbols.bool_true
                } else {
                    symbols.bool_false
                };
                (s.to_string(), CellAlign::Center)
            } else {
                (
                    truncate_to_display_width(&format_thousands(*n), inner_w),
                    CellAlign::Right,
                )
            }
        }
        SqlValue::Real(f) => (
            truncate_to_display_width(&format!("{:.6}", f), inner_w),
            CellAlign::Right,
        ),
        SqlValue::Text(t) => (truncate_to_display_width(t, inner_w), CellAlign::Left),
        SqlValue::Blob(b) => (
            truncate_to_display_width(&format!("<blob {} bytes>", b.len()), inner_w),
            CellAlign::Left,
        ),
    }
}

fn cell_val_style(
    val: &SqlValue,
    col: &Column,
    theme: &Theme,
    is_focused: bool,
    enum_values: &[String],
) -> Style {
    if is_focused {
        return Style::default()
            .fg(theme.accent)
            .add_modifier(Modifier::BOLD);
    }
    let col_upper = col.col_type.to_uppercase();
    match val {
        SqlValue::Null => Style::default()
            .fg(theme.fg_faint)
            .add_modifier(Modifier::ITALIC),
        SqlValue::Blob(_) => Style::default()
            .fg(theme.purple)
            .add_modifier(Modifier::ITALIC),
        SqlValue::Integer(n) => {
            if col_upper.contains("BOOL") {
                if *n != 0 {
                    Style::default().fg(theme.green)
                } else {
                    Style::default().fg(theme.fg_faint)
                }
            } else {
                Style::default().fg(theme.blue)
            }
        }
        SqlValue::Real(_) => Style::default().fg(theme.blue),
        SqlValue::Text(text) => {
            if col_upper.contains("DATETIME")
                || col_upper.contains("TIMESTAMP")
                || col_upper.contains("DATE")
            {
                Style::default().fg(theme.pink).add_modifier(Modifier::DIM)
            } else if !enum_values.is_empty() {
                Style::default().fg(enum_value_color(text, enum_values, theme))
            } else {
                Style::default()
                    .fg(theme.fg_dim)
                    .add_modifier(Modifier::DIM)
            }
        }
    }
}

// ── sub-render functions ─────────────────────────────────────────────────────

fn compute_visible_cols(state: &GridState, data_width: u16) -> Vec<(usize, u16)> {
    let mut visible_cols = Vec::new();
    let mut cumul = 0u16;
    for col_idx in state.h_scroll..state.col_widths.len() {
        let w = state.col_widths[col_idx];
        if visible_cols.is_empty() || cumul + w <= data_width {
            visible_cols.push((col_idx, w));
            cumul += w;
        } else {
            break;
        }
    }

    if cumul < data_width {
        let extra = data_width - cumul;
        let text_cols: Vec<usize> = visible_cols
            .iter()
            .enumerate()
            .filter_map(|(visible_idx, (col_idx, _))| {
                matches!(
                    affinity(&state.columns[*col_idx].col_type),
                    ColAffinity::Text | ColAffinity::Blob
                )
                .then_some(visible_idx)
            })
            .collect();

        let targets = if text_cols.is_empty() {
            visible_cols
                .is_empty()
                .then(Vec::new)
                .unwrap_or_else(|| vec![visible_cols.len() - 1])
        } else {
            text_cols
        };

        if !targets.is_empty() {
            let base = extra / targets.len() as u16;
            let remainder = extra % targets.len() as u16;
            for (i, target_idx) in targets.into_iter().enumerate() {
                visible_cols[target_idx].1 = visible_cols[target_idx]
                    .1
                    .saturating_add(base + u16::from(i < remainder as usize));
            }
        }
    }

    visible_cols
}

fn render_header(
    buf: &mut Buffer,
    area: Rect,
    gutter_width: u16,
    visible_cols: &[(usize, u16)],
    state: &GridState,
    theme: &Theme,
    symbols: &Symbols,
) {
    let header_y = area.y;
    let header_style = Style::default().bg(theme.bg_raised);
    for y in 0..HEADER_ROWS.min(area.height) {
        buf.set_string(
            area.x,
            header_y + y,
            " ".repeat(area.width as usize),
            header_style,
        );
    }

    let mut col_x = area.x + gutter_width;
    for &(col_idx, cell_w) in visible_cols {
        if col_x >= area.x + area.width {
            break;
        }
        let col = &state.columns[col_idx];
        let actual_w = cell_w.min(area.x + area.width - col_x);

        let badge = col_badge(col);
        let bcolor = badge_color(col, theme);
        let is_pk = col.is_pk;
        let is_fk = state.fk_cols.get(col_idx).copied().unwrap_or(false);
        let pfx = match (is_pk, is_fk) {
            (true, true) => Some(format!(" {} {}", symbols.pk_icon, symbols.fk_icon)),
            (true, false) => Some(format!(" {}", symbols.pk_icon)),
            (false, true) => Some(format!(" {}", symbols.fk_icon)),
            (false, false) => None,
        };

        for y in 0..HEADER_ROWS.min(area.height) {
            buf.set_string(
                col_x,
                header_y + y,
                " ".repeat(actual_w as usize),
                header_style,
            );
        }

        let sort_arrow: Option<String> = if let Some(s) = &state.sort {
            if s.col_idx == col_idx {
                Some(if s.direction == SortDir::Asc {
                    symbols.sort_asc.to_string()
                } else {
                    symbols.sort_desc.to_string()
                })
            } else {
                None
            }
        } else {
            None
        };

        let filter_active = state
            .filter
            .columns
            .get(&col.name)
            .is_some_and(|cf| cf.rules.iter().any(|r| r.enabled));

        let arrow_reserve = if sort_arrow.is_some() { 2usize } else { 0usize };
        let max_name_w = (actual_w as usize).saturating_sub(2 + arrow_reserve);
        let name_truncated = truncate_to_display_width(&format!(" {}", col.name), max_name_w);
        buf.set_string(
            col_x,
            header_y,
            &name_truncated,
            Style::default()
                .bg(theme.bg_raised)
                .fg(theme.fg)
                .add_modifier(Modifier::BOLD),
        );

        let sort_x = col_x + actual_w.saturating_sub(2);
        if let Some(arrow) = sort_arrow {
            if sort_x > col_x && sort_x < area.x + area.width {
                buf.set_string(
                    sort_x,
                    header_y,
                    &arrow,
                    Style::default().fg(theme.accent).bg(theme.bg_raised),
                );
            }
        }

        let meta_text = match (pfx.as_deref(), filter_active) {
            (Some(pfx), true) => format!("{}{pfx} {}", badge.trim_end(), symbols.filter_marker),
            (Some(pfx), false) => format!("{}{pfx}", badge.trim_end()),
            (None, true) => format!("{} {}", badge.trim_end(), symbols.filter_marker),
            (None, false) => badge.trim_end().to_string(),
        };
        let meta_truncated =
            truncate_to_display_width(&format!(" {}", meta_text), actual_w as usize);
        if HEADER_ROWS > 1 && header_y + 1 < area.y + area.height {
            buf.set_string(
                col_x,
                header_y + 1,
                &meta_truncated,
                Style::default()
                    .bg(theme.bg_raised)
                    .fg(bcolor)
                    .add_modifier(Modifier::DIM),
            );
        }

        if col_x > area.x + gutter_width {
            buf.set_string(
                col_x,
                header_y,
                symbols.box_vertical.to_string(),
                Style::default().fg(theme.line).bg(theme.bg_raised),
            );
            if HEADER_ROWS > 1 && header_y + 1 < area.y + area.height {
                buf.set_string(
                    col_x,
                    header_y + 1,
                    symbols.box_vertical.to_string(),
                    Style::default().fg(theme.line).bg(theme.bg_raised),
                );
            }
        }

        col_x += cell_w;
    }

    let divider_y = area.y + HEADER_ROWS - 1;
    if divider_y < area.y + area.height {
        buf.set_string(
            area.x,
            divider_y,
            symbols
                .box_horizontal
                .to_string()
                .repeat(area.width as usize),
            Style::default().fg(theme.line).bg(theme.bg_raised),
        );
    }

    if visible_cols
        .last()
        .is_some_and(|(last_idx, _)| last_idx + 1 < state.col_widths.len())
    {
        let chevron_reserve = if alphabet_rail::should_show_rail(state) {
            alphabet_rail::RAIL_WIDTH + 1
        } else {
            1
        };
        let chevron_x = area.x + area.width.saturating_sub(chevron_reserve + 1);
        if chevron_x >= area.x + gutter_width && chevron_x < area.x + area.width {
            buf.set_string(
                chevron_x,
                header_y,
                symbols.selection.to_string(),
                Style::default().fg(theme.fg_mute).bg(theme.bg_raised),
            );
        }
    }
}

fn render_data_rows(
    buf: &mut Buffer,
    area: Rect,
    gutter_width: u16,
    visible_cols: &[(usize, u16)],
    state: &GridState,
    insert_row: Option<&InsertRowState>,
    theme: &Theme,
    symbols: &Symbols,
) {
    let gutter_digits = digits(state.window.total_rows.max(1));
    let viewport_rows = state.window.viewport_rows;
    let display_start = display_viewport_start(state, insert_row);
    let display_total_rows = total_display_rows(state, insert_row);
    for row_in_view in 0..viewport_rows {
        let display_abs_row = display_start + row_in_view as i64;
        if display_abs_row >= display_total_rows {
            break;
        }
        let row_y = area.y + HEADER_ROWS + row_in_view as u16;
        if row_y >= area.y + area.height {
            break;
        }

        match display_row_kind(state, insert_row, display_abs_row) {
            Some(VisibleGridRow::Data { real_abs }) => render_existing_row(
                buf,
                area,
                gutter_width,
                visible_cols,
                state,
                theme,
                symbols,
                gutter_digits,
                row_y,
                real_abs,
            ),
            Some(VisibleGridRow::Insert(insert_state)) => render_insert_row(
                buf,
                area,
                gutter_width,
                visible_cols,
                theme,
                symbols,
                gutter_digits,
                row_y,
                insert_state,
            ),
            None => break,
        }
    }
}

fn render_focused_border(
    buf: &mut Buffer,
    area: Rect,
    gutter_width: u16,
    visible_cols: &[(usize, u16)],
    state: &GridState,
    insert_row: Option<&InsertRowState>,
    theme: &Theme,
    symbols: &Symbols,
) {
    if let Some(insert_row) = insert_row {
        let focused_col = insert_row.selected;
        let focused_row_in_view =
            insert_row.insert_position as i64 - display_viewport_start(state, Some(insert_row));
        if focused_row_in_view < 0 || focused_row_in_view >= state.window.viewport_rows as i64 {
            return;
        }
        let vis_pos = match visible_cols.iter().position(|&(c, _)| c == focused_col) {
            Some(p) => p,
            None => return,
        };
        let cell_y = area.y + HEADER_ROWS + focused_row_in_view as u16;
        let mut cell_x = area.x + gutter_width;
        for &(_, cell_w) in &visible_cols[..vis_pos] {
            cell_x += cell_w;
        }
        let cell_w = visible_cols[vis_pos].1;
        let focused_bg = insert_row_background(theme, true);
        draw_cell_border(
            buf, area, cell_x, cell_y, cell_w, focused_bg, theme, symbols,
        );
        return;
    }

    let focused_row_in_view = state.focused_row as i64 - state.viewport_start;
    if focused_row_in_view < 0 || focused_row_in_view >= state.window.viewport_rows as i64 {
        return;
    }
    let focused_col = state.focused_col;

    let vis_pos = match visible_cols.iter().position(|&(c, _)| c == focused_col) {
        Some(p) => p,
        None => return,
    };

    let cell_y = area.y + HEADER_ROWS + focused_row_in_view as u16;
    if cell_y >= area.y + area.height {
        return;
    }

    let mut cell_x = area.x + gutter_width;
    for &(_, cell_w) in &visible_cols[..vis_pos] {
        cell_x += cell_w;
    }
    let cell_w = visible_cols[vis_pos].1;
    let focused_bg = row_background(state, theme, state.focused_row as i64, true);
    draw_cell_border(
        buf, area, cell_x, cell_y, cell_w, focused_bg, theme, symbols,
    );
}

fn draw_cell_border(
    buf: &mut Buffer,
    area: Rect,
    cell_x: u16,
    cell_y: u16,
    cell_w: u16,
    cell_bg: Color,
    theme: &Theme,
    symbols: &Symbols,
) {
    if cell_y >= area.y + area.height || cell_x >= area.x + area.width || cell_w < 2 {
        return;
    }

    let border_style = Style::default()
        .fg(theme.accent)
        .bg(cell_bg)
        .remove_modifier(Modifier::all());
    let right_x = cell_x + cell_w - 1;

    if cell_y >= area.y + HEADER_ROWS {
        let ty = cell_y - 1;
        buf.set_string(cell_x, ty, symbols.focus_top_left.to_string(), border_style);
        if cell_w > 2 {
            let mid = symbols
                .box_horizontal
                .to_string()
                .repeat((cell_w - 2) as usize);
            buf.set_string(cell_x + 1, ty, &mid, border_style);
        }
        if right_x < area.x + area.width {
            buf.set_string(
                right_x,
                ty,
                symbols.focus_top_right.to_string(),
                border_style,
            );
        }
    }

    buf.set_string(
        cell_x,
        cell_y,
        symbols.box_vertical.to_string(),
        border_style,
    );
    if right_x < area.x + area.width {
        buf.set_string(
            right_x,
            cell_y,
            symbols.box_vertical.to_string(),
            border_style,
        );
    }

    let by = cell_y + 1;
    if by < area.y + area.height {
        buf.set_string(
            cell_x,
            by,
            symbols.focus_bottom_left.to_string(),
            border_style,
        );
        if cell_w > 2 {
            let mid = symbols
                .box_horizontal
                .to_string()
                .repeat((cell_w - 2) as usize);
            buf.set_string(cell_x + 1, by, &mid, border_style);
        }
        if right_x < area.x + area.width {
            buf.set_string(
                right_x,
                by,
                symbols.focus_bottom_right.to_string(),
                border_style,
            );
        }
    }
}

fn row_background(state: &GridState, theme: &Theme, abs_row: i64, is_focused: bool) -> Color {
    let base = if is_focused {
        theme.bg_raised
    } else if abs_row % 2 == 0 {
        theme.bg
    } else {
        theme.bg_soft
    };
    if state.is_row_selected(abs_row) {
        mix_color(base, theme.accent, if is_focused { 0.28 } else { 0.18 })
    } else {
        base
    }
}

fn insert_row_background(theme: &Theme, is_selected_cell: bool) -> Color {
    if is_selected_cell {
        mix_color(theme.bg_raised, theme.accent, 0.16)
    } else {
        mix_color(theme.bg_raised, theme.accent, 0.08)
    }
}

enum VisibleGridRow<'a> {
    Data { real_abs: i64 },
    Insert(&'a InsertRowState),
}

fn total_display_rows(state: &GridState, insert_row: Option<&InsertRowState>) -> i64 {
    state.window.total_rows + i64::from(insert_row.is_some())
}

fn display_viewport_start(state: &GridState, insert_row: Option<&InsertRowState>) -> i64 {
    if let Some(insert_row) = insert_row {
        if (insert_row.insert_position as i64) < state.viewport_start {
            state.viewport_start + 1
        } else {
            state.viewport_start
        }
    } else {
        state.viewport_start
    }
}

fn display_row_kind<'a>(
    state: &GridState,
    insert_row: Option<&'a InsertRowState>,
    display_abs_row: i64,
) -> Option<VisibleGridRow<'a>> {
    if let Some(insert_row) = insert_row {
        let insert_pos = insert_row.insert_position as i64;
        if display_abs_row == insert_pos {
            return Some(VisibleGridRow::Insert(insert_row));
        }
        let real_abs = if display_abs_row > insert_pos {
            display_abs_row - 1
        } else {
            display_abs_row
        };
        if real_abs >= 0 && real_abs < state.window.total_rows {
            Some(VisibleGridRow::Data { real_abs })
        } else {
            None
        }
    } else if display_abs_row >= 0 && display_abs_row < state.window.total_rows {
        Some(VisibleGridRow::Data {
            real_abs: display_abs_row,
        })
    } else {
        None
    }
}

fn render_existing_row(
    buf: &mut Buffer,
    area: Rect,
    gutter_width: u16,
    visible_cols: &[(usize, u16)],
    state: &GridState,
    theme: &Theme,
    symbols: &Symbols,
    gutter_digits: usize,
    row_y: u16,
    abs_row: i64,
) {
    let is_focused = abs_row == state.focused_row as i64;
    let is_selected = state.is_row_selected(abs_row);
    let row_bg = row_background(state, theme, abs_row, is_focused);

    buf.set_string(
        area.x,
        row_y,
        " ".repeat(area.width as usize),
        Style::default().bg(row_bg),
    );

    let row_num_str = format!("{:>width$} ", abs_row + 1, width = gutter_digits);
    let gutter_fg = if is_focused || is_selected {
        theme.accent
    } else {
        theme.fg_faint
    };
    buf.set_string(
        area.x,
        row_y,
        &row_num_str,
        Style::default().bg(row_bg).fg(gutter_fg),
    );

    let mut col_x = area.x + gutter_width;

    if let Some(row_data) = state.window.get_row(abs_row) {
        for &(col_idx, cell_w) in visible_cols {
            if col_x >= area.x + area.width {
                break;
            }
            let col = &state.columns[col_idx];
            let actual_w = cell_w.min(area.x + area.width - col_x);
            let inner_w = (actual_w as usize).saturating_sub(2);

            let is_focused_cell = is_focused && col_idx == state.focused_col;
            if actual_w > 0 {
                buf.set_string(
                    col_x,
                    row_y,
                    " ".repeat(actual_w as usize),
                    Style::default().bg(row_bg),
                );
            }

            if let Some(val) = row_data.get(col_idx) {
                let (content, align) = format_cell_content(val, col, inner_w, symbols);
                let enum_values = state
                    .enumerated_values
                    .get(col_idx)
                    .map(Vec::as_slice)
                    .unwrap_or(&[]);
                let style =
                    cell_val_style(val, col, theme, is_focused_cell, enum_values).bg(row_bg);
                let display_w = UnicodeWidthStr::width(content.as_str());
                let content_x = match align {
                    CellAlign::Left => col_x + 1,
                    CellAlign::Right => col_x + 1 + inner_w.saturating_sub(display_w) as u16,
                    CellAlign::Center => col_x + 1 + (inner_w.saturating_sub(display_w) / 2) as u16,
                };
                buf.set_string(content_x, row_y, &content, style);
            }

            col_x += cell_w;
        }
    } else {
        buf.set_string(
            area.x + gutter_width,
            row_y,
            symbols.ellipsis.to_string(),
            Style::default().fg(theme.fg_faint).bg(row_bg),
        );
    }
}

fn render_insert_row(
    buf: &mut Buffer,
    area: Rect,
    gutter_width: u16,
    visible_cols: &[(usize, u16)],
    theme: &Theme,
    symbols: &Symbols,
    gutter_digits: usize,
    row_y: u16,
    insert_row: &InsertRowState,
) {
    let row_bg = insert_row_background(theme, false);
    buf.set_string(
        area.x,
        row_y,
        " ".repeat(area.width as usize),
        Style::default().bg(row_bg),
    );

    let row_num_str = format!("{:>width$} ", "+", width = gutter_digits);
    buf.set_string(
        area.x,
        row_y,
        &row_num_str,
        Style::default()
            .bg(row_bg)
            .fg(theme.accent)
            .add_modifier(Modifier::BOLD),
    );

    let mut col_x = area.x + gutter_width;
    for &(col_idx, cell_w) in visible_cols {
        if col_x >= area.x + area.width {
            break;
        }
        let actual_w = cell_w.min(area.x + area.width - col_x);
        let inner_w = (actual_w as usize).saturating_sub(2);
        let selected = col_idx == insert_row.selected;
        let cell_bg = insert_row_background(theme, selected);

        if actual_w > 0 {
            buf.set_string(
                col_x,
                row_y,
                " ".repeat(actual_w as usize),
                Style::default().bg(cell_bg),
            );
        }

        if let Some(field) = insert_row.fields.get(col_idx) {
            let content = truncate_to_display_width(
                &field.grid_display_value(selected, symbols.cursor),
                inner_w,
            );
            let style = if selected {
                Style::default()
                    .fg(if field.is_valid() {
                        theme.accent
                    } else {
                        theme.red
                    })
                    .bg(cell_bg)
                    .add_modifier(Modifier::BOLD)
            } else if field.is_valid() {
                Style::default().fg(theme.fg_dim).bg(cell_bg)
            } else {
                Style::default().fg(theme.red).bg(cell_bg)
            };
            buf.set_string(col_x + 1, row_y, &content, style);
        }

        col_x += cell_w;
    }
}

#[derive(Debug, Clone, Copy)]
struct ScrollbarMetrics {
    track_x: u16,
    track_y_start: u16,
    track_height: i64,
    thumb_height: i64,
    thumb_offset: i64,
}

fn vertical_scrollbar_metrics(area: Rect, state: &GridState) -> Option<ScrollbarMetrics> {
    let total = state.window.total_rows;
    if total <= 0 || area.width == 0 {
        return None;
    }

    let track_height = area.height.saturating_sub(HEADER_ROWS) as i64;
    if track_height <= 0 {
        return None;
    }

    let track_x = area.x + area.width - 1;
    let track_y_start = area.y + HEADER_ROWS;
    let thumb_height = ((state.window.viewport_rows as i64 * track_height) / total.max(1))
        .max(1)
        .min(track_height);
    let thumb_offset = if total > 1 {
        (state.focused_row as i64 * (track_height - thumb_height)) / (total - 1)
    } else {
        0
    };
    let max_thumb_offset = (track_height - thumb_height).max(0);

    Some(ScrollbarMetrics {
        track_x,
        track_y_start,
        track_height,
        thumb_height,
        thumb_offset: thumb_offset.clamp(0, max_thumb_offset),
    })
}

fn render_vertical_scrollbar(
    buf: &mut Buffer,
    area: Rect,
    state: &GridState,
    theme: &Theme,
    symbols: &Symbols,
) {
    let Some(metrics) = vertical_scrollbar_metrics(area, state) else {
        return;
    };

    for ty in 0..metrics.track_height {
        buf.set_string(
            metrics.track_x,
            metrics.track_y_start + ty as u16,
            symbols.box_vertical.to_string(),
            Style::default().fg(theme.line_soft),
        );
    }

    for ty in 0..metrics.thumb_height {
        let ty_abs = metrics.track_y_start + (metrics.thumb_offset + ty) as u16;
        if ty_abs < area.y + area.height {
            buf.set_string(
                metrics.track_x,
                ty_abs,
                symbols.scrollbar_thumb.to_string(),
                Style::default().fg(theme.fg_mute),
            );
        }
    }
}

pub(crate) fn scrollbar_drag_start(area: Rect, state: &GridState, y: u16) -> Option<(i64, i64)> {
    let metrics = vertical_scrollbar_metrics(area, state)?;
    if y < metrics.track_y_start || y >= metrics.track_y_start + metrics.track_height as u16 {
        return None;
    }

    let thumb_top = metrics.track_y_start + metrics.thumb_offset as u16;
    let thumb_bottom = thumb_top + metrics.thumb_height as u16;
    let grab_offset = if y >= thumb_top && y < thumb_bottom {
        i64::from(y - thumb_top)
    } else {
        metrics.thumb_height / 2
    };

    scrollbar_drag_target_row(area, state, y, grab_offset).map(|row| (grab_offset, row))
}

pub(crate) fn scrollbar_drag_target_row(
    area: Rect,
    state: &GridState,
    y: u16,
    grab_offset: i64,
) -> Option<i64> {
    let metrics = vertical_scrollbar_metrics(area, state)?;
    if state.window.total_rows <= 1 {
        return Some(0);
    }

    let max_thumb_offset = (metrics.track_height - metrics.thumb_height).max(0);
    if max_thumb_offset == 0 {
        return Some(0);
    }

    let pointer_offset = i64::from(y).saturating_sub(i64::from(metrics.track_y_start));
    let thumb_offset = pointer_offset
        .saturating_sub(grab_offset)
        .clamp(0, max_thumb_offset);

    Some(
        ((thumb_offset * (state.window.total_rows - 1)) + (max_thumb_offset / 2))
            / max_thumb_offset,
    )
}

fn render_loading_indicator(
    buf: &mut Buffer,
    area: Rect,
    state: &GridState,
    theme: &Theme,
    symbols: &Symbols,
) {
    if !state.window.fetch_in_flight {
        return;
    }
    let steps = area.height.saturating_sub(HEADER_ROWS + 1) as u64;
    if steps == 0 {
        return;
    }
    let pos = (state.window.tick_count / 15) % steps;
    let y = area.y + HEADER_ROWS + pos as u16;
    let x = area.x + area.width.saturating_sub(1);
    if y < area.y + area.height {
        buf.set_string(
            x,
            y,
            symbols.loading.to_string(),
            Style::default().fg(theme.accent).bg(theme.bg),
        );
    }
}

// ── public render entry point ────────────────────────────────────────────────

pub fn render_grid(
    frame: &mut Frame,
    area: Rect,
    state: &mut GridState,
    insert_row: Option<&InsertRowState>,
    theme: &Theme,
    symbols: &Symbols,
) {
    if area.width == 0 || area.height == 0 {
        return;
    }

    let viewport_rows = area.height.saturating_sub(HEADER_ROWS) as usize;
    state.window.viewport_rows = viewport_rows;

    let gutter_digits = digits(state.window.total_rows.max(1));
    let gutter_width = (gutter_digits + 1) as u16;
    let rail_width = if alphabet_rail::should_show_rail(state) {
        alphabet_rail::RAIL_WIDTH
    } else {
        0
    };
    // reserve right-side space for scrollbar and optional alphabet rail
    let data_width = area
        .width
        .saturating_sub(gutter_width)
        .saturating_sub(1 + rail_width);
    if state.avail_col_width != data_width {
        state.recompute_col_widths(data_width);
    }

    let visible_cols = compute_visible_cols(state, data_width);

    let buf = frame.buffer_mut();
    buf.set_style(area, Style::default().bg(theme.bg));

    if area.height >= HEADER_ROWS {
        render_header(
            buf,
            area,
            gutter_width,
            &visible_cols,
            state,
            theme,
            symbols,
        );
    }

    if state.window.total_rows == 0 && insert_row.is_none() && area.height > HEADER_ROWS {
        buf.set_string(
            area.x + gutter_width,
            area.y + HEADER_ROWS,
            "Empty table",
            Style::default().fg(theme.fg_faint).bg(theme.bg),
        );
        return;
    }

    if area.height > HEADER_ROWS {
        render_data_rows(
            buf,
            area,
            gutter_width,
            &visible_cols,
            state,
            insert_row,
            theme,
            symbols,
        );
        render_focused_border(
            buf,
            area,
            gutter_width,
            &visible_cols,
            state,
            insert_row,
            theme,
            symbols,
        );
        render_vertical_scrollbar(buf, area, state, theme, symbols);
        render_loading_indicator(buf, area, state, theme, symbols);
    }
    let _ = buf;
    alphabet_rail::render_rail(frame, area, state, theme);
}

pub fn hit_test(area: Rect, state: &GridState, x: u16, y: u16) -> Option<GridHit> {
    if area.width == 0
        || area.height == 0
        || x < area.x
        || x >= area.x + area.width
        || y < area.y
        || y >= area.y + area.height
    {
        return None;
    }

    let gutter_digits = digits(state.window.total_rows.max(1));
    let gutter_width = (gutter_digits + 1) as u16;
    let rail_width = if alphabet_rail::should_show_rail(state) {
        alphabet_rail::RAIL_WIDTH
    } else {
        0
    };
    let data_width = area
        .width
        .saturating_sub(gutter_width)
        .saturating_sub(1 + rail_width);

    if vertical_scrollbar_metrics(area, state).is_some_and(|metrics| {
        x == metrics.track_x
            && y >= metrics.track_y_start
            && y < metrics.track_y_start + metrics.track_height as u16
    }) {
        return Some(GridHit::Scrollbar);
    }

    if let Some(letter) = alphabet_rail::hit_test(area, state, x, y) {
        return Some(GridHit::AlphabetRail(letter));
    }

    if y < area.y + HEADER_ROWS {
        let col = hit_test_col(area, state, x, gutter_width, data_width)?;
        return Some(GridHit::Header(col));
    }

    if x < area.x + gutter_width {
        let row = hit_test_row(area, state, y)?;
        return Some(GridHit::RowGutter(row));
    }

    let row = hit_test_row(area, state, y)?;
    let col = hit_test_col(area, state, x, gutter_width, data_width)?;
    Some(GridHit::Cell { row, col })
}

pub enum GridHit {
    Header(usize),
    RowGutter(usize),
    Cell { row: usize, col: usize },
    AlphabetRail(char),
    Scrollbar,
}

fn hit_test_row(area: Rect, state: &GridState, y: u16) -> Option<usize> {
    if y < area.y + HEADER_ROWS {
        return None;
    }
    let row_in_view = (y - area.y - HEADER_ROWS) as i64;
    let abs_row = state.viewport_start + row_in_view;
    if abs_row < 0 || abs_row >= state.window.total_rows {
        None
    } else {
        Some(abs_row as usize)
    }
}

fn hit_test_col(
    area: Rect,
    state: &GridState,
    x: u16,
    gutter_width: u16,
    data_width: u16,
) -> Option<usize> {
    let visible_cols = compute_visible_cols(state, data_width);
    let mut col_x = area.x + gutter_width;
    for (col_idx, w) in visible_cols {
        let col_end = col_x.saturating_add(w);
        if x >= col_x && x < col_end {
            return Some(col_idx);
        }
        col_x = col_end;
    }
    None
}

#[cfg(test)]
mod tests {
    use super::{
        compute_visible_cols, enum_value_color, scrollbar_drag_start, scrollbar_drag_target_row,
        GridInit, GridState, RowSelection,
    };
    use crate::db::{schema::Column, types::SqlValue};
    use crate::theme::Theme;
    use ratatui::layout::Rect;

    fn make_col(name: &str, col_type: &str, is_pk: bool) -> Column {
        Column {
            cid: 0,
            name: name.to_string(),
            col_type: col_type.to_string(),
            not_null: false,
            default_value: None,
            is_pk,
        }
    }

    #[test]
    fn enum_values_in_same_column_get_distinct_colors() {
        let theme = Theme::default();
        let enum_values = vec!["COMPLETED".to_string(), "PENDING".to_string()];

        let completed = enum_value_color("COMPLETED", &enum_values, &theme);
        let pending = enum_value_color("PENDING", &enum_values, &theme);

        assert_ne!(completed, pending);
    }

    #[test]
    fn recompute_col_widths_preserves_sampled_widths_even_in_narrow_viewports() {
        let columns = vec![make_col("service", "TEXT", false)];
        let mut grid = GridState::new(GridInit {
            table_name: "payments".to_string(),
            columns,
            fk_cols: vec![false],
            enumerated_values: vec![Vec::new()],
            rows: vec![vec![SqlValue::Text("x".to_string())]],
            width_sample_rows: vec![vec![SqlValue::Text(
                "Microsoft Exchange Online".to_string(),
            )]],
            total_rows: 1,
            area_width: 8,
        });

        let narrow = grid.col_widths[0];
        grid.recompute_col_widths(40);

        assert!(narrow > 10);
        assert_eq!(grid.col_widths[0], narrow);
    }

    #[test]
    fn last_visible_column_expands_to_fill_viewport() {
        let columns = vec![
            make_col("id", "INTEGER", false),
            make_col("name", "TEXT", false),
            make_col("city", "TEXT", false),
        ];
        let mut grid = GridState::new(GridInit {
            table_name: "customers".to_string(),
            columns,
            fk_cols: vec![false, false, false],
            enumerated_values: vec![Vec::new(), Vec::new(), Vec::new()],
            rows: vec![vec![
                SqlValue::Integer(1),
                SqlValue::Text("Alice".to_string()),
                SqlValue::Text("Brussels".to_string()),
            ]],
            width_sample_rows: vec![],
            total_rows: 1,
            area_width: 80,
        });
        grid.col_widths = vec![6, 8, 8];

        let visible = compute_visible_cols(&grid, 20);

        assert_eq!(visible, vec![(0, 6), (1, 14)]);
    }

    #[test]
    fn extra_width_prefers_text_columns_over_numeric_columns() {
        let columns = vec![
            make_col("id", "INTEGER", false),
            make_col("title", "TEXT", false),
            make_col("score", "INTEGER", false),
        ];
        let mut grid = GridState::new(GridInit {
            table_name: "scores".to_string(),
            columns,
            fk_cols: vec![false, false, false],
            enumerated_values: vec![Vec::new(), Vec::new(), Vec::new()],
            rows: vec![vec![
                SqlValue::Integer(1),
                SqlValue::Text("Alice".to_string()),
                SqlValue::Integer(42),
            ]],
            width_sample_rows: vec![],
            total_rows: 1,
            area_width: 28,
        });
        grid.col_widths = vec![6, 8, 6];

        let visible = compute_visible_cols(&grid, 28);

        assert_eq!(visible, vec![(0, 6), (1, 16), (2, 6)]);
    }

    #[test]
    fn scrollbar_drag_maps_pointer_position_to_row() {
        let columns = vec![make_col("name", "TEXT", false)];
        let mut grid = GridState::new(GridInit {
            table_name: "customers".to_string(),
            columns,
            fk_cols: vec![false],
            enumerated_values: vec![Vec::new()],
            rows: vec![vec![SqlValue::Text("Alice".to_string())]; 50],
            width_sample_rows: vec![],
            total_rows: 100,
            area_width: 40,
        });
        grid.window.viewport_rows = 20;
        let area = Rect {
            x: 0,
            y: 0,
            width: 20,
            height: 13,
        };

        let (grab_offset, top_row) = scrollbar_drag_start(area, &grid, 3).expect("thumb drag");
        let lower_row =
            scrollbar_drag_target_row(area, &grid, 10, grab_offset).expect("lower drag target");

        assert_eq!(top_row, 0);
        assert!(lower_row > 0);
        assert!(lower_row < grid.window.total_rows);
    }

    #[test]
    fn scroll_down_starts_moving_viewport_before_bottom_edge() {
        let columns = vec![make_col("name", "TEXT", false)];
        let mut grid = GridState::new(GridInit {
            table_name: "customers".to_string(),
            columns,
            fk_cols: vec![false],
            enumerated_values: vec![Vec::new()],
            rows: vec![vec![SqlValue::Text("Alice".to_string())]; 20],
            width_sample_rows: vec![],
            total_rows: 100,
            area_width: 40,
        });
        grid.window.viewport_rows = 10;

        grid.scroll_down(5);

        assert_eq!(grid.focused_row, 5);
        assert_eq!(grid.viewport_start, 1);
    }

    #[test]
    fn extending_row_selection_tracks_anchor_and_head() {
        let columns = vec![make_col("name", "TEXT", false)];
        let mut grid = GridState::new(GridInit {
            table_name: "customers".to_string(),
            columns,
            fk_cols: vec![false],
            enumerated_values: vec![Vec::new()],
            rows: vec![vec![SqlValue::Text("Alice".to_string())]; 10],
            width_sample_rows: vec![],
            total_rows: 10,
            area_width: 40,
        });

        grid.extend_row_selection_down(2);

        assert_eq!(grid.focused_row, 2);
        assert_eq!(
            grid.row_selection,
            RowSelection::Range { anchor: 0, head: 2 }
        );
        assert!(grid.is_row_selected(1));
        assert!(!grid.is_row_selected(3));
    }

    #[test]
    fn focus_cell_clears_row_selection() {
        let columns = vec![make_col("name", "TEXT", false)];
        let mut grid = GridState::new(GridInit {
            table_name: "customers".to_string(),
            columns,
            fk_cols: vec![false],
            enumerated_values: vec![Vec::new()],
            rows: vec![vec![SqlValue::Text("Alice".to_string())]; 10],
            width_sample_rows: vec![],
            total_rows: 10,
            area_width: 40,
        });
        grid.select_all_rows();

        grid.focus_cell(4, 0);

        assert_eq!(grid.row_selection, RowSelection::None);
        assert_eq!(grid.focused_row, 4);
    }
}
