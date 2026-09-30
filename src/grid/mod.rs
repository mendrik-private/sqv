pub mod alphabet_rail;
pub mod layout;
pub mod virtual_scroll;

use std::collections::{BTreeSet, HashMap};

use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::{Color, Modifier, Style},
    Frame,
};

use crate::{
    db::{
        query::{OrderBy, ViewQuery},
        schema::Column,
        types::{ColumnKind, SqlValue},
    },
    filter::predicate::filter_to_sql,
    symbols::Symbols,
    theme::Theme,
    ui::{
        popup::InsertRowState,
        widgets::{
            cell::{cell_text, fit_cell, Align},
            scrollbar::Scrollbar,
            text::{put, text_width, truncate_with_ellipsis},
        },
    },
    view_settings::{SortKey, ViewSettings},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SortDir {
    Asc,
    Desc,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SortSpec {
    pub col_idx: usize,
    pub direction: SortDir,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RowSelection {
    None,
    Rows(BTreeSet<usize>),
    /// Every row except the listed ones, so deselecting a few rows of a large
    /// table stays proportional to the exceptions.
    All {
        except: BTreeSet<usize>,
    },
}

impl RowSelection {
    pub fn all() -> Self {
        RowSelection::All {
            except: BTreeSet::new(),
        }
    }
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

pub struct GridState {
    pub table_name: String,
    pub columns: Vec<Column>,
    /// Presentation kind of each column, derived once from its declared type.
    pub kinds: Vec<ColumnKind>,
    pub window: virtual_scroll::VirtualWindow,
    /// A stable sample of rows that column widths are measured on.
    pub width_sample_rows: Vec<Vec<SqlValue>>,
    pub focused_row: usize,
    pub focused_col: usize,
    pub col_widths: Vec<u16>,
    /// How wide each column would be to show its sampled values in full.
    desired_widths: Vec<u16>,
    widths_dirty: bool,
    /// First scrollable entry of the visible-column order.
    pub h_scroll: usize,
    pub fk_cols: Vec<bool>,
    pub enumerated_values: Vec<Vec<String>>,
    enum_slots: Vec<HashMap<String, usize>>,
    pub needs_fetch: bool,
    /// Whether `window.total_rows` is current; a changed view needs a new count.
    pub count_known: bool,
    pub viewport_start: i64,
    pub avail_col_width: u16,
    /// Sort keys in priority order.
    pub sort: Vec<SortSpec>,
    pub filter: crate::filter::FilterSet,
    pub hidden: BTreeSet<usize>,
    pub width_overrides: HashMap<usize, u16>,
    /// Keeps the first visible column in place while scrolling sideways.
    pub frozen: bool,
    /// Views and tables without a safe rowid cannot be edited.
    pub readonly: bool,
    /// Why the last read failed; shown instead of the rows until one succeeds.
    pub load_error: Option<String>,
    pub row_selection: RowSelection,
    row_selection_anchor: Option<usize>,
    row_selection_base: Option<BTreeSet<usize>>,
}

/// Header text, badge and divider rows above the data.
const HEADER_ROWS: u16 = 3;
const VIEWPORT_SCROLL_MARGIN_ROWS: usize = 5;
const MIN_OVERRIDE_WIDTH: u16 = 4;

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
        let mut state = Self {
            table_name,
            kinds: Vec::new(),
            columns: Vec::new(),
            window: virtual_scroll::VirtualWindow::new(0, rows, total_rows),
            width_sample_rows,
            focused_row: 0,
            focused_col: 0,
            col_widths: Vec::new(),
            desired_widths: Vec::new(),
            widths_dirty: true,
            h_scroll: 0,
            fk_cols: Vec::new(),
            enumerated_values: Vec::new(),
            enum_slots: Vec::new(),
            needs_fetch: false,
            count_known: true,
            viewport_start: 0,
            avail_col_width: area_width,
            sort: Vec::new(),
            filter: crate::filter::FilterSet::default(),
            hidden: BTreeSet::new(),
            width_overrides: HashMap::new(),
            frozen: false,
            readonly: false,
            load_error: None,
            row_selection: RowSelection::None,
            row_selection_anchor: None,
            row_selection_base: None,
        };
        state.install_columns(columns, fk_cols);
        state.set_enum_values(if enumerated_values.len() == col_count {
            enumerated_values
        } else {
            vec![Vec::new(); col_count]
        });
        state
    }

    fn install_columns(&mut self, columns: Vec<Column>, fk_cols: Vec<bool>) {
        self.kinds = columns
            .iter()
            .map(|column| ColumnKind::of(&column.col_type, &column.name))
            .collect();
        self.fk_cols = if fk_cols.len() == columns.len() {
            fk_cols
        } else {
            vec![false; columns.len()]
        };
        self.columns = columns;
        self.widths_dirty = true;
    }

    /// Replaces the columns after a schema change, keeping sort keys, hidden
    /// columns, width overrides, enum sets and width samples of columns that
    /// still exist.
    pub fn set_columns(&mut self, columns: Vec<Column>, fk_cols: Vec<bool>) {
        let old_names: Vec<String> = self.columns.iter().map(|c| c.name.clone()).collect();
        let remap = |old: usize| -> Option<usize> {
            let name = old_names.get(old)?;
            columns.iter().position(|column| &column.name == name)
        };
        self.sort = self
            .sort
            .iter()
            .filter_map(|spec| {
                Some(SortSpec {
                    col_idx: remap(spec.col_idx)?,
                    direction: spec.direction,
                })
            })
            .collect();
        self.hidden = self.hidden.iter().filter_map(|&c| remap(c)).collect();
        self.width_overrides = self
            .width_overrides
            .iter()
            .filter_map(|(&c, &w)| Some((remap(c)?, w)))
            .collect();
        let enum_values = columns
            .iter()
            .map(|column| {
                old_names
                    .iter()
                    .position(|name| name == &column.name)
                    .and_then(|old| self.enumerated_values.get(old).cloned())
                    .unwrap_or_default()
            })
            .collect();
        self.width_sample_rows = self
            .width_sample_rows
            .iter()
            .map(|row| {
                columns
                    .iter()
                    .map(|column| {
                        old_names
                            .iter()
                            .position(|name| name == &column.name)
                            .and_then(|old| row.get(old).cloned())
                            .unwrap_or(SqlValue::Null)
                    })
                    .collect()
            })
            .collect();
        let focused_name = old_names.get(self.focused_col).cloned();
        self.install_columns(columns, fk_cols);
        self.set_enum_values(enum_values);
        self.focused_col = focused_name
            .and_then(|name| self.columns.iter().position(|c| c.name == name))
            .unwrap_or(0);
        self.h_scroll = 0;
        let filter_columns: Vec<String> = self.columns.iter().map(|c| c.name.clone()).collect();
        self.filter
            .columns
            .retain(|name, _| filter_columns.contains(name));
    }

    /// Rows to measure column widths on, taken once from the first loaded window.
    pub fn set_width_sample(&mut self, rows: Vec<Vec<SqlValue>>) {
        self.width_sample_rows = rows;
        self.widths_dirty = true;
    }

    /// Enum value sets per column; each value gets a stable colour slot.
    pub fn set_enum_values(&mut self, values: Vec<Vec<String>>) {
        self.enum_slots = values.iter().map(|set| enum_slots(set)).collect();
        self.enumerated_values = values;
    }

    /// The persistent part of the view, keyed by column name.
    pub fn settings(&self) -> ViewSettings {
        let name = |col: usize| self.columns[col].name.clone();
        ViewSettings {
            filter: self.filter.clone(),
            sort: self
                .sort
                .iter()
                .map(|spec| SortKey {
                    column: name(spec.col_idx),
                    ascending: spec.direction == SortDir::Asc,
                })
                .collect(),
            hidden: self.hidden.iter().map(|&c| name(c)).collect(),
            widths: self
                .width_overrides
                .iter()
                .map(|(&c, &w)| (name(c), w))
                .collect(),
            frozen: self.frozen,
        }
    }

    pub fn apply_settings(&mut self, settings: ViewSettings) {
        let index = |name: &str| self.columns.iter().position(|c| c.name == name);
        self.sort = settings
            .sort
            .iter()
            .filter_map(|key| {
                Some(SortSpec {
                    col_idx: index(&key.column)?,
                    direction: if key.ascending {
                        SortDir::Asc
                    } else {
                        SortDir::Desc
                    },
                })
            })
            .collect();
        self.hidden = settings
            .hidden
            .iter()
            .filter_map(|name| index(name))
            .collect();
        if self.hidden.len() >= self.columns.len() {
            self.hidden.clear();
        }
        self.width_overrides = settings
            .widths
            .iter()
            .filter_map(|(name, &w)| Some((index(name)?, w)))
            .collect();
        let names: Vec<String> = self.columns.iter().map(|c| c.name.clone()).collect();
        self.filter = settings.filter;
        self.filter.columns.retain(|name, _| names.contains(name));
        self.frozen = settings.frozen;
        self.widths_dirty = true;
        if self.hidden.contains(&self.focused_col) {
            self.focused_col = self.display_columns().first().copied().unwrap_or(0);
        }
    }

    /// Column indexes in display order, without hidden columns.
    pub fn display_columns(&self) -> Vec<usize> {
        (0..self.columns.len())
            .filter(|col| !self.hidden.contains(col))
            .collect()
    }

    fn header_meta(&self, col: usize, symbols: &Symbols) -> String {
        let mut meta = format!(" {}", self.kinds[col].badge());
        if self.columns[col].is_pk {
            meta.push(' ');
            meta.push_str(&symbols.pk_icon);
        }
        if self.fk_cols.get(col).copied().unwrap_or(false) {
            meta.push(' ');
            meta.push_str(&symbols.fk_icon);
        }
        if self.column_filtered(col) {
            meta.push(' ');
            meta.push_str(&symbols.filter_marker);
        }
        meta
    }

    fn column_filtered(&self, col: usize) -> bool {
        self.filter
            .columns
            .get(&self.columns[col].name)
            .is_some_and(|cf| cf.rules.iter().any(|r| r.enabled))
    }

    /// The sort marker of a column: an arrow, plus its priority when several
    /// columns are sorted.
    fn sort_marker(&self, col: usize, symbols: &Symbols) -> Option<String> {
        let position = self.sort.iter().position(|spec| spec.col_idx == col)?;
        let arrow = match self.sort[position].direction {
            SortDir::Asc => symbols.sort_asc,
            SortDir::Desc => symbols.sort_desc,
        };
        Some(if self.sort.len() > 1 {
            format!("{arrow}{}", position + 1)
        } else {
            arrow.to_string()
        })
    }

    pub fn recompute_col_widths(&mut self, symbols: &Symbols) {
        let sizing_rows = if self.width_sample_rows.is_empty() {
            &self.window.rows
        } else {
            &self.width_sample_rows
        };
        let metas: Vec<String> = (0..self.columns.len())
            .map(|col| self.header_meta(col, symbols))
            .collect();
        let markers: Vec<usize> = (0..self.columns.len())
            .map(|col| {
                // Room for a marker keeps widths stable when sorting changes.
                self.sort_marker(col, symbols)
                    .map_or(2, |m| text_width(&m) + 1)
            })
            .collect();
        let headers: Vec<layout::HeaderNeeds> = self
            .columns
            .iter()
            .enumerate()
            .map(|(col, column)| layout::HeaderNeeds {
                name: &column.name,
                sort_marker_width: markers[col],
                meta: &metas[col],
            })
            .collect();
        let measured = layout::compute_col_widths(&self.kinds, &headers, sizing_rows, symbols);
        self.col_widths = measured
            .widths
            .iter()
            .enumerate()
            .map(|(col, &width)| self.width_overrides.get(&col).copied().unwrap_or(width))
            .collect();
        self.desired_widths = measured.desired;
        self.widths_dirty = false;
        self.adjust_h_scroll();
    }

    /// Narrows or widens a column by `delta` cells; the override is saved with
    /// the view.
    pub fn adjust_column_width(&mut self, col: usize, delta: i16) {
        if col >= self.columns.len() {
            return;
        }
        let current = self
            .col_widths
            .get(col)
            .or_else(|| self.width_overrides.get(&col))
            .copied()
            .unwrap_or(layout::CELL_PADDING + 8);
        let width = (current as i32 + delta as i32).clamp(MIN_OVERRIDE_WIDTH as i32, 200) as u16;
        self.width_overrides.insert(col, width);
        if let Some(slot) = self.col_widths.get_mut(col) {
            *slot = width;
        }
        self.adjust_h_scroll();
    }

    /// Hides a column unless it is the last visible one.
    pub fn hide_column(&mut self, col: usize) -> bool {
        if self.display_columns().len() <= 1 {
            return false;
        }
        self.hidden.insert(col);
        let visible = self.display_columns();
        self.focused_col = visible
            .iter()
            .copied()
            .find(|&c| c > col)
            .or_else(|| visible.last().copied())
            .unwrap_or(0);
        self.h_scroll = self.h_scroll.min(visible.len().saturating_sub(1));
        self.adjust_h_scroll();
        true
    }

    pub fn show_all_columns(&mut self) {
        self.hidden.clear();
    }

    pub fn toggle_frozen(&mut self) {
        self.frozen = !self.frozen;
        self.adjust_h_scroll();
    }

    /// Cycles the focused column through ascending, descending and unsorted,
    /// making it the only sort key.
    pub fn cycle_sort(&mut self, col: usize) {
        let current = self
            .sort
            .iter()
            .find(|spec| spec.col_idx == col)
            .map(|s| s.direction);
        self.sort = match current {
            None => vec![SortSpec {
                col_idx: col,
                direction: SortDir::Asc,
            }],
            Some(SortDir::Asc) => vec![SortSpec {
                col_idx: col,
                direction: SortDir::Desc,
            }],
            Some(SortDir::Desc) => Vec::new(),
        };
        self.widths_dirty = true;
    }

    /// Adds the column as a further sort key, or cycles it within the keys.
    pub fn add_sort_key(&mut self, col: usize) {
        match self.sort.iter().position(|spec| spec.col_idx == col) {
            None => self.sort.push(SortSpec {
                col_idx: col,
                direction: SortDir::Asc,
            }),
            Some(index) if self.sort[index].direction == SortDir::Asc => {
                self.sort[index].direction = SortDir::Desc;
            }
            Some(index) => {
                self.sort.remove(index);
            }
        }
        self.widths_dirty = true;
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

    /// Scrolls so the viewport starts at `offset`, keeping the focus inside it.
    pub fn scroll_viewport_to(&mut self, offset: i64) {
        let vp = self.window.viewport_rows.max(1) as i64;
        let max_start = (self.window.total_rows - vp).max(0);
        self.viewport_start = offset.clamp(0, max_start);
        let last_visible = (self.viewport_start + vp - 1)
            .min(self.window.total_rows - 1)
            .max(0);
        self.focused_row =
            (self.focused_row as i64).clamp(self.viewport_start, last_visible) as usize;
        self.check_needs_fetch();
    }

    /// Records the rendered viewport height and re-establishes the scroll
    /// invariants, so a taller terminal loads the rows it newly shows.
    pub fn set_viewport_rows(&mut self, rows: usize) {
        if self.window.viewport_rows == rows {
            return;
        }
        self.window.viewport_rows = rows;
        self.adjust_viewport();
        self.check_needs_fetch();
    }

    fn adjust_viewport(&mut self) {
        let vp = self.window.viewport_rows.max(1);
        let fr = self.focused_row as i64;
        let total = self.window.total_rows;
        let margin = VIEWPORT_SCROLL_MARGIN_ROWS.min(vp.saturating_sub(1) / 2) as i64;

        if fr < self.viewport_start + margin {
            self.viewport_start = fr - margin;
        } else if fr >= self.viewport_start + vp as i64 - margin {
            self.viewport_start = fr - vp as i64 + margin + 1;
        }

        self.viewport_start = self.viewport_start.max(0);
        let max_start = (total - vp as i64).max(0);
        self.viewport_start = self.viewport_start.min(max_start);
    }

    /// Requests a fetch when the loaded window does not cover what is shown.
    pub(crate) fn check_needs_fetch(&mut self) {
        let vp = self.window.viewport_rows as i64;
        let window_end = self.window.offset + self.window.rows.len() as i64;
        let shown_end = (self.viewport_start + vp).min(self.window.total_rows);
        let uncovered = self.viewport_start < self.window.offset || shown_end > window_end;
        if uncovered || self.window.needs_prefetch(self.focused_row as i64) {
            self.needs_fetch = true;
        }
    }

    fn step_column(&mut self, forward: bool) {
        let order = self.display_columns();
        let Some(position) = order.iter().position(|&c| c == self.focused_col) else {
            self.focused_col = order.first().copied().unwrap_or(0);
            return;
        };
        let next = if forward {
            order.get(position + 1)
        } else {
            position.checked_sub(1).and_then(|p| order.get(p))
        };
        if let Some(&col) = next {
            self.focused_col = col;
            self.adjust_h_scroll();
        }
    }

    pub fn move_col_right(&mut self) {
        self.step_column(true);
    }

    pub fn move_col_left(&mut self) {
        self.step_column(false);
    }

    pub fn move_col_first(&mut self) {
        self.focused_col = self.display_columns().first().copied().unwrap_or(0);
        self.h_scroll = 0;
    }

    pub fn move_col_last(&mut self) {
        if let Some(&last) = self.display_columns().last() {
            self.focused_col = last;
            self.adjust_h_scroll();
        }
    }

    /// Scrolls columns sideways without moving the focus (mouse Shift-wheel).
    pub fn scroll_columns(&mut self, right: bool) {
        let count = self.display_columns().len();
        if right {
            self.h_scroll = (self.h_scroll + 1).min(count.saturating_sub(1));
        } else {
            self.h_scroll = self.h_scroll.saturating_sub(1);
        }
    }

    pub fn clear_row_selection(&mut self) {
        self.row_selection = RowSelection::None;
        self.row_selection_anchor = None;
        self.row_selection_base = None;
    }

    pub fn commit_row_selection(&mut self) {
        self.row_selection_anchor = None;
        self.row_selection_base = None;
        if matches!(&self.row_selection, RowSelection::Rows(rows) if rows.is_empty()) {
            self.row_selection = RowSelection::None;
        }
    }

    /// Selected rows that still exist; the table may have shrunk since selecting.
    fn existing_selected_rows(
        rows: &BTreeSet<usize>,
        total_rows: usize,
    ) -> impl Iterator<Item = usize> + '_ {
        rows.range(..total_rows).copied()
    }

    pub fn selected_rows(&self) -> Vec<usize> {
        let total_rows = self.window.total_rows.max(0) as usize;
        match &self.row_selection {
            RowSelection::None => Vec::new(),
            RowSelection::Rows(rows) => Self::existing_selected_rows(rows, total_rows).collect(),
            RowSelection::All { except } => (0..total_rows)
                .filter(|row| !except.contains(row))
                .collect(),
        }
    }

    pub fn selected_row_count(&self) -> usize {
        let total_rows = self.window.total_rows.max(0) as usize;
        match &self.row_selection {
            RowSelection::None => 0,
            RowSelection::Rows(rows) => Self::existing_selected_rows(rows, total_rows).count(),
            RowSelection::All { except } => {
                total_rows - Self::existing_selected_rows(except, total_rows).count()
            }
        }
    }

    pub fn select_only_row(&mut self, row: usize) {
        self.clear_row_selection();
        if row < self.window.total_rows.max(0) as usize {
            self.row_selection = RowSelection::Rows(BTreeSet::from([row]));
        }
    }

    pub fn toggle_row_selected(&mut self, row: usize) {
        self.commit_row_selection();
        let total_rows = self.window.total_rows.max(0) as usize;
        if row >= total_rows {
            return;
        }

        match &mut self.row_selection {
            RowSelection::None => {
                self.row_selection = RowSelection::Rows(BTreeSet::from([row]));
            }
            RowSelection::Rows(rows) | RowSelection::All { except: rows } => {
                if !rows.remove(&row) {
                    rows.insert(row);
                }
            }
        }
        if !self.has_row_selection() {
            self.row_selection = RowSelection::None;
        }
    }

    pub fn extend_row_selection_down(&mut self, n: usize) {
        if self.window.total_rows <= 0 {
            return;
        }
        if matches!(&self.row_selection, RowSelection::All { .. }) {
            self.scroll_down(n);
            return;
        }
        self.ensure_shift_selection_started();
        self.scroll_down(n);
        self.refresh_shift_selection();
    }

    pub fn extend_row_selection_up(&mut self, n: usize) {
        if self.window.total_rows <= 0 {
            return;
        }
        if matches!(&self.row_selection, RowSelection::All { .. }) {
            self.scroll_up(n);
            return;
        }
        self.ensure_shift_selection_started();
        self.scroll_up(n);
        self.refresh_shift_selection();
    }

    pub fn select_all_rows(&mut self) {
        self.row_selection = if self.window.total_rows > 0 {
            RowSelection::all()
        } else {
            RowSelection::None
        };
    }

    /// Cheap enough for the renderer, which asks for every visible row: O(log n)
    /// for explicit rows, O(exceptions) for an all-rows selection.
    pub fn has_row_selection(&self) -> bool {
        let total_rows = self.window.total_rows.max(0) as usize;
        match &self.row_selection {
            RowSelection::None => false,
            RowSelection::Rows(rows) => Self::existing_selected_rows(rows, total_rows)
                .next()
                .is_some(),
            RowSelection::All { except } => {
                Self::existing_selected_rows(except, total_rows).count() < total_rows
            }
        }
    }

    pub fn is_row_selected(&self, abs_row: i64) -> bool {
        if abs_row < 0 {
            return false;
        }
        let abs_row = abs_row as usize;
        match &self.row_selection {
            RowSelection::None => false,
            RowSelection::Rows(rows) => rows.contains(&abs_row),
            RowSelection::All { except } => {
                abs_row < self.window.total_rows.max(0) as usize && !except.contains(&abs_row)
            }
        }
    }

    pub fn focus_cell(&mut self, row: usize, col: usize) {
        self.clear_row_selection();
        self.focus_cell_preserve_selection(row, col);
    }

    pub fn focus_cell_preserve_selection(&mut self, row: usize, col: usize) {
        self.focused_row = row.min(self.window.total_rows.saturating_sub(1).max(0) as usize);
        if self.columns.is_empty() {
            self.focused_col = 0;
            return;
        }
        self.focused_col = col.min(self.columns.len() - 1);
        self.adjust_viewport();
        self.adjust_h_scroll();
        self.check_needs_fetch();
    }

    pub fn order_by(&self) -> Vec<OrderBy> {
        self.sort
            .iter()
            .filter_map(|spec| {
                Some(OrderBy {
                    column: self.columns.get(spec.col_idx)?.name.clone(),
                    ascending: spec.direction == SortDir::Asc,
                })
            })
            .collect()
    }

    /// The table as currently sorted and filtered.
    pub fn view_query(&self) -> anyhow::Result<ViewQuery> {
        let (where_clause, where_params) = filter_to_sql(&self.filter)?;
        Ok(ViewQuery {
            table: self.table_name.clone(),
            order_by: self.order_by(),
            where_clause,
            where_params,
        })
    }

    /// Whether the primary sort key is free text, which letter jumps address.
    pub fn is_text_sorted(&self) -> bool {
        self.sort
            .first()
            .and_then(|spec| self.kinds.get(spec.col_idx))
            .is_some_and(|kind| kind.is_textual())
    }

    /// Drops the cached window after a write so the next tick refetches it and
    /// recounts the rows.
    pub fn invalidate_window(&mut self) {
        self.window.rows.clear();
        self.window.rowids.clear();
        self.window.fetch_in_flight = false;
        self.needs_fetch = true;
        self.count_known = false;
    }

    /// Moves to the first row and drops the cached window, for when the sort or
    /// filter changed and old offsets no longer mean anything.
    pub fn reset_to_top(&mut self) {
        self.viewport_start = 0;
        self.focused_row = 0;
        self.window.rows.clear();
        self.window.rowids.clear();
        self.window.offset = 0;
        self.count_known = false;
    }

    /// Re-establishes the focus and viewport invariants after `total_rows` changed.
    pub fn clamp_to_total_rows(&mut self) {
        let total_rows = self.window.total_rows;
        self.focused_row = self.focused_row.min((total_rows - 1).max(0) as usize);
        let max_start = (total_rows - self.window.viewport_rows as i64).max(0);
        self.viewport_start = self.viewport_start.min(max_start);
    }

    /// Accounts for `count` rows deleted from this table.
    pub fn rows_removed(&mut self, count: usize) {
        self.clear_row_selection();
        self.window.total_rows = self.window.total_rows.saturating_sub(count as i64).max(0);
        self.clamp_to_total_rows();
        self.invalidate_window();
    }

    fn ensure_shift_selection_started(&mut self) {
        if self.row_selection_anchor.is_some() {
            return;
        }

        self.row_selection_anchor = Some(self.focused_row);
        self.row_selection_base = Some(match &self.row_selection {
            RowSelection::None | RowSelection::All { .. } => BTreeSet::new(),
            RowSelection::Rows(rows) => rows.clone(),
        });
    }

    fn refresh_shift_selection(&mut self) {
        let Some(anchor) = self.row_selection_anchor else {
            return;
        };

        let mut rows = self.row_selection_base.clone().unwrap_or_default();
        match self.focused_row.cmp(&anchor) {
            std::cmp::Ordering::Greater => rows.extend(anchor..self.focused_row),
            std::cmp::Ordering::Less => rows.extend((self.focused_row + 1)..=anchor),
            std::cmp::Ordering::Equal => {}
        }

        self.row_selection = if rows.is_empty() {
            RowSelection::None
        } else {
            RowSelection::Rows(rows)
        };
    }

    /// Scrolls sideways just enough to show the focused column.
    fn adjust_h_scroll(&mut self) {
        let order = self.display_columns();
        let Some(position) = order.iter().position(|&c| c == self.focused_col) else {
            return;
        };
        let pinned = usize::from(self.frozen && !order.is_empty());
        if position < pinned {
            return;
        }
        let scroll_min = pinned;
        self.h_scroll = self
            .h_scroll
            .clamp(scroll_min, order.len().saturating_sub(1).max(scroll_min));
        if position < self.h_scroll {
            self.h_scroll = position;
            return;
        }
        let avail = self.avail_col_width as usize;
        if avail == 0 || self.col_widths.len() != self.columns.len() {
            return;
        }
        let width_of = |c: usize| self.col_widths[c] as usize;
        let pinned_width: usize = order[..pinned].iter().map(|&c| width_of(c)).sum();
        loop {
            let used: usize = pinned_width
                + order[self.h_scroll..=position]
                    .iter()
                    .map(|&c| width_of(c))
                    .sum::<usize>();
            if used <= avail || self.h_scroll >= position {
                break;
            }
            self.h_scroll += 1;
        }
    }
}

// ── enum colours ─────────────────────────────────────────────────────────────

fn fnv1a(text: &str) -> u64 {
    text.bytes().fold(0xcbf29ce484222325, |hash, byte| {
        (hash ^ u64::from(byte)).wrapping_mul(0x100000001b3)
    })
}

const ENUM_PALETTE_SIZE: usize = 10;

/// A colour slot per value: its hash decides, and collisions probe to the next
/// free slot in hash order, so colours stay the same across sessions and sorts
/// and the values of one column never share a colour.
fn enum_slots(values: &[String]) -> HashMap<String, usize> {
    let mut by_hash: Vec<&String> = values.iter().collect();
    by_hash.sort_by_key(|value| (fnv1a(value), (*value).clone()));
    let mut taken = [false; ENUM_PALETTE_SIZE];
    let mut slots = HashMap::new();
    for value in by_hash {
        let mut slot = (fnv1a(value) % ENUM_PALETTE_SIZE as u64) as usize;
        if taken.iter().all(|t| *t) {
            taken = [false; ENUM_PALETTE_SIZE];
        }
        while taken[slot] {
            slot = (slot + 1) % ENUM_PALETTE_SIZE;
        }
        taken[slot] = true;
        slots.insert(value.clone(), slot);
    }
    slots
}

/// Hues for enum values. Accent (focus), red (errors) and the pure type colours
/// are left out so an enum value never reads as focus, error or data type.
fn enum_palette(theme: &Theme) -> [Color; ENUM_PALETTE_SIZE] {
    [
        theme.teal,
        theme.green,
        theme.yellow,
        mix_color(theme.teal, theme.blue, 0.5),
        mix_color(theme.green, theme.yellow, 0.5),
        mix_color(theme.teal, theme.fg, 0.4),
        mix_color(theme.yellow, theme.fg, 0.4),
        mix_color(theme.green, theme.fg, 0.45),
        mix_color(theme.teal, theme.purple, 0.45),
        mix_color(theme.yellow, theme.pink, 0.45),
    ]
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
        Color::White => (255, 255, 255),
        Color::Gray => (128, 128, 128),
        Color::DarkGray => (64, 64, 64),
        _ => (0, 0, 0),
    }
}

// ── cell styling ─────────────────────────────────────────────────────────────

fn badge_color(kind: ColumnKind, theme: &Theme) -> Color {
    match kind {
        ColumnKind::Date | ColumnKind::Datetime | ColumnKind::EpochDatetime => theme.pink,
        ColumnKind::Integer | ColumnKind::Numeric { .. } | ColumnKind::Boolean => theme.yellow,
        ColumnKind::Real { .. } => theme.blue,
        ColumnKind::Text | ColumnKind::Untyped => theme.teal,
        ColumnKind::Blob => theme.purple,
    }
}

fn cell_style(state: &GridState, col: usize, value: &SqlValue, theme: &Theme) -> Style {
    let kind = state.kinds[col];
    match value {
        SqlValue::Null => Style::default()
            .fg(theme.fg_faint)
            .add_modifier(Modifier::ITALIC),
        SqlValue::Blob(_) => Style::default()
            .fg(theme.purple)
            .add_modifier(Modifier::ITALIC),
        SqlValue::Integer(n) if kind == ColumnKind::Boolean => {
            Style::default().fg(if *n != 0 { theme.green } else { theme.fg_faint })
        }
        _ if kind.is_temporal() => Style::default().fg(theme.pink),
        SqlValue::Integer(_) | SqlValue::Real(_) => Style::default().fg(theme.blue),
        SqlValue::Text(text) => match state.enum_slots.get(col).and_then(|slots| slots.get(text)) {
            Some(&slot) => Style::default().fg(enum_palette(theme)[slot]),
            None => Style::default().fg(theme.fg_dim),
        },
    }
}

fn show_cell_focus(state: &GridState, insert_row: Option<&InsertRowState>) -> bool {
    insert_row.is_some() || !state.has_row_selection()
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

fn focused_cell_background(row_bg: Color, theme: &Theme) -> Color {
    mix_color(row_bg, theme.accent, 0.16)
}

fn insert_row_background(theme: &Theme) -> Color {
    mix_color(theme.bg_raised, theme.accent, 0.08)
}

// ── layout ───────────────────────────────────────────────────────────────────

/// Horizontal geometry of the grid inside its panel.
#[derive(Debug, Clone, Copy)]
struct GridGeometry {
    gutter_width: u16,
    gutter_digits: usize,
    data_width: u16,
}

fn digits(n: i64) -> usize {
    if n == 0 {
        1
    } else {
        n.unsigned_abs().to_string().len()
    }
}

fn geometry(area: Rect, state: &GridState) -> GridGeometry {
    let gutter_digits = digits(state.window.total_rows.max(1));
    let gutter_width = (gutter_digits + 1) as u16;
    let rail_width = if alphabet_rail::should_show_rail(state) {
        alphabet_rail::RAIL_WIDTH
    } else {
        0
    };
    // Right side: one cell for the "more columns" marker, one for the scrollbar.
    let data_width = area
        .width
        .saturating_sub(gutter_width)
        .saturating_sub(2 + rail_width);
    GridGeometry {
        gutter_width,
        gutter_digits,
        data_width,
    }
}

/// The columns shown with their widths: the frozen column first when set, then
/// columns from the horizontal scroll position. Spare width goes to columns
/// whose values do not fit, in proportion to what they lack.
fn compute_visible_cols(state: &GridState, data_width: u16) -> Vec<(usize, u16)> {
    let order = state.display_columns();
    if state.col_widths.len() != state.columns.len() {
        return Vec::new();
    }
    let pinned = usize::from(state.frozen && !order.is_empty());
    let scroll_start = state.h_scroll.max(pinned);
    let candidates = order[..pinned]
        .iter()
        .chain(order.get(scroll_start..).unwrap_or(&[]));
    let mut visible: Vec<(usize, u16)> = Vec::new();
    let mut used = 0u16;
    for &col in candidates {
        let width = state.col_widths[col];
        if visible.is_empty() || used + width <= data_width {
            visible.push((col, width.min(data_width.max(1))));
            used = used.saturating_add(width);
        } else {
            break;
        }
    }
    let mut spare = data_width.saturating_sub(used);
    if spare == 0 {
        return visible;
    }
    let lacking: Vec<u16> = visible
        .iter()
        .map(|&(col, width)| {
            if state.width_overrides.contains_key(&col) {
                0
            } else {
                state
                    .desired_widths
                    .get(col)
                    .copied()
                    .unwrap_or(width)
                    .saturating_sub(width)
            }
        })
        .collect();
    let total_lacking: u32 = lacking.iter().map(|&l| l as u32).sum();
    if total_lacking > 0 {
        let budget = spare.min(total_lacking.min(u16::MAX as u32) as u16);
        let mut given = 0u16;
        for (index, lack) in lacking.iter().enumerate() {
            let share = ((*lack as u32 * budget as u32) / total_lacking) as u16;
            visible[index].1 += share;
            given += share;
        }
        spare -= given;
    }
    // Whatever is left widens text columns, never numbers.
    let text: Vec<usize> = visible
        .iter()
        .enumerate()
        .filter(|(_, (col, _))| state.kinds[*col].is_textual())
        .map(|(index, _)| index)
        .collect();
    if !text.is_empty() && spare > 0 {
        let base = spare / text.len() as u16;
        let remainder = spare % text.len() as u16;
        for (i, &index) in text.iter().enumerate() {
            visible[index].1 += base + u16::from((i as u16) < remainder);
        }
    }
    visible
}

/// Per-frame layout and styling shared by the grid's render passes.
#[derive(Clone, Copy)]
struct GridFrame<'a> {
    area: Rect,
    geometry: GridGeometry,
    visible_cols: &'a [(usize, u16)],
    state: &'a GridState,
    insert_row: Option<&'a InsertRowState>,
    theme: &'a Theme,
    symbols: &'a Symbols,
}

fn render_header(buf: &mut Buffer, frame: &GridFrame) {
    let GridFrame {
        area,
        geometry,
        visible_cols,
        state,
        theme,
        symbols,
        ..
    } = *frame;
    let bg = theme.bg_raised;
    buf.set_style(
        Rect {
            height: HEADER_ROWS.min(area.height),
            ..area
        },
        Style::default().bg(bg),
    );
    let line_style = Style::default().fg(theme.line).bg(bg);
    let right = area.x + geometry.gutter_width + geometry.data_width;

    let mut col_x = area.x + geometry.gutter_width;
    for (position, &(col, width)) in visible_cols.iter().enumerate() {
        if col_x >= right {
            break;
        }
        let width = width.min(right - col_x);
        let end = col_x + width;
        if position > 0 {
            for row in 0..2 {
                put(
                    buf,
                    col_x,
                    area.y + row,
                    end,
                    &symbols.box_vertical.to_string(),
                    line_style,
                );
            }
        }
        let marker = state.sort_marker(col, symbols);
        let marker_width = marker.as_deref().map_or(0, text_width);
        let name_room =
            (width as usize).saturating_sub(2 + marker_width + usize::from(marker.is_some()));
        let name = truncate_with_ellipsis(&state.columns[col].name, name_room, symbols.ellipsis);
        let name_style = Style::default()
            .fg(if col == state.focused_col {
                theme.accent
            } else {
                theme.fg
            })
            .bg(bg)
            .add_modifier(Modifier::BOLD);
        put(buf, col_x + 1, area.y, end, &name, name_style);
        if let Some(marker) = marker {
            let marker_x = end.saturating_sub(1 + marker_width as u16);
            put(
                buf,
                marker_x,
                area.y,
                end,
                &marker,
                Style::default().fg(theme.accent).bg(bg),
            );
        }
        let meta = truncate_with_ellipsis(
            &state.header_meta(col, symbols),
            width as usize,
            symbols.ellipsis,
        );
        put(
            buf,
            col_x,
            area.y + 1,
            end,
            &meta,
            Style::default()
                .fg(badge_color(state.kinds[col], theme))
                .bg(bg)
                .add_modifier(Modifier::DIM),
        );
        col_x = end;
    }

    if HEADER_ROWS <= area.height {
        let divider: String = symbols
            .box_horizontal
            .to_string()
            .repeat(area.width as usize);
        put(
            buf,
            area.x,
            area.y + HEADER_ROWS - 1,
            area.right(),
            &divider,
            line_style,
        );
    }
    let order = state.display_columns();
    let marker_style = Style::default().fg(theme.accent).bg(bg);
    let hidden_right = visible_cols
        .last()
        .is_some_and(|&(last, _)| order.last().is_some_and(|&end| end != last));
    if hidden_right && right < area.right() {
        put(buf, right, area.y, right + 1, "›", marker_style);
    }
    let pinned = usize::from(state.frozen);
    if state.h_scroll > pinned && geometry.gutter_width > 0 {
        put(
            buf,
            area.x + geometry.gutter_width - 1,
            area.y,
            area.x + geometry.gutter_width,
            "‹",
            marker_style,
        );
    }
    if state.window.fetch_in_flight && (state.window.tick_count / 10) % 2 == 0 {
        put(
            buf,
            area.x,
            area.y + 1,
            area.x + 1,
            &symbols.loading.to_string(),
            Style::default().fg(theme.accent).bg(bg),
        );
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
    match insert_row {
        Some(insert_row) if (insert_row.insert_position as i64) < state.viewport_start => {
            state.viewport_start + 1
        }
        _ => state.viewport_start,
    }
}

fn display_row_kind<'a>(
    state: &GridState,
    insert_row: Option<&'a InsertRowState>,
    display_abs_row: i64,
) -> Option<VisibleGridRow<'a>> {
    let real_abs = match insert_row {
        Some(insert_row) => {
            let insert_pos = insert_row.insert_position as i64;
            if display_abs_row == insert_pos {
                return Some(VisibleGridRow::Insert(insert_row));
            }
            if display_abs_row > insert_pos {
                display_abs_row - 1
            } else {
                display_abs_row
            }
        }
        None => display_abs_row,
    };
    (real_abs >= 0 && real_abs < state.window.total_rows)
        .then_some(VisibleGridRow::Data { real_abs })
}

fn render_data_rows(buf: &mut Buffer, frame: &GridFrame) {
    let GridFrame {
        area,
        state,
        insert_row,
        ..
    } = *frame;
    let display_start = display_viewport_start(state, insert_row);
    let display_total_rows = total_display_rows(state, insert_row);
    for row_in_view in 0..state.window.viewport_rows {
        let display_abs_row = display_start + row_in_view as i64;
        let row_y = area.y + HEADER_ROWS + row_in_view as u16;
        if display_abs_row >= display_total_rows || row_y >= area.bottom() {
            break;
        }
        match display_row_kind(state, insert_row, display_abs_row) {
            Some(VisibleGridRow::Data { real_abs }) => {
                render_existing_row(buf, frame, row_y, real_abs)
            }
            Some(VisibleGridRow::Insert(insert_state)) => {
                render_insert_row(buf, frame, row_y, insert_state)
            }
            None => break,
        }
    }
}

/// Draws a gutter row number.
fn render_gutter(buf: &mut Buffer, frame: &GridFrame, row_y: u16, label: &str, style: Style) {
    let text = format!("{:>width$} ", label, width = frame.geometry.gutter_digits);
    put(
        buf,
        frame.area.x,
        row_y,
        frame.area.x + frame.geometry.gutter_width,
        &text,
        style,
    );
}

/// Marks a focused cell inside its own bounds: a tinted background, an accent
/// bar in the leading padding cell, and bold accent content.
fn paint_focus(buf: &mut Buffer, cell: Rect, background: Color, theme: &Theme, symbols: &Symbols) {
    buf.set_style(cell, Style::default().bg(background));
    put(
        buf,
        cell.x,
        cell.y,
        cell.right(),
        &symbols.active_bar.to_string(),
        Style::default().fg(theme.accent).bg(background),
    );
}

fn render_existing_row(buf: &mut Buffer, frame: &GridFrame, row_y: u16, abs_row: i64) {
    let GridFrame {
        area,
        geometry,
        visible_cols,
        state,
        insert_row,
        theme,
        symbols,
    } = *frame;
    let is_focused = abs_row == state.focused_row as i64;
    let row_bg = row_background(state, theme, abs_row, is_focused);
    let right = area.x + geometry.gutter_width + geometry.data_width;
    buf.set_style(
        Rect::new(area.x, row_y, right - area.x, 1),
        Style::default().bg(row_bg),
    );
    let gutter_fg = if is_focused || state.is_row_selected(abs_row) {
        theme.accent
    } else {
        theme.fg_faint
    };
    render_gutter(
        buf,
        frame,
        row_y,
        &(abs_row + 1).to_string(),
        Style::default().bg(row_bg).fg(gutter_fg),
    );

    let Some(row) = state.window.get_row(abs_row) else {
        put(
            buf,
            area.x + geometry.gutter_width + 1,
            row_y,
            right,
            &symbols.ellipsis.to_string(),
            Style::default().fg(theme.fg_faint).bg(row_bg),
        );
        return;
    };
    let focus_visible = show_cell_focus(state, insert_row) && insert_row.is_none();
    let mut col_x = area.x + geometry.gutter_width;
    for &(col, width) in visible_cols {
        if col_x >= right {
            break;
        }
        let width = width.min(right - col_x);
        let cell = Rect::new(col_x, row_y, width, 1);
        let focused = focus_visible && is_focused && col == state.focused_col;
        let cell_bg = if focused {
            let background = focused_cell_background(row_bg, theme);
            paint_focus(buf, cell, background, theme, symbols);
            background
        } else {
            row_bg
        };
        if let Some(value) = row.get(col) {
            let (text, align) = cell_text(value, state.kinds[col], symbols);
            let inner = width.saturating_sub(2) as usize;
            let fitted = fit_cell(&text, inner, align, symbols.ellipsis);
            let pad = inner.saturating_sub(text_width(&fitted)) as u16;
            let offset = match align {
                Align::Left => 0,
                Align::Right => pad,
                Align::Center => pad / 2,
            };
            let mut style = cell_style(state, col, value, theme).bg(cell_bg);
            if focused {
                style = style.fg(theme.accent).add_modifier(Modifier::BOLD);
            }
            put(
                buf,
                col_x + 1 + offset,
                row_y,
                col_x + width.saturating_sub(1),
                &fitted,
                style,
            );
        }
        col_x += width;
    }
}

fn render_insert_row(buf: &mut Buffer, frame: &GridFrame, row_y: u16, insert_row: &InsertRowState) {
    let GridFrame {
        area,
        geometry,
        visible_cols,
        theme,
        symbols,
        ..
    } = *frame;
    let row_bg = insert_row_background(theme);
    let right = area.x + geometry.gutter_width + geometry.data_width;
    buf.set_style(
        Rect::new(area.x, row_y, right - area.x, 1),
        Style::default().bg(row_bg),
    );
    render_gutter(
        buf,
        frame,
        row_y,
        "+",
        Style::default()
            .bg(row_bg)
            .fg(theme.accent)
            .add_modifier(Modifier::BOLD),
    );

    let mut col_x = area.x + geometry.gutter_width;
    for &(col, width) in visible_cols {
        if col_x >= right {
            break;
        }
        let width = width.min(right - col_x);
        let Some(field) = insert_row.fields.get(col) else {
            col_x += width;
            continue;
        };
        let cell = Rect::new(col_x, row_y, width, 1);
        let inner = Rect::new(col_x + 1, row_y, width.saturating_sub(2), 1);
        let valid_fg = if field.is_valid() {
            theme.fg_dim
        } else {
            theme.red
        };
        if col == insert_row.selected {
            let background = focused_cell_background(row_bg, theme);
            paint_focus(buf, cell, background, theme, symbols);
            let style = Style::default()
                .fg(if field.is_valid() {
                    theme.accent
                } else {
                    theme.red
                })
                .bg(background)
                .add_modifier(Modifier::BOLD);
            if field.touched {
                field.input.render(
                    buf,
                    inner,
                    style,
                    Some((symbols.cursor, style)),
                    symbols.ellipsis,
                );
            } else {
                let text = truncate_with_ellipsis(
                    &field.placeholder(),
                    inner.width.saturating_sub(1) as usize,
                    symbols.ellipsis,
                );
                let end = put(
                    buf,
                    inner.x,
                    row_y,
                    inner.right(),
                    &text,
                    style.add_modifier(Modifier::DIM),
                );
                put(
                    buf,
                    end,
                    row_y,
                    inner.right(),
                    &symbols.cursor.to_string(),
                    style,
                );
            }
        } else {
            let text = truncate_with_ellipsis(
                &field.placeholder(),
                inner.width as usize,
                symbols.ellipsis,
            );
            let fg = if field.writable {
                valid_fg
            } else {
                theme.fg_faint
            };
            put(
                buf,
                inner.x,
                row_y,
                inner.right(),
                &text,
                Style::default().fg(fg).bg(row_bg),
            );
        }
        col_x += width;
    }
}

fn scrollbar(area: Rect, state: &GridState) -> (Scrollbar, u16, u16) {
    let track = area.height.saturating_sub(HEADER_ROWS);
    (
        Scrollbar {
            offset: state.viewport_start.max(0) as usize,
            total: state.window.total_rows.max(0) as usize,
            viewport: state.window.viewport_rows,
        },
        area.y + HEADER_ROWS,
        track,
    )
}

/// Starts a scrollbar drag at row `y`: returns how far into the thumb it was
/// grabbed and the viewport offset to scroll to.
pub(crate) fn scrollbar_drag_start(area: Rect, state: &GridState, y: u16) -> Option<(u16, i64)> {
    let (bar, top, track) = scrollbar(area, state);
    let thumb = bar.thumb(track)?;
    let cell = y.checked_sub(top).filter(|cell| *cell < track)?;
    let grab = if cell >= thumb.start && cell < thumb.start + thumb.len {
        cell - thumb.start
    } else {
        thumb.len / 2
    };
    Some((grab, bar.offset_at(track, cell, grab) as i64))
}

/// The viewport offset for a drag at row `y` that grabbed the thumb at `grab`.
pub(crate) fn scrollbar_drag_offset(
    area: Rect,
    state: &GridState,
    y: u16,
    grab: u16,
) -> Option<i64> {
    let (bar, top, track) = scrollbar(area, state);
    bar.thumb(track)?;
    Some(bar.offset_at(track, y.saturating_sub(top), grab) as i64)
}

fn render_empty(buf: &mut Buffer, frame: &GridFrame) {
    let GridFrame {
        area,
        geometry,
        state,
        theme,
        ..
    } = *frame;
    let (message, hint) = if state.window.fetch_in_flight {
        ("Loading rows…", "")
    } else if let Some(error) = &state.load_error {
        let hint = crate::ui::widgets::text::sanitize(error);
        put(
            buf,
            area.x + geometry.gutter_width + 1,
            area.y + HEADER_ROWS,
            area.right(),
            "Could not load the rows",
            Style::default().fg(theme.red).bg(theme.bg),
        );
        put(
            buf,
            area.x + geometry.gutter_width + 1,
            area.y + HEADER_ROWS + 1,
            area.right(),
            &hint,
            Style::default().fg(theme.fg_mute).bg(theme.bg),
        );
        return;
    } else if !state.filter.is_empty() {
        ("No rows match the filters", "Press F to clear them")
    } else if state.readonly {
        ("No rows", "")
    } else {
        ("This table is empty", "Press i to insert a row")
    };
    let x = area.x + geometry.gutter_width + 1;
    let y = area.y + HEADER_ROWS;
    put(
        buf,
        x,
        y,
        area.right(),
        message,
        Style::default().fg(theme.fg_mute).bg(theme.bg),
    );
    put(
        buf,
        x,
        y + 1,
        area.right(),
        hint,
        Style::default().fg(theme.fg_faint).bg(theme.bg),
    );
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
    state.set_viewport_rows(area.height.saturating_sub(HEADER_ROWS) as usize);
    let geometry = geometry(area, state);
    if state.widths_dirty || state.col_widths.len() != state.columns.len() {
        state.recompute_col_widths(symbols);
    }
    if state.avail_col_width != geometry.data_width {
        state.avail_col_width = geometry.data_width;
        state.adjust_h_scroll();
    }
    let visible_cols = compute_visible_cols(state, geometry.data_width);

    let buf = frame.buffer_mut();
    buf.set_style(area, Style::default().bg(theme.bg));
    let grid_frame = GridFrame {
        area,
        geometry,
        visible_cols: &visible_cols,
        state,
        insert_row,
        theme,
        symbols,
    };

    if area.height >= HEADER_ROWS {
        render_header(buf, &grid_frame);
    }
    if area.height <= HEADER_ROWS {
        return;
    }
    if state.window.total_rows == 0 && insert_row.is_none() {
        render_empty(buf, &grid_frame);
    } else {
        render_data_rows(buf, &grid_frame);
        let (bar, top, track) = scrollbar(area, state);
        bar.render(
            buf,
            Rect::new(area.right() - 1, top, 1, track),
            theme.bg,
            theme,
            symbols,
        );
    }
    alphabet_rail::render_rail(frame, area, state, theme);
}

pub enum GridHit {
    Header(usize),
    RowGutter(usize),
    Cell { row: usize, col: usize },
    AlphabetRail(char),
    Scrollbar,
}

pub fn hit_test(area: Rect, state: &GridState, x: u16, y: u16) -> Option<GridHit> {
    if !area.contains(ratatui::layout::Position { x, y }) {
        return None;
    }
    let geometry = geometry(area, state);
    if x == area.right() - 1 && y >= area.y + HEADER_ROWS {
        let (bar, _, track) = scrollbar(area, state);
        return bar.thumb(track).map(|_| GridHit::Scrollbar);
    }
    if let Some(letter) = alphabet_rail::hit_test(area, state, x, y) {
        return Some(GridHit::AlphabetRail(letter));
    }
    if y < area.y + HEADER_ROWS {
        return hit_test_col(area, state, x, geometry).map(GridHit::Header);
    }
    let row = hit_test_row(area, state, y)?;
    if x < area.x + geometry.gutter_width {
        return Some(GridHit::RowGutter(row));
    }
    let col = hit_test_col(area, state, x, geometry)?;
    Some(GridHit::Cell { row, col })
}

fn hit_test_row(area: Rect, state: &GridState, y: u16) -> Option<usize> {
    let row_in_view = y.checked_sub(area.y + HEADER_ROWS)? as i64;
    let abs_row = state.viewport_start + row_in_view;
    (abs_row >= 0 && abs_row < state.window.total_rows).then_some(abs_row as usize)
}

fn hit_test_col(area: Rect, state: &GridState, x: u16, geometry: GridGeometry) -> Option<usize> {
    let mut col_x = area.x + geometry.gutter_width;
    for (col, width) in compute_visible_cols(state, geometry.data_width) {
        let end = col_x.saturating_add(width);
        if x >= col_x && x < end {
            return Some(col);
        }
        col_x = end;
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::{backend::TestBackend, Terminal};

    fn make_col(name: &str, col_type: &str, is_pk: bool) -> Column {
        Column {
            name: name.to_string(),
            col_type: col_type.to_string(),
            not_null: false,
            default_value: None,
            is_pk,
            pk_position: i64::from(is_pk),
            writable: true,
        }
    }

    fn grid(columns: Vec<Column>, rows: Vec<Vec<SqlValue>>, total_rows: i64) -> GridState {
        let count = columns.len();
        GridState::new(GridInit {
            table_name: "t".to_string(),
            columns,
            fk_cols: vec![false; count],
            enumerated_values: vec![Vec::new(); count],
            width_sample_rows: rows.clone(),
            rows,
            total_rows,
            area_width: 80,
        })
    }

    fn render(state: &mut GridState, width: u16, height: u16) -> Vec<String> {
        let theme = Theme::default();
        let symbols = Symbols::default_with_nerd_font(false);
        let mut terminal = Terminal::new(TestBackend::new(width, height)).expect("terminal");
        terminal
            .draw(|frame| {
                let area = frame.area();
                render_grid(frame, area, state, None, &theme, &symbols);
            })
            .expect("draw");
        let buffer = terminal.backend().buffer();
        (0..height)
            .map(|y| {
                (0..width)
                    .map(|x| buffer[(x, y)].symbol().to_string())
                    .collect()
            })
            .collect()
    }

    #[test]
    fn enum_colours_are_distinct_and_stable() {
        let values: Vec<String> = ["new", "open", "done", "late"]
            .iter()
            .map(|v| v.to_string())
            .collect();
        let slots = enum_slots(&values);
        let mut used: Vec<usize> = slots.values().copied().collect();
        used.sort_unstable();
        used.dedup();
        assert_eq!(used.len(), 4);
        let mut reordered = values.clone();
        reordered.reverse();
        assert_eq!(
            enum_slots(&reordered),
            slots,
            "colours do not depend on order"
        );
    }

    #[test]
    fn headers_show_full_names_and_numbers_are_never_cut() {
        let mut state = grid(
            vec![
                make_col("CustomerId", "INTEGER", true),
                make_col("Bytes", "INTEGER", false),
            ],
            vec![
                vec![SqlValue::Integer(1), SqlValue::Integer(1_000)],
                vec![SqlValue::Integer(2), SqlValue::Integer(11_170_334)],
            ],
            2,
        );
        let lines = render(&mut state, 60, 8);
        assert!(lines[0].contains("CustomerId"), "{:?}", lines[0]);
        assert!(lines.iter().any(|line| line.contains("11170334")));
    }

    #[test]
    fn focus_is_drawn_inside_the_cell_without_covering_neighbours() {
        let rows: Vec<Vec<SqlValue>> = (0..5)
            .map(|i| vec![SqlValue::Text(format!("value{i}"))])
            .collect();
        let mut state = grid(vec![make_col("name", "TEXT", false)], rows, 5);
        state.focus_cell(2, 0);
        let lines = render(&mut state, 40, 10);
        assert!(
            lines[4].contains("value1"),
            "row above stays visible: {:?}",
            lines[4]
        );
        assert!(
            lines[6].contains("value3"),
            "row below stays visible: {:?}",
            lines[6]
        );
        assert!(lines[5].contains('▌'));
    }

    #[test]
    fn a_taller_viewport_requests_the_rows_it_shows() {
        let rows: Vec<Vec<SqlValue>> = (0..50).map(|i| vec![SqlValue::Integer(i)]).collect();
        let mut state = grid(vec![make_col("id", "INTEGER", true)], rows, 500);
        state.needs_fetch = false;
        state.set_viewport_rows(60);
        assert!(state.needs_fetch);
    }

    #[test]
    fn spare_width_goes_to_columns_that_lack_it() {
        let mut state = grid(
            vec![
                make_col("id", "INTEGER", true),
                make_col("name", "TEXT", false),
            ],
            vec![vec![SqlValue::Integer(1), SqlValue::Text("x".repeat(30))]],
            1,
        );
        state.recompute_col_widths(&Symbols::default_with_nerd_font(false));
        let id_width = state.col_widths[0];
        let visible = compute_visible_cols(&state, 100);
        assert_eq!(visible[0].1, id_width, "numbers are not stretched");
        assert!(visible[1].1 >= 32);
    }

    #[test]
    fn hidden_frozen_and_multi_sorted_columns_persist_by_name() {
        let mut state = grid(
            vec![
                make_col("a", "TEXT", false),
                make_col("b", "TEXT", false),
                make_col("c", "INTEGER", false),
            ],
            Vec::new(),
            0,
        );
        state.hide_column(1);
        state.cycle_sort(2);
        state.add_sort_key(0);
        state.toggle_frozen();
        state.adjust_column_width(0, 3);
        let settings = state.settings();
        assert_eq!(
            settings
                .sort
                .iter()
                .map(|k| k.column.as_str())
                .collect::<Vec<_>>(),
            vec!["c", "a"]
        );

        let mut restored = grid(
            vec![
                make_col("c", "INTEGER", false),
                make_col("a", "TEXT", false),
                make_col("b", "TEXT", false),
            ],
            Vec::new(),
            0,
        );
        restored.apply_settings(settings);
        assert_eq!(restored.display_columns(), vec![0, 1]);
        assert_eq!(restored.order_by().len(), 2);
        assert_eq!(restored.order_by()[0].column, "c");
        assert!(restored.frozen && restored.width_overrides.contains_key(&1));
    }

    #[test]
    fn schema_changes_keep_settings_of_surviving_columns() {
        let mut state = grid(
            vec![make_col("a", "TEXT", false), make_col("b", "TEXT", false)],
            Vec::new(),
            0,
        );
        state.set_enum_values(vec![vec!["x".into()], Vec::new()]);
        state.cycle_sort(1);
        state.set_columns(
            vec![make_col("b", "TEXT", false), make_col("z", "TEXT", false)],
            vec![false, false],
        );
        assert_eq!(
            state.sort,
            vec![SortSpec {
                col_idx: 0,
                direction: SortDir::Asc
            }]
        );
        assert!(state.enumerated_values[0].is_empty() && state.enumerated_values[1].is_empty());
    }

    #[test]
    fn scrollbar_drag_maps_to_viewport_offsets() {
        let mut state = grid(vec![make_col("id", "INTEGER", true)], Vec::new(), 1000);
        state.window.viewport_rows = 20;
        let area = Rect::new(0, 0, 40, 23);
        let (grab, offset) = scrollbar_drag_start(area, &state, 3).expect("drag start");
        assert_eq!(offset, 0);
        assert_eq!(scrollbar_drag_offset(area, &state, 22, grab), Some(980));
    }

    #[test]
    fn scroll_down_moves_viewport_before_the_bottom_edge() {
        let mut state = grid(vec![make_col("id", "INTEGER", true)], Vec::new(), 100);
        state.window.viewport_rows = 20;
        for _ in 0..16 {
            state.scroll_down(1);
        }
        assert!(state.viewport_start > 0);
    }

    #[test]
    fn row_selection_tracks_anchor_and_can_retract() {
        let mut state = grid(vec![make_col("id", "INTEGER", true)], Vec::new(), 20);
        state.window.viewport_rows = 10;
        state.focus_cell(3, 0);
        state.extend_row_selection_down(2);
        assert_eq!(state.selected_rows(), vec![3, 4]);
        state.extend_row_selection_up(1);
        assert_eq!(state.selected_rows(), vec![3]);
        state.toggle_row_selected(7);
        assert_eq!(state.selected_rows(), vec![3, 7]);
    }

    #[test]
    fn deselecting_from_select_all_records_only_the_exception() {
        let mut state = grid(vec![make_col("name", "TEXT", false)], Vec::new(), 1_000_000);
        state.select_all_rows();
        state.toggle_row_selected(7);
        assert_eq!(
            state.row_selection,
            RowSelection::All {
                except: BTreeSet::from([7])
            }
        );
        assert!(!state.is_row_selected(7) && state.is_row_selected(8));
        assert_eq!(state.selected_row_count(), 999_999);
        state.toggle_row_selected(7);
        assert_eq!(state.row_selection, RowSelection::all());
    }

    #[test]
    fn row_selection_hides_cell_focus_except_while_inserting() {
        let mut state = grid(vec![make_col("name", "TEXT", false)], Vec::new(), 10);
        state.select_all_rows();
        assert!(!show_cell_focus(&state, None));
        let insert = InsertRowState::new("t".into(), state.columns.clone(), 0);
        assert!(show_cell_focus(&state, Some(&insert)));
        state.focus_cell(4, 0);
        assert_eq!(state.row_selection, RowSelection::None);
    }
}
