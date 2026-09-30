mod features;
mod input;

use std::{collections::VecDeque, sync::Arc};

use ratatui::layout::Rect;
use tokio::sync::mpsc::UnboundedSender;

use crate::{
    config::Config,
    db::{
        self,
        query::ViewQuery,
        schema::{Column, Schema},
        types::{ColumnKind, SqlValue},
        DbPool, SearchHit, SqlOutcome,
    },
    grid::{GridInit, GridState, RowSelection},
    symbols::Symbols,
    theme::Theme,
    ui::{
        popup::{
            command_palette::CopyFormat, global_search::GlobalHit, schema_view::SchemaLine,
            CommandPaletteState, DatePickerState, FilterPopupState, HelpState, InsertRowState,
            PaletteCommand, PopupKind, TextEditorState, ValuePickerState,
        },
        sidebar::SidebarState,
        toast::{ToastKind, ToastState},
    },
    view_settings,
};

#[derive(Debug, Clone, PartialEq)]
pub enum AppMode {
    Browse,
    Edit,
}

pub enum FocusPane {
    Sidebar,
    Grid,
}

#[derive(Debug, Clone)]
pub struct JumpFrame {
    pub table: String,
    pub rowid: i64,
    pub col: usize,
}

#[derive(Clone)]
pub struct PendingJumpTarget {
    pub table: String,
    pub rowid: i64,
    pub col: Option<usize>,
}

/// An open tab. Inactive tabs keep their grid (focus, scroll, selection,
/// sort, filters and columns) so switching back restores it.
pub struct TableTab {
    pub table_name: String,
    saved: Option<GridState>,
}

impl TableTab {
    pub fn new(table_name: String) -> Self {
        Self {
            table_name,
            saved: None,
        }
    }
}

#[derive(Debug, Clone)]
pub enum UndoOp {
    Update,
    Insert,
    Delete,
}

#[derive(Debug, Clone)]
pub struct UndoFrame {
    pub op: UndoOp,
    pub table: String,
    pub rowid: i64,
    pub cols: Vec<(String, SqlValue)>,
}

#[derive(Debug, Clone)]
pub enum ConfirmKind {
    DeleteRow {
        table: String,
        rowid: i64,
    },
    DeleteSelectedRows {
        table: String,
        rowids: Vec<i64>,
    },
    /// Deletes the whole table except the rows with the `keep` rowids.
    ClearTable {
        table: String,
        keep: Vec<i64>,
    },
}

pub struct PendingConfirm {
    pub message: String,
    pub kind: ConfirmKind,
    pub created: std::time::Instant,
}

struct GridScrollbarDrag {
    grab_offset: u16,
}

const VALUE_PICKER_DISTINCT_LIMIT: usize = 100;
const UNDO_HISTORY_LIMIT: usize = 100;
pub(crate) const CONFIRM_TIMEOUT_SECS: u64 = 8;

struct FocusedCellContext {
    col: Column,
    table_name: String,
    rowid: i64,
    cell_value: SqlValue,
    is_fk: bool,
}

pub struct App {
    pub schema: Schema,
    pub sidebar: SidebarState,
    pub should_quit: bool,
    pub dirty: bool,
    pub theme: Theme,
    pub symbols: Symbols,
    pub focus: FocusPane,
    pub open_tabs: Vec<TableTab>,
    pub active_tab: Option<usize>,
    pub sidebar_visible: bool,
    /// The grid of the active tab.
    pub grid: Option<GridState>,
    pub mode: AppMode,
    pub popup: Option<PopupKind>,
    /// Popups suspended under the current one (help over an editor, the JSON
    /// viewer over a record); closing the top popup restores the next.
    popup_stack: Vec<PopupKind>,
    pub toast: ToastState,
    /// Whether writes are refused.
    pub readonly: bool,
    /// Started with `--readonly`: the pool itself cannot write, so read-only
    /// mode cannot be switched off.
    pub readonly_locked: bool,
    pub jump_stack: Vec<JumpFrame>,
    pub db_path: String,
    pub undo_stack: VecDeque<UndoFrame>,
    pub pending_confirm: Option<PendingConfirm>,
    pub screen_area: Rect,
    pub tabbar_area: Rect,
    pub sidebar_area: Option<Rect>,
    pub grid_outer_area: Option<Rect>,
    pub grid_inner_area: Option<Rect>,
    pub pending_jump_target: Option<PendingJumpTarget>,
    grid_scrollbar_drag: Option<GridScrollbarDrag>,
    pool: Arc<DbPool>,
    tx: UnboundedSender<Message>,
    pub file_check_in_flight: bool,
    pub pending_external_refresh: bool,
    file_change_pending: bool,
    grid_request_serial: u64,
    navigation_request_serial: u64,
    write_in_flight: bool,
    popup_request_serial: u64,
    /// `'` was pressed; the next letter jumps in a text-sorted column.
    letter_jump_armed: bool,
}

#[derive(Debug)]
pub enum Message {
    Quit,
    Key(crossterm::event::KeyEvent),
    Mouse(crossterm::event::MouseEvent),
    Resize,
    Tick,
    WindowReady {
        request_id: u64,
        table: String,
        offset: i64,
        rows: Vec<Vec<SqlValue>>,
        rowids: Vec<Option<i64>>,
        /// The row count, when it had to be recounted.
        total_rows: Option<i64>,
    },
    GridReadFailed {
        request_id: u64,
        table: String,
        error: String,
    },
    EnumValuesReady {
        table: String,
        sets: Vec<Vec<String>>,
    },
    ScrollDown(usize),
    ScrollUp(usize),
    MoveRight,
    MoveLeft,
    MoveDown,
    MoveUp,
    MoveColFirst,
    MoveColLast,
    MoveFirstCell,
    MoveLastCell,
    OpenPopup,
    OpenDirectEdit,
    SetFocusedCellNull,
    CommitInsertRow,
    ClosePopup,
    CommitEdit,
    EditCommitted {
        rowid: i64,
        table: String,
        col: String,
        original: SqlValue,
    },
    EditFailed(String),
    DistinctValuesReady {
        request_id: u64,
        table: String,
        rowid: i64,
        col: Column,
        original: SqlValue,
        values: Vec<String>,
    },
    DistinctValuesFailed {
        request_id: u64,
        table: String,
        rowid: i64,
        col: Column,
        original: SqlValue,
        error: String,
    },
    /// Results for the Find or foreign-key picker search.
    SearchReady {
        request_id: u64,
        hits: Vec<SearchHit>,
    },
    SearchFailed {
        request_id: u64,
        error: String,
    },
    GlobalSearchReady {
        request_id: u64,
        hits: Vec<GlobalHit>,
    },
    JumpToFk,
    FkJumpReady {
        request_id: u64,
        frame: JumpFrame,
        table: String,
        rowid: i64,
    },
    NavigationFailed {
        request_id: u64,
        error: String,
    },
    JumpBack,
    JumpOffsetReady {
        request_id: u64,
        table: String,
        offset: Option<i64>,
        col: Option<usize>,
    },
    CycleSort,
    AddSortKey,
    JumpToLetter(char),
    JumpToSortedOffset {
        request_id: u64,
        table: String,
        offset: i64,
    },
    OpenFilterPopup,
    ClearFilters,
    InsertRow,
    DeleteRow,
    ConfirmReady {
        message: String,
        kind: ConfirmKind,
    },
    ConfirmDelete,
    CancelConfirm,
    UndoAction,
    RowInserted {
        table: String,
        rowid: i64,
    },
    RowDeleted {
        table: String,
        rowid: i64,
        cols: Vec<(String, SqlValue)>,
    },
    RowsDeleted {
        table: String,
        count: usize,
    },
    UndoCompleted {
        table: String,
        message: String,
    },
    UndoFailed {
        frame: UndoFrame,
        error: String,
    },
    OpenCommandPalette,
    OpenHelp,
    ExportDone {
        path: String,
        count: u64,
    },
    ExportFailed(String),
    SchemaReady(Schema),
    SchemaLoadFailed {
        external: bool,
        error: String,
    },
    CopyCell,
    CopyRows(CopyFormat),
    CopyReady {
        text: String,
        message: String,
    },
    /// A background task finished with something to tell the user.
    Notify(String, ToastKind),
    FileChanged,
    ExternalRefresh(Schema),
    OpenFind,
    SchemaViewReady {
        name: String,
        ddl: String,
        lines: Vec<SchemaLine>,
    },
    SqlDone {
        request_id: u64,
        result: Result<SqlOutcome, String>,
    },
}

impl App {
    pub fn new(
        schema: Schema,
        config: Config,
        pool: Arc<DbPool>,
        tx: UnboundedSender<Message>,
        readonly: bool,
        db_path: String,
    ) -> Self {
        let theme = config
            .resolve_theme()
            .expect("config should be validated before app creation");
        let symbols = config
            .resolve_symbols()
            .expect("config should be validated before app creation");
        Self {
            schema,
            sidebar: SidebarState::default(),
            should_quit: false,
            dirty: true,
            theme,
            symbols,
            focus: FocusPane::Sidebar,
            open_tabs: Vec::new(),
            active_tab: None,
            sidebar_visible: true,
            grid: None,
            mode: AppMode::Browse,
            popup: None,
            popup_stack: Vec::new(),
            toast: ToastState::new(),
            readonly,
            readonly_locked: readonly,
            jump_stack: Vec::new(),
            db_path,
            undo_stack: VecDeque::new(),
            pending_confirm: None,
            screen_area: Rect::default(),
            tabbar_area: Rect::default(),
            sidebar_area: None,
            grid_outer_area: None,
            grid_inner_area: None,
            pending_jump_target: None,
            grid_scrollbar_drag: None,
            pool,
            tx,
            file_check_in_flight: false,
            pending_external_refresh: false,
            file_change_pending: false,
            grid_request_serial: 0,
            navigation_request_serial: 0,
            write_in_flight: false,
            popup_request_serial: 0,
            letter_jump_armed: false,
        }
    }

    pub fn update(&mut self, msg: Message) {
        match msg {
            Message::Quit => self.should_quit = true,
            Message::Resize => self.dirty = true,
            Message::Key(key) => {
                self.dirty = true;
                self.handle_key(key);
            }
            Message::Mouse(ev) => {
                self.dirty = true;
                self.handle_mouse(ev);
            }
            Message::Tick => self.tick(),
            Message::WindowReady {
                request_id,
                table,
                offset,
                rows,
                rowids,
                total_rows,
            } => {
                if request_id != self.grid_request_serial {
                    return;
                }
                self.on_window_ready(table, offset, rows, rowids, total_rows);
            }
            Message::GridReadFailed {
                request_id,
                table,
                error,
            } => {
                if request_id == self.grid_request_serial {
                    if let Some(grid) = self.grid_for(&table) {
                        grid.window.fetch_in_flight = false;
                        grid.needs_fetch = false;
                        grid.load_error = Some(error.clone());
                    }
                    self.toast.push(error, ToastKind::Error);
                    self.dirty = true;
                }
            }
            Message::EnumValuesReady { table, sets } => {
                if let Some(grid) = self.grid_for(&table) {
                    if sets.len() == grid.columns.len() {
                        grid.set_enum_values(sets);
                    }
                }
                self.dirty = true;
            }
            Message::ScrollDown(n) => self.scroll_grid_down(n),
            Message::ScrollUp(n) => self.scroll_grid_up(n),
            Message::MoveDown => self.scroll_grid_down(1),
            Message::MoveUp => self.scroll_grid_up(1),
            Message::MoveRight => self.update_grid(GridState::move_col_right),
            Message::MoveLeft => self.update_grid(GridState::move_col_left),
            Message::MoveColFirst => self.update_grid(GridState::move_col_first),
            Message::MoveColLast => self.update_grid(GridState::move_col_last),
            Message::MoveFirstCell => self.update_grid(|grid| {
                grid.move_col_first();
                grid.commit_row_selection();
                grid.scroll_to_row(0);
            }),
            Message::MoveLastCell => self.update_grid(|grid| {
                grid.move_col_last();
                grid.commit_row_selection();
                grid.scroll_to_end();
            }),
            Message::OpenPopup => {
                self.dirty = true;
                if !self.ensure_writable() {
                    return;
                }
                if let Some(cell) = self.focused_cell_context() {
                    if !(cell.is_fk && self.open_fk_picker(&cell)) {
                        self.open_cell_editor(cell);
                    }
                }
            }
            Message::OpenDirectEdit => {
                self.dirty = true;
                if !self.ensure_writable() {
                    return;
                }
                if let Some(cell) = self.focused_cell_context() {
                    self.open_text_editor(
                        cell.table_name,
                        cell.rowid,
                        cell.col.name,
                        cell.col.col_type,
                        cell.cell_value,
                    );
                }
            }
            Message::SetFocusedCellNull => {
                self.dirty = true;
                if !self.ensure_writable() {
                    return;
                }
                if let Some(cell) = self.focused_cell_context() {
                    if cell.col.not_null {
                        self.toast.push("Column is NOT NULL", ToastKind::Error);
                    } else if cell.cell_value == SqlValue::Null {
                        self.toast.push("Cell is already NULL", ToastKind::Info);
                    } else {
                        self.submit_cell_edit(
                            cell.table_name,
                            cell.col.name,
                            cell.rowid,
                            SqlValue::Null,
                            cell.cell_value,
                        );
                    }
                }
            }
            Message::ClosePopup => {
                self.finish_popup();
                self.dirty = true;
            }
            Message::CommitEdit => self.commit_edit(),
            Message::EditCommitted {
                rowid,
                table,
                col,
                original,
            } => {
                self.write_in_flight = false;
                self.push_undo(UndoFrame {
                    op: UndoOp::Update,
                    table,
                    rowid,
                    cols: vec![(col, original)],
                });
                if !self.finish_popup() {
                    self.refresh_grid_rows();
                }
                self.toast.push("Cell updated", ToastKind::Success);
                self.dirty = true;
            }
            Message::EditFailed(err) => {
                self.write_in_flight = false;
                self.toast.push(format!("Error: {err}"), ToastKind::Error);
                self.dirty = true;
            }
            Message::DistinctValuesReady {
                request_id,
                table,
                rowid,
                col,
                original,
                values,
            } => {
                if request_id != self.popup_request_serial {
                    return;
                }
                if self.focused_cell_is(&table, rowid, &col.name) {
                    if should_use_value_picker(&values) {
                        self.open_popup(PopupKind::ValuePicker(ValuePickerState::new(
                            table,
                            rowid,
                            col.name,
                            col.col_type,
                            values,
                            original,
                        )));
                    } else {
                        self.open_text_editor(table, rowid, col.name, col.col_type, original);
                    }
                }
                self.dirty = true;
            }
            Message::DistinctValuesFailed {
                request_id,
                table,
                rowid,
                col,
                original,
                error,
            } => {
                if request_id != self.popup_request_serial {
                    return;
                }
                if self.focused_cell_is(&table, rowid, &col.name) {
                    self.toast
                        .push(format!("Distinct lookup failed: {error}"), ToastKind::Error);
                    self.open_text_editor(table, rowid, col.name, col.col_type, original);
                }
                self.dirty = true;
            }
            Message::SearchReady { request_id, hits } => {
                if request_id == self.popup_request_serial {
                    self.on_search_ready(hits);
                }
            }
            Message::SearchFailed { request_id, error } => {
                if request_id == self.popup_request_serial {
                    self.on_search_failed(error);
                }
            }
            Message::GlobalSearchReady { request_id, hits } => {
                if request_id == self.popup_request_serial {
                    self.on_global_search_ready(hits);
                }
            }
            Message::JumpToFk => {
                self.jump_to_foreign_key();
                self.dirty = true;
            }
            Message::FkJumpReady {
                request_id,
                frame,
                table,
                rowid,
            } => {
                if request_id == self.navigation_request_serial {
                    self.jump_stack.push(frame);
                    self.open_table_at(table, rowid, None);
                }
                self.dirty = true;
            }
            Message::NavigationFailed { request_id, error } => {
                if request_id == self.navigation_request_serial {
                    self.toast
                        .push(format!("Navigation failed: {error}"), ToastKind::Error);
                    self.dirty = true;
                }
            }
            Message::JumpOffsetReady {
                request_id,
                table,
                offset,
                col,
            } => {
                if request_id != self.navigation_request_serial {
                    return;
                }
                match offset {
                    Some(row) if self.grid.as_ref().is_some_and(|g| g.table_name == table) => {
                        self.update_grid(|grid| {
                            let col = col.unwrap_or(grid.focused_col);
                            grid.focus_cell(row as usize, col);
                        });
                    }
                    Some(_) => {}
                    None => self.toast.push(
                        "Row not found in the current view; a filter may hide it",
                        ToastKind::Error,
                    ),
                }
                self.dirty = true;
            }
            Message::CycleSort => self.change_sort(false),
            Message::AddSortKey => self.change_sort(true),
            Message::JumpToSortedOffset {
                request_id,
                table,
                offset,
            } => {
                if request_id != self.navigation_request_serial {
                    return;
                }
                if self
                    .grid
                    .as_ref()
                    .is_some_and(|grid| grid.table_name == table)
                {
                    self.update_grid(|grid| {
                        grid.commit_row_selection();
                        grid.scroll_to_row(offset);
                    });
                }
            }
            Message::JumpToLetter(letter) => {
                self.jump_to_letter(letter);
                self.dirty = true;
            }
            Message::JumpBack => {
                if let Some(frame) = self.jump_stack.pop() {
                    self.open_table_at(frame.table, frame.rowid, Some(frame.col));
                }
                self.dirty = true;
            }
            Message::OpenFilterPopup => {
                let popup = self.grid.as_ref().and_then(|grid| {
                    let col = grid.columns.get(grid.focused_col)?;
                    let col_filter = grid
                        .filter
                        .columns
                        .get(&col.name)
                        .cloned()
                        .unwrap_or_default();
                    Some(FilterPopupState::new(
                        col.name.clone(),
                        col.col_type.clone(),
                        col_filter,
                    ))
                });
                if let Some(popup) = popup {
                    self.open_popup(PopupKind::FilterPopup(popup));
                }
                self.dirty = true;
            }
            Message::ClearFilters => {
                self.next_navigation_request();
                let cleared = self.grid.as_mut().is_some_and(|grid| {
                    let had_filters = !grid.filter.is_empty();
                    grid.filter = crate::filter::FilterSet::default();
                    grid.reset_to_top();
                    had_filters
                });
                if self.grid.is_some() {
                    self.save_view_settings();
                    self.fetch_window_around_focus();
                    if cleared {
                        self.toast.push("Filters cleared", ToastKind::Info);
                    }
                }
                if matches!(self.popup, Some(PopupKind::FilterPopup(_))) {
                    self.finish_popup();
                }
                self.dirty = true;
            }
            Message::InsertRow => self.start_insert_row(),
            Message::CommitInsertRow => self.commit_insert_row(),
            Message::RowInserted { table, rowid } => {
                self.write_in_flight = false;
                self.invalidate_grid_requests();
                if let Some(grid) = self.grid_for(&table) {
                    grid.window.total_rows += 1;
                    grid.invalidate_window();
                }
                self.push_undo(UndoFrame {
                    op: UndoOp::Insert,
                    table,
                    rowid,
                    cols: Vec::new(),
                });
                self.finish_popup();
                self.toast.push("Row inserted", ToastKind::Success);
                self.dirty = true;
            }
            Message::DeleteRow => {
                if self.ensure_writable() {
                    self.request_delete_confirmation();
                }
                self.dirty = true;
            }
            Message::ConfirmReady { message, kind } => {
                self.pending_confirm = Some(PendingConfirm {
                    message,
                    kind,
                    created: std::time::Instant::now(),
                });
                self.dirty = true;
            }
            Message::ConfirmDelete => self.confirm_delete(),
            Message::RowDeleted { table, rowid, cols } => {
                self.write_in_flight = false;
                self.invalidate_grid_requests();
                if let Some(grid) = self.grid_for(&table) {
                    grid.rows_removed(1);
                }
                self.push_undo(UndoFrame {
                    op: UndoOp::Delete,
                    table,
                    rowid,
                    cols,
                });
                self.toast.push("Row deleted", ToastKind::Success);
                self.dirty = true;
            }
            Message::RowsDeleted { table, count } => {
                self.write_in_flight = false;
                self.invalidate_grid_requests();
                if let Some(grid) = self.grid_for(&table) {
                    grid.rows_removed(count);
                }
                let message = match count {
                    0 => "No rows deleted".to_string(),
                    1 => "1 row deleted".to_string(),
                    _ => format!("{count} rows deleted"),
                };
                self.toast.push(message, ToastKind::Success);
                self.dirty = true;
            }
            Message::CancelConfirm => {
                self.pending_confirm = None;
                self.toast.push("Deletion cancelled", ToastKind::Info);
                self.dirty = true;
            }
            Message::UndoAction => self.undo(),
            Message::UndoCompleted { table, message } => {
                self.write_in_flight = false;
                self.invalidate_grid_requests();
                if let Some(grid) = self.grid_for(&table) {
                    grid.invalidate_window();
                }
                self.toast.push(message, ToastKind::Info);
                self.dirty = true;
            }
            Message::UndoFailed { frame, error } => {
                self.write_in_flight = false;
                self.undo_stack.push_back(frame);
                self.toast
                    .push(format!("Undo failed: {error}"), ToastKind::Error);
                self.dirty = true;
            }
            Message::OpenCommandPalette => {
                let table_names = self
                    .schema
                    .tables
                    .iter()
                    .chain(&self.schema.views)
                    .map(|t| t.name.clone())
                    .collect();
                self.open_popup(PopupKind::CommandPalette(CommandPaletteState::new(
                    table_names,
                )));
                self.dirty = true;
            }
            Message::OpenHelp => {
                self.toggle_help();
                self.dirty = true;
            }
            Message::ExportDone { path, count } => {
                self.toast.push(
                    format!("Exported {count} rows to {path}"),
                    ToastKind::Success,
                );
                self.dirty = true;
            }
            Message::ExportFailed(error) => {
                self.toast
                    .push(format!("Export failed: {error}"), ToastKind::Error);
                self.dirty = true;
            }
            Message::SchemaReady(schema) => {
                self.schema = schema;
                self.clamp_sidebar_selection();
                self.refresh_active_grid_schema();
                self.toast.push("Schema reloaded", ToastKind::Success);
                self.dirty = true;
            }
            Message::SchemaLoadFailed { external, error } => {
                if external {
                    self.finish_file_check();
                }
                self.toast
                    .push(format!("Schema reload failed: {error}"), ToastKind::Error);
                self.dirty = true;
            }
            Message::CopyCell => {
                self.copy_cell();
                self.dirty = true;
            }
            Message::CopyRows(format) => {
                self.copy_rows(format);
                self.dirty = true;
            }
            Message::CopyReady { text, message } => {
                self.copy_to_clipboard(&text, &message);
                self.dirty = true;
            }
            Message::Notify(message, kind) => {
                self.toast.push(message, kind);
                self.dirty = true;
            }
            Message::FileChanged => {
                if self.file_check_in_flight {
                    self.file_change_pending = true;
                    return;
                }
                self.file_check_in_flight = true;
                self.spawn_schema_load(true);

                // Refresh data now, but never underneath an open popup.
                if self.mode == AppMode::Edit {
                    self.pending_external_refresh = true;
                } else if self.grid.is_some() {
                    self.refresh_grid_rows();
                } else if let Some(table) = self.active_table_name() {
                    self.request_table_view(&table);
                }
                self.dirty = true;
            }
            Message::ExternalRefresh(new_schema) => {
                if self.schema != new_schema {
                    self.schema = new_schema;
                    self.clamp_sidebar_selection();
                    self.toast.push("Schema changed", ToastKind::Info);
                    if self.mode == AppMode::Edit {
                        self.pending_external_refresh = true;
                    } else {
                        self.refresh_active_grid_schema();
                    }
                }
                self.finish_file_check();
                self.dirty = true;
            }
            Message::OpenFind => {
                self.open_find();
                self.dirty = true;
            }
            Message::SchemaViewReady { name, ddl, lines } => {
                self.show_schema_view(name, ddl, lines);
                self.dirty = true;
            }
            Message::SqlDone { request_id, result } => {
                if request_id == self.popup_request_serial {
                    self.on_sql_done(result);
                }
                self.dirty = true;
            }
        }
    }

    pub fn view(&mut self, frame: &mut ratatui::Frame) {
        crate::ui::render(frame, self);
    }

    fn tick(&mut self) {
        self.toast.tick();
        if let Some(grid) = self.grid.as_mut() {
            grid.window.tick_count = grid.window.tick_count.wrapping_add(1);
        }
        self.fetch_window_if_needed();
        if self.grid.as_ref().is_some_and(|g| g.window.fetch_in_flight) {
            self.dirty = true;
        }
        let expired = self
            .pending_confirm
            .as_ref()
            .is_some_and(|c| c.created.elapsed().as_secs() >= CONFIRM_TIMEOUT_SECS);
        if expired {
            self.pending_confirm = None;
            self.toast
                .push("Deletion not confirmed; nothing deleted", ToastKind::Info);
            self.dirty = true;
        }
    }

    fn execute_palette_command(&mut self, cmd: PaletteCommand) {
        let needs_grid = !matches!(
            cmd,
            PaletteCommand::SqlConsole
                | PaletteCommand::SearchAllTables
                | PaletteCommand::ShowSchema
                | PaletteCommand::NextTab
                | PaletteCommand::PrevTab
                | PaletteCommand::CloseTab
                | PaletteCommand::ReloadSchema
                | PaletteCommand::ToggleSidebar
                | PaletteCommand::ToggleReadonly
                | PaletteCommand::Help
                | PaletteCommand::Quit
                | PaletteCommand::SwitchTable(_)
        );
        if needs_grid && self.grid.is_none() {
            self.toast.push("Open a table first", ToastKind::Info);
            return;
        }
        match cmd {
            PaletteCommand::Export(format) => self.open_export(format),
            PaletteCommand::CopyCell => self.copy_cell(),
            PaletteCommand::CopyRows(format) => self.copy_rows(format),
            PaletteCommand::CopyColumn => self.copy_column(),
            PaletteCommand::Find => self.open_find(),
            PaletteCommand::GoToRow => self.open_goto(),
            PaletteCommand::SqlConsole => self.open_sql_console(),
            PaletteCommand::SearchAllTables => self.open_global_search(),
            PaletteCommand::ShowRecord => self.open_record(),
            PaletteCommand::ShowSchema => {
                let name = match self.focus {
                    FocusPane::Sidebar => self.sidebar.selected_name(&self.schema),
                    FocusPane::Grid => None,
                }
                .or_else(|| self.active_table_name());
                match name {
                    Some(name) => self.open_schema(name),
                    None => self.toast.push("Select a table first", ToastKind::Info),
                }
            }
            PaletteCommand::ReferencingRows => self.open_references(),
            PaletteCommand::FilterColumn => self.update(Message::OpenFilterPopup),
            PaletteCommand::ClearFilters => self.update(Message::ClearFilters),
            PaletteCommand::SortColumn => self.change_sort(false),
            PaletteCommand::ShowHiddenColumns => {
                let hidden = self.grid.as_ref().map_or(0, |g| g.hidden.len());
                if hidden == 0 {
                    self.toast.push("No columns are hidden", ToastKind::Info);
                } else {
                    self.update_grid(GridState::show_all_columns);
                    self.save_view_settings();
                    self.toast
                        .push(format!("Showing {hidden} hidden columns"), ToastKind::Info);
                }
            }
            PaletteCommand::ToggleFreezeColumn => {
                self.update_grid(GridState::toggle_frozen);
                self.save_view_settings();
            }
            PaletteCommand::InsertRow => self.start_insert_row(),
            PaletteCommand::DeleteRows => self.update(Message::DeleteRow),
            PaletteCommand::Undo => self.undo(),
            PaletteCommand::NextTab => self.cycle_tab(true),
            PaletteCommand::PrevTab => self.cycle_tab(false),
            PaletteCommand::CloseTab => {
                if let Some(index) = self.active_tab {
                    self.close_tab(index);
                }
            }
            PaletteCommand::ReloadSchema => self.spawn_schema_load(false),
            PaletteCommand::ToggleSidebar => self.toggle_sidebar(),
            PaletteCommand::ToggleReadonly => self.toggle_readonly(),
            PaletteCommand::Help => self.toggle_help(),
            PaletteCommand::Quit => self.should_quit = true,
            PaletteCommand::SwitchTable(name) => self.open_table(name),
        }
    }

    fn toggle_readonly(&mut self) {
        if self.readonly_locked {
            self.toast.push(
                "Opened with --readonly; restart without it to edit",
                ToastKind::Error,
            );
            return;
        }
        self.readonly = !self.readonly;
        let message = if self.readonly {
            "Read-only mode on"
        } else {
            "Read-only mode off"
        };
        self.toast.push(message, ToastKind::Info);
    }

    pub(crate) fn toggle_sidebar(&mut self) {
        self.sidebar_visible = !self.sidebar_visible;
        if !self.sidebar_visible {
            self.focus = FocusPane::Grid;
        }
    }

    /// Opens help, or closes it when it is showing. Help stacks over an open
    /// popup, which comes back when help closes.
    fn toggle_help(&mut self) {
        if matches!(self.popup, Some(PopupKind::Help(_))) {
            self.finish_popup();
        } else {
            self.push_popup(PopupKind::Help(HelpState::new()));
        }
    }

    /// Whether writes to the current view are refused, by read-only mode or
    /// because the view cannot be edited (a view, or a table without rowid).
    pub fn is_readonly_view(&self) -> bool {
        self.readonly || self.grid.as_ref().is_some_and(|grid| grid.readonly)
    }

    fn ensure_writable(&mut self) -> bool {
        if self.readonly {
            let message = if self.readonly_locked {
                "Opened with --readonly; writes are disabled"
            } else {
                "Read-only mode is on; turn it off in the command palette"
            };
            self.toast.push(message, ToastKind::Error);
            return false;
        }
        if let Some(grid) = self.grid.as_ref().filter(|grid| grid.readonly) {
            let message = if self
                .schema
                .relation(&grid.table_name)
                .is_some_and(|meta| meta.is_view)
            {
                "Views are read-only"
            } else {
                "This table has no rowid, so its rows cannot be edited safely"
            };
            self.toast.push(message, ToastKind::Error);
            return false;
        }
        true
    }

    fn copy_to_clipboard(&mut self, text: &str, success_message: &str) {
        let osc52 = format!("\x1b]52;c;{}\x07", base64_encode(text.as_bytes()));
        // Tests must not set the clipboard of the terminal running them.
        #[cfg(not(test))]
        {
            use std::io::Write;
            let _ = std::io::stdout().write_all(osc52.as_bytes());
            let _ = std::io::stdout().flush();
        }
        #[cfg(test)]
        let _ = osc52;
        self.toast.push(success_message, ToastKind::Success);
    }

    /// Applies a focus or scroll change to the grid and fetches the rows it
    /// brought into view.
    fn update_grid(&mut self, change: impl FnOnce(&mut GridState)) {
        if let Some(grid) = self.grid.as_mut() {
            change(grid);
            self.fetch_window_if_needed();
        }
        self.dirty = true;
    }

    fn scroll_grid_down(&mut self, n: usize) {
        self.update_grid(|grid| {
            grid.commit_row_selection();
            grid.scroll_down(n);
        });
    }

    fn scroll_grid_up(&mut self, n: usize) {
        self.update_grid(|grid| {
            grid.commit_row_selection();
            grid.scroll_up(n);
        });
    }

    fn scroll_grid_to_row(&mut self, row: i64) {
        self.update_grid(|grid| {
            grid.commit_row_selection();
            grid.scroll_to_row(row);
        });
    }

    fn change_sort(&mut self, add_key: bool) {
        self.next_navigation_request();
        let Some(grid) = self.grid.as_mut() else {
            return;
        };
        let col = grid.focused_col;
        if add_key {
            grid.add_sort_key(col);
        } else {
            grid.cycle_sort(col);
        }
        grid.reset_to_top();
        self.save_view_settings();
        self.fetch_window_around_focus();
        self.dirty = true;
    }

    fn jump_to_letter(&mut self, letter: char) {
        let view = match self.grid.as_ref() {
            Some(grid) if grid.is_text_sorted() => grid.view_query(),
            Some(_) => {
                self.toast
                    .push("Sort a text column (s) to jump by letter", ToastKind::Info);
                return;
            }
            None => return,
        };
        let view = match view {
            Ok(view) => view,
            Err(error) => {
                self.toast.push(error.to_string(), ToastKind::Error);
                return;
            }
        };
        let request_id = self.next_navigation_request();
        let table = view.table.clone();
        self.spawn_db(
            move |conn| db::count_rows_before_letter(conn, &view, letter),
            move |result| match result {
                Ok(offset) => Message::JumpToSortedOffset {
                    request_id,
                    table,
                    offset,
                },
                Err(error) => Message::NavigationFailed { request_id, error },
            },
        );
    }

    fn ensure_inline_insert_visible(grid: &mut GridState, insert_position: usize) {
        let viewport_rows = grid.window.viewport_rows.max(1) as i64;
        let insert_position = insert_position as i64;
        let current_display_start = if insert_position < grid.viewport_start {
            grid.viewport_start + 1
        } else {
            grid.viewport_start
        };

        let target_display_start = if insert_position < current_display_start {
            insert_position
        } else if insert_position >= current_display_start + viewport_rows {
            insert_position - viewport_rows + 1
        } else {
            current_display_start
        };

        let target_real_start = if insert_position < target_display_start {
            target_display_start - 1
        } else {
            target_display_start
        };
        let max_start = (grid.window.total_rows - viewport_rows + 1).max(0);
        grid.viewport_start = target_real_start.clamp(0, max_start);
    }

    fn start_insert_row(&mut self) {
        self.dirty = true;
        if !self.ensure_writable() {
            return;
        }
        let Some(grid) = self.grid.as_mut() else {
            return;
        };
        let insert_position = if grid.window.total_rows <= 0 {
            0
        } else {
            (grid.focused_row + 1).min(grid.window.total_rows as usize)
        };
        grid.clear_row_selection();
        let state = InsertRowState::new(
            grid.table_name.clone(),
            grid.columns.clone(),
            insert_position,
        );
        grid.focus_cell(grid.focused_row, state.selected);
        Self::ensure_inline_insert_visible(grid, insert_position);
        self.open_popup(PopupKind::InsertRow(state));
    }

    fn commit_insert_row(&mut self) {
        self.dirty = true;
        if !self.ensure_writable() {
            return;
        }
        let insert_spec = match &self.popup {
            Some(PopupKind::InsertRow(state)) => match state.build_insert_values() {
                Ok(values) => Some((state.table.clone(), values)),
                Err(err) => {
                    self.toast.push(err.to_string(), ToastKind::Error);
                    None
                }
            },
            _ => None,
        };
        let Some((table, values)) = insert_spec else {
            return;
        };
        if !self.begin_write() {
            return;
        }
        let job_table = table.clone();
        self.spawn_db(
            move |conn| db::write::insert_row(conn, &job_table, &values),
            move |result| match result {
                Ok(rowid) => Message::RowInserted { table, rowid },
                Err(error) => Message::EditFailed(error),
            },
        );
    }

    fn commit_edit(&mut self) {
        self.dirty = true;
        if !self.ensure_writable() {
            self.finish_popup();
            return;
        }
        if let Some(PopupKind::TextEditor(state)) = self.popup.as_ref() {
            if !state.valid {
                self.toast.push(
                    format!("Invalid value for {}", state.col_type),
                    ToastKind::Error,
                );
                return;
            }
        }
        let write_info = self.popup.as_ref().and_then(|p| match p {
            PopupKind::TextEditor(s) => Some((
                s.table.clone(),
                s.col_name.clone(),
                s.rowid,
                s.as_sql_value().ok()?,
                s.original.clone(),
            )),
            PopupKind::ValuePicker(s) => s.selected_sql_value().map(|v| {
                (
                    s.table.clone(),
                    s.col_name.clone(),
                    s.rowid,
                    v,
                    s.original.clone(),
                )
            }),
            PopupKind::DatePicker(s) => Some((
                s.table.clone(),
                s.col_name.clone(),
                s.rowid,
                s.as_sql_value(),
                s.original.clone(),
            )),
            PopupKind::FkPicker(s) => s.selected_value().map(|v| {
                (
                    s.source_table.clone(),
                    s.source_col.clone(),
                    s.source_rowid,
                    v,
                    s.original.clone(),
                )
            }),
            _ => None,
        });
        match write_info {
            Some((table, col, rowid, value, original)) => {
                self.submit_cell_edit(table, col, rowid, value, original);
            }
            None => self
                .toast
                .push("No value selected to save", ToastKind::Error),
        }
    }

    fn undo(&mut self) {
        self.dirty = true;
        if self.readonly {
            self.toast.push("Read-only: cannot undo", ToastKind::Error);
            return;
        }
        if self.undo_stack.is_empty() {
            self.toast.push("Nothing to undo", ToastKind::Info);
            return;
        }
        if !self.begin_write() {
            return;
        }
        let Some(frame) = self.undo_stack.pop_back() else {
            return;
        };
        let message = match &frame.op {
            UndoOp::Update => format!("Undo: restored row {}", frame.rowid),
            UndoOp::Insert => format!("Undo: deleted inserted row {}", frame.rowid),
            UndoOp::Delete => format!("Undo: restored deleted row {}", frame.rowid),
        };
        let work = frame.clone();
        self.spawn_db(
            move |conn| match work.op {
                UndoOp::Update => {
                    for (column, value) in &work.cols {
                        db::write::commit_cell_edit(conn, &work.table, column, work.rowid, value)?;
                    }
                    Ok(())
                }
                UndoOp::Insert => db::write::delete_row(conn, &work.table, work.rowid),
                UndoOp::Delete => {
                    db::write::reinsert_row(conn, &work.table, work.rowid, &work.cols)
                }
            },
            move |result| match result {
                Ok(()) => Message::UndoCompleted {
                    table: frame.table,
                    message,
                },
                Err(error) => Message::UndoFailed { frame, error },
            },
        );
    }

    fn focused_cell_context(&mut self) -> Option<FocusedCellContext> {
        let grid = self.grid.as_ref()?;
        let col_idx = grid.focused_col;
        let col = grid.columns.get(col_idx)?.clone();
        if !col.writable {
            self.toast
                .push("Generated columns are read-only", ToastKind::Error);
            return None;
        }
        let table_name = grid.table_name.clone();
        let abs_row = grid.focused_row as i64;
        let cell_value = grid
            .window
            .get_row(abs_row)
            .and_then(|row| row.get(col_idx))
            .cloned()?;
        let is_fk = grid.fk_cols.get(col_idx).copied().unwrap_or(false);
        let rowid = grid.window.get_rowid(abs_row).or_else(|| {
            self.toast.push(
                "Row is not loaded or cannot be edited safely",
                ToastKind::Error,
            );
            None
        })?;

        Some(FocusedCellContext {
            col,
            table_name,
            rowid,
            cell_value,
            is_fk,
        })
    }

    /// Whether the focus is still on the cell a background lookup was started for.
    fn focused_cell_is(&self, table: &str, rowid: i64, col_name: &str) -> bool {
        self.grid.as_ref().is_some_and(|grid| {
            grid.table_name == table
                && grid.window.get_rowid(grid.focused_row as i64) == Some(rowid)
                && grid
                    .columns
                    .get(grid.focused_col)
                    .is_some_and(|column| column.name == col_name)
        })
    }

    pub(crate) fn focused_cell_can_be_set_null(&self) -> bool {
        if self.is_readonly_view() {
            return false;
        }
        let Some(grid) = self.grid.as_ref() else {
            return false;
        };
        let col_idx = grid.focused_col;
        let Some(col) = grid.columns.get(col_idx) else {
            return false;
        };
        if col.not_null {
            return false;
        }
        grid.window
            .get_row(grid.focused_row as i64)
            .and_then(|row| row.get(col_idx))
            .is_some_and(|value| *value != SqlValue::Null)
    }

    /// Opens the editor that fits the column kind and current value. Plain
    /// values first look up the column's distinct values to decide on a value
    /// picker.
    fn open_cell_editor(&mut self, cell: FocusedCellContext) {
        let FocusedCellContext {
            col,
            table_name,
            rowid,
            cell_value: original,
            ..
        } = cell;
        let kind = ColumnKind::of(&col.col_type, &col.name);
        if matches!(kind, ColumnKind::Datetime | ColumnKind::EpochDatetime)
            || DatePickerState::supports_datetime(&original)
        {
            self.open_popup(PopupKind::DatePicker(DatePickerState::datetime(
                table_name, rowid, col.name, original,
            )));
        } else if kind == ColumnKind::Date || DatePickerState::supports_date(&original) {
            self.open_popup(PopupKind::DatePicker(DatePickerState::date(
                table_name, rowid, col.name, original,
            )));
        } else if matches!(&original, SqlValue::Blob(_)) || kind == ColumnKind::Blob {
            self.open_text_editor(table_name, rowid, col.name, col.col_type, original);
        } else {
            let request_id = self.next_popup_request();
            let (job_table, job_column) = (table_name.clone(), col.name.clone());
            self.spawn_db(
                move |conn| {
                    db::load_distinct_values(
                        conn,
                        &job_table,
                        &job_column,
                        VALUE_PICKER_DISTINCT_LIMIT + 1,
                    )
                },
                move |result| match result {
                    Ok(values) => Message::DistinctValuesReady {
                        request_id,
                        table: table_name,
                        rowid,
                        col,
                        original,
                        values,
                    },
                    Err(error) => Message::DistinctValuesFailed {
                        request_id,
                        table: table_name,
                        rowid,
                        col,
                        original,
                        error,
                    },
                },
            );
        }
    }

    fn jump_to_foreign_key(&mut self) {
        let jump = self.grid.as_ref().and_then(|g| {
            let col_idx = g.focused_col;
            if !g.fk_cols.get(col_idx).copied().unwrap_or(false) {
                return None;
            }
            let col = g.columns.get(col_idx)?;
            let fk = self.schema.table(&g.table_name)?.foreign_key(&col.name)?;
            let frame = JumpFrame {
                table: g.table_name.clone(),
                rowid: g.window.get_rowid(g.focused_row as i64)?,
                col: col_idx,
            };
            let value = g
                .window
                .get_row(g.focused_row as i64)?
                .get(col_idx)?
                .clone();
            Some((frame, fk.to_table.clone(), fk.to_col.clone(), value))
        });
        let Some((frame, to_table, to_col, value)) = jump else {
            self.toast
                .push("The focused cell is not a link", ToastKind::Info);
            return;
        };
        if value == SqlValue::Null {
            self.toast.push("The link is NULL", ToastKind::Info);
            return;
        }
        let request_id = self.next_navigation_request();
        let job_table = to_table.clone();
        self.spawn_db(
            move |conn| db::find_rowid_by_value(conn, &job_table, &to_col, &value),
            move |result| match result {
                Ok(Some(rowid)) => Message::FkJumpReady {
                    request_id,
                    frame,
                    table: to_table,
                    rowid,
                },
                Ok(None) => Message::NavigationFailed {
                    request_id,
                    error: "Referenced row was not found or has no navigable rowid".to_string(),
                },
                Err(error) => Message::NavigationFailed { request_id, error },
            },
        );
    }

    fn request_delete_confirmation(&mut self) {
        let Some(grid) = self.grid.as_ref() else {
            return;
        };
        let table = grid.table_name.clone();
        let view = match grid.view_query() {
            Ok(view) => view,
            Err(error) => {
                self.toast.push(error.to_string(), ToastKind::Error);
                return;
            }
        };
        match &grid.row_selection {
            RowSelection::None => {
                let Some(rowid) = grid.window.get_rowid(grid.focused_row as i64) else {
                    return;
                };
                let message = format!("Delete row #{}? [y/n]", grid.focused_row + 1);
                self.update(Message::ConfirmReady {
                    message,
                    kind: ConfirmKind::DeleteRow { table, rowid },
                });
            }
            RowSelection::Rows(_) => {
                let offsets = grid
                    .selected_rows()
                    .into_iter()
                    .map(|row| row as i64)
                    .collect::<Vec<_>>();
                if offsets.is_empty() {
                    return;
                }
                self.spawn_db(
                    move |conn| {
                        let rowids = db::fetch_rowids_at_offsets(conn, &view, &offsets)?;
                        if rowids.len() != offsets.len() {
                            anyhow::bail!("Some selected rows no longer exist");
                        }
                        Ok(rowids)
                    },
                    move |result| match result {
                        Ok(rowids) => {
                            let noun = if rowids.len() == 1 { "row" } else { "rows" };
                            Message::ConfirmReady {
                                message: format!("Delete {} selected {noun}? [y/n]", rowids.len()),
                                kind: ConfirmKind::DeleteSelectedRows { table, rowids },
                            }
                        }
                        Err(error) => Message::Notify(error, ToastKind::Error),
                    },
                );
            }
            RowSelection::All { except } => {
                let except = except.iter().map(|&row| row as i64).collect::<Vec<_>>();
                let filtered = !grid.filter.is_empty();
                let columns = grid.columns.clone();
                self.spawn_db(
                    move |conn| delete_all_confirmation(conn, &view, &columns, &except, filtered),
                    move |result| match result {
                        Ok(Some((message, kind))) => Message::ConfirmReady { message, kind },
                        Ok(None) => Message::Notify("Nothing to delete".into(), ToastKind::Info),
                        Err(error) => Message::Notify(error, ToastKind::Error),
                    },
                );
            }
        }
    }

    fn confirm_delete(&mut self) {
        self.dirty = true;
        let Some(confirm) = self.pending_confirm.take() else {
            return;
        };
        if !self.begin_write() {
            self.pending_confirm = Some(confirm);
            return;
        }
        match confirm.kind {
            ConfirmKind::DeleteRow { table, rowid } => {
                let columns = self
                    .schema
                    .table(&table)
                    .map(|meta| meta.columns.clone())
                    .unwrap_or_default();
                let job_table = table.clone();
                self.spawn_db(
                    move |conn| {
                        db::write::delete_row_with_backup(conn, &job_table, &columns, rowid)
                    },
                    move |result| match result {
                        Ok(cols) => Message::RowDeleted { table, rowid, cols },
                        Err(error) => Message::EditFailed(error),
                    },
                );
            }
            ConfirmKind::DeleteSelectedRows { table, rowids } => {
                let job_table = table.clone();
                self.spawn_db(
                    move |conn| db::write::delete_rows_by_rowids(conn, &job_table, &rowids),
                    move |result| match result {
                        Ok(count) => Message::RowsDeleted { table, count },
                        Err(error) => Message::EditFailed(error),
                    },
                );
            }
            ConfirmKind::ClearTable { table, keep } => {
                let job_table = table.clone();
                self.spawn_db(
                    move |conn| db::write::clear_table(conn, &job_table, &keep),
                    move |result| match result {
                        Ok(count) => Message::RowsDeleted { table, count },
                        Err(error) => Message::EditFailed(error),
                    },
                );
            }
        }
    }

    /// Runs blocking database work off the UI thread and reports its outcome,
    /// including connection and task failures, as exactly one message.
    fn spawn_db<T: Send + 'static>(
        &self,
        work: impl FnOnce(&rusqlite::Connection) -> anyhow::Result<T> + Send + 'static,
        reply: impl FnOnce(Result<T, String>) -> Message + Send + 'static,
    ) {
        let pool = Arc::clone(&self.pool);
        let tx = self.tx.clone();
        tokio::task::spawn(async move {
            let result = tokio::task::spawn_blocking(move || work(&*pool.get()?)).await;
            let result = match result {
                Ok(result) => result.map_err(|error| error.to_string()),
                Err(error) => Err(error.to_string()),
            };
            let _ = tx.send(reply(result));
        });
    }

    fn spawn_schema_load(&self, external: bool) {
        self.spawn_db(db::load_schema, move |result| match result {
            Ok(schema) if external => Message::ExternalRefresh(schema),
            Ok(schema) => Message::SchemaReady(schema),
            Err(error) => Message::SchemaLoadFailed { external, error },
        });
    }

    /// Completes an external change check, re-running it if the file changed again
    /// while it was in flight.
    fn finish_file_check(&mut self) {
        self.file_check_in_flight = false;
        if self.file_change_pending {
            self.file_change_pending = false;
            let _ = self.tx.send(Message::FileChanged);
        }
    }

    fn clamp_sidebar_selection(&mut self) {
        let total = self.sidebar.visible_count(&self.schema);
        if self.sidebar.selected >= total {
            self.sidebar.selected = total.saturating_sub(1);
        }
    }

    /// Starts a new grid read, making every older read response stale.
    fn next_grid_request(&mut self) -> u64 {
        self.grid_request_serial = self.grid_request_serial.wrapping_add(1);
        self.grid_request_serial
    }

    fn invalidate_grid_requests(&mut self) {
        self.next_grid_request();
    }

    fn next_navigation_request(&mut self) -> u64 {
        self.navigation_request_serial = self.navigation_request_serial.wrapping_add(1);
        self.navigation_request_serial
    }

    /// Makes lookups started for the current popup stale and returns the id
    /// for a new one.
    fn next_popup_request(&mut self) -> u64 {
        self.popup_request_serial = self.popup_request_serial.wrapping_add(1);
        self.popup_request_serial
    }

    /// Shows `popup` in place of any open popup and returns the request id for
    /// loading its content.
    fn open_popup(&mut self, popup: PopupKind) -> u64 {
        self.popup_stack.clear();
        self.popup = Some(popup);
        self.mode = AppMode::Edit;
        self.next_popup_request()
    }

    /// Shows `popup` over the open one, which returns when `popup` closes.
    fn push_popup(&mut self, popup: PopupKind) -> u64 {
        if let Some(current) = self.popup.take() {
            self.popup_stack.push(current);
        }
        self.popup = Some(popup);
        self.mode = AppMode::Edit;
        self.next_popup_request()
    }

    /// Admits one database write at a time; its completion message releases the gate.
    fn begin_write(&mut self) -> bool {
        if self.write_in_flight {
            self.toast
                .push("A database write is already running", ToastKind::Info);
            return false;
        }
        self.write_in_flight = true;
        true
    }

    fn push_undo(&mut self, frame: UndoFrame) {
        if self.undo_stack.len() == UNDO_HISTORY_LIMIT {
            self.undo_stack.pop_front();
        }
        self.undo_stack.push_back(frame);
    }

    /// Closes the top popup, restoring a popup suspended under it. Returns
    /// whether a deferred external refresh ran.
    fn finish_popup(&mut self) -> bool {
        self.next_popup_request();
        self.popup = self.popup_stack.pop();
        if self.popup.is_some() {
            return false;
        }
        self.mode = AppMode::Browse;
        if let Some(grid) = self.grid.as_mut() {
            grid.clamp_to_total_rows();
        }
        if self.pending_external_refresh {
            self.pending_external_refresh = false;
            self.refresh_active_grid_schema();
            true
        } else {
            false
        }
    }

    fn open_text_editor(
        &mut self,
        table: String,
        rowid: i64,
        col_name: String,
        col_type: String,
        original: SqlValue,
    ) {
        let readonly = self.is_readonly_view();
        self.open_popup(PopupKind::TextEditor(TextEditorState::new(
            table, rowid, col_name, col_type, original, readonly,
        )));
    }

    fn submit_cell_edit(
        &mut self,
        table: String,
        col: String,
        rowid: i64,
        value: SqlValue,
        original: SqlValue,
    ) {
        self.dirty = true;
        if !self.begin_write() {
            return;
        }
        let (job_table, job_col) = (table.clone(), col.clone());
        self.spawn_db(
            move |conn| db::write::commit_cell_edit(conn, &job_table, &job_col, rowid, &value),
            move |result| match result {
                Ok(()) => Message::EditCommitted {
                    rowid,
                    table,
                    col,
                    original,
                },
                Err(error) => Message::EditFailed(error),
            },
        );
    }

    pub(crate) fn active_table_name(&self) -> Option<String> {
        self.active_tab
            .and_then(|index| self.open_tabs.get(index))
            .map(|tab| tab.table_name.clone())
    }

    /// The active grid, when it shows `table`.
    fn grid_for(&mut self, table: &str) -> Option<&mut GridState> {
        self.grid.as_mut().filter(|grid| grid.table_name == table)
    }

    // ── tabs ─────────────────────────────────────────────────────────────

    fn open_table(&mut self, name: String) {
        if self.schema.relation(&name).is_none() {
            self.toast
                .push(format!("{name:?} does not exist"), ToastKind::Error);
            return;
        }
        self.focus = FocusPane::Grid;
        match self.open_tabs.iter().position(|t| t.table_name == name) {
            Some(index) if Some(index) == self.active_tab && self.grid.is_some() => {}
            Some(index) => self.activate_tab(index),
            None => {
                self.stash_active_grid();
                self.open_tabs.push(TableTab::new(name.clone()));
                self.active_tab = Some(self.open_tabs.len() - 1);
                self.request_table_view(&name);
            }
        }
        self.dirty = true;
    }

    /// Opens `table` and focuses the row with `rowid` once it is loaded.
    fn open_table_at(&mut self, table: String, rowid: i64, col: Option<usize>) {
        self.open_table(table.clone());
        self.jump_to_rowid(table, rowid, col);
    }

    /// Keeps the active grid in its tab so switching back restores it.
    fn stash_active_grid(&mut self) {
        let (Some(index), Some(grid)) = (self.active_tab, self.grid.take()) else {
            return;
        };
        if let Some(tab) = self.open_tabs.get_mut(index) {
            if tab.table_name == grid.table_name {
                tab.saved = Some(grid);
            }
        }
    }

    fn activate_tab(&mut self, idx: usize) {
        if idx >= self.open_tabs.len() {
            return;
        }
        self.focus = FocusPane::Grid;
        if self.active_tab == Some(idx) && self.grid.is_some() {
            return;
        }
        self.stash_active_grid();
        self.active_tab = Some(idx);
        self.show_active_tab();
        self.dirty = true;
    }

    fn cycle_tab(&mut self, forward: bool) {
        let count = self.open_tabs.len();
        let Some(active) = self.active_tab.filter(|_| count > 1) else {
            return;
        };
        let next = if forward {
            (active + 1) % count
        } else {
            (active + count - 1) % count
        };
        self.activate_tab(next);
    }

    /// Shows the active tab's saved grid, refreshing its rows, or loads it.
    fn show_active_tab(&mut self) {
        let Some(index) = self.active_tab else {
            self.grid = None;
            return;
        };
        let name = self.open_tabs[index].table_name.clone();
        match self.open_tabs[index].saved.take() {
            Some(grid) if self.schema.relation(&name).is_some() => {
                self.next_navigation_request();
                self.invalidate_grid_requests();
                self.grid = Some(grid);
                self.refresh_active_grid_schema();
            }
            _ => self.request_table_view(&name),
        }
    }

    fn close_tab(&mut self, idx: usize) {
        if idx >= self.open_tabs.len() {
            return;
        }
        let closing_active = self.active_tab == Some(idx);
        let next_active =
            next_active_tab_after_close(self.active_tab, idx, self.open_tabs.len() - 1);
        self.open_tabs.remove(idx);
        self.active_tab = next_active;
        if closing_active {
            self.grid = None;
            self.show_active_tab();
        }
        if self.active_tab.is_none() {
            self.focus = FocusPane::Sidebar;
            self.sidebar_visible = true;
        }
        self.dirty = true;
    }

    // ── grid loading ─────────────────────────────────────────────────────

    /// Builds the grid for a table or view from the schema, applies its saved
    /// view settings and starts loading rows and enum colours.
    fn request_table_view(&mut self, name: &str) {
        self.next_navigation_request();
        self.invalidate_grid_requests();
        self.grid = None;
        let Some(meta) = self.schema.relation(name) else {
            self.toast
                .push(format!("{name:?} no longer exists"), ToastKind::Error);
            return;
        };
        let columns = meta.columns.clone();
        let mut grid = GridState::new(GridInit {
            table_name: name.to_string(),
            fk_cols: meta.foreign_key_flags(),
            enumerated_values: Vec::new(),
            columns: columns.clone(),
            rows: Vec::new(),
            width_sample_rows: Vec::new(),
            total_rows: 0,
            area_width: self.grid_inner_area.map_or(0, |area| area.width),
        });
        grid.readonly = meta.is_view || !meta.has_mutable_rowid();
        grid.apply_settings(view_settings::load(&self.db_path, name));
        grid.count_known = false;
        self.grid = Some(grid);
        self.fetch_window_around_focus();

        let table = name.to_string();
        let job_table = table.clone();
        self.spawn_db(
            move |conn| db::enum_value_sets(conn, &job_table, &columns),
            move |result| match result {
                Ok(sets) => Message::EnumValuesReady { table, sets },
                // Colours are a nicety; a failed scan leaves values uncoloured.
                Err(_) => Message::EnumValuesReady {
                    table,
                    sets: Vec::new(),
                },
            },
        );
    }

    /// Brings the active grid in line with the schema, keeping its focus,
    /// sort, filters and column settings for columns that still exist, and
    /// reloads its rows.
    fn refresh_active_grid_schema(&mut self) {
        self.next_navigation_request();
        let Some(table) = self.grid.as_ref().map(|grid| grid.table_name.clone()) else {
            if let Some(table) = self.active_table_name() {
                self.request_table_view(&table);
            }
            return;
        };
        let Some(meta) = self.schema.relation(&table) else {
            self.invalidate_grid_requests();
            self.grid = None;
            self.toast
                .push(format!("{table:?} no longer exists"), ToastKind::Error);
            return;
        };
        let columns = meta.columns.clone();
        let fk_cols = meta.foreign_key_flags();
        let readonly = meta.is_view || !meta.has_mutable_rowid();
        let Some(grid) = self.grid.as_mut() else {
            return;
        };
        if grid.columns != columns || grid.fk_cols != fk_cols {
            grid.set_columns(columns, fk_cols);
        }
        grid.readonly = readonly;
        self.refresh_grid_rows();
    }

    /// Reloads the visible rows and the row count, keeping the rows on screen
    /// until the new ones arrive.
    fn refresh_grid_rows(&mut self) {
        if let Some(grid) = self.grid.as_mut() {
            grid.count_known = false;
            grid.window.fetch_in_flight = false;
            self.fetch_window_around_focus();
        }
    }

    /// Starts a window fetch around the focused row unless one is already running.
    fn fetch_window_if_needed(&mut self) {
        if self
            .grid
            .as_ref()
            .is_some_and(|grid| grid.needs_fetch && !grid.window.fetch_in_flight)
        {
            self.fetch_window_around_focus();
        }
    }

    /// Reads the window around the focused row, superseding any read in
    /// flight. The row count is only recounted when the view changed.
    fn fetch_window_around_focus(&mut self) {
        let Some(grid) = self.grid.as_mut() else {
            return;
        };
        grid.needs_fetch = false;
        let view = match grid.view_query() {
            Ok(view) => view,
            Err(error) => {
                grid.window.fetch_in_flight = false;
                self.toast.push(error.to_string(), ToastKind::Error);
                return;
            }
        };
        grid.window.fetch_in_flight = true;
        let (offset, limit) = grid.window.fetch_params(grid.focused_row as i64);
        let recount = !grid.count_known;
        let columns = grid.columns.clone();
        let table = view.table.clone();
        let request_id = self.next_grid_request();
        self.spawn_db(
            move |conn| {
                let total = if recount {
                    Some(db::count_rows(conn, &view)?)
                } else {
                    None
                };
                let fetched = db::fetch_rows(conn, &view, &columns, offset, limit)?;
                Ok((fetched, total))
            },
            move |result| match result {
                Ok((fetched, total_rows)) => Message::WindowReady {
                    request_id,
                    table,
                    offset,
                    rows: fetched.rows,
                    rowids: fetched.rowids,
                    total_rows,
                },
                Err(error) => Message::GridReadFailed {
                    request_id,
                    table,
                    error,
                },
            },
        );
        self.dirty = true;
    }

    fn on_window_ready(
        &mut self,
        table: String,
        offset: i64,
        rows: Vec<Vec<SqlValue>>,
        rowids: Vec<Option<i64>>,
        total_rows: Option<i64>,
    ) {
        let counted = total_rows.is_some();
        if let Some(grid) = self.grid_for(&table) {
            if grid.width_sample_rows.is_empty() && !rows.is_empty() {
                grid.set_width_sample(rows.clone());
            }
            grid.window.offset = offset;
            grid.window.rows = rows;
            grid.window.rowids = rowids;
            if let Some(total) = total_rows {
                grid.window.total_rows = total;
                grid.count_known = true;
            }
            grid.window.fetch_in_flight = false;
            grid.load_error = None;
            grid.clamp_to_total_rows();
            grid.needs_fetch = false;
            grid.check_needs_fetch();
        }
        if counted {
            if let Some(target) = self
                .pending_jump_target
                .clone()
                .filter(|t| t.table == table)
            {
                self.pending_jump_target = None;
                self.jump_to_rowid(target.table, target.rowid, target.col);
            }
        }
        self.dirty = true;
    }

    /// Focuses the row with `rowid` in `table`'s current view. Waits for the
    /// table to be open and counted when it is not yet.
    fn jump_to_rowid(&mut self, table: String, rowid: i64, col: Option<usize>) {
        let ready = self
            .grid
            .as_ref()
            .filter(|grid| grid.table_name == table && grid.count_known)
            .map(GridState::view_query);
        let view = match ready {
            None => {
                self.pending_jump_target = Some(PendingJumpTarget { table, rowid, col });
                return;
            }
            Some(Err(error)) => {
                self.toast.push(error.to_string(), ToastKind::Error);
                return;
            }
            Some(Ok(view)) => view,
        };
        let request_id = self.next_navigation_request();
        self.spawn_db(
            move |conn| db::fetch_offset_for_rowid(conn, &view, rowid),
            move |result| match result {
                Ok(offset) => Message::JumpOffsetReady {
                    request_id,
                    table,
                    offset,
                    col,
                },
                Err(error) => Message::NavigationFailed { request_id, error },
            },
        );
    }

    fn save_view_settings(&mut self) {
        let Some(grid) = self.grid.as_ref() else {
            return;
        };
        if let Err(error) = view_settings::save(&grid.settings(), &self.db_path, &grid.table_name) {
            self.toast.push(
                format!("Could not save the view settings: {error}"),
                ToastKind::Error,
            );
        }
    }

    fn apply_column_filter(&mut self, col_name: String, col_filter: crate::filter::ColumnFilter) {
        self.next_navigation_request();
        if let Some(grid) = self.grid.as_mut() {
            if !col_filter.rules.is_empty() {
                grid.filter.columns.insert(col_name, col_filter);
            } else {
                grid.filter.columns.remove(&col_name);
            }
            grid.reset_to_top();
            self.save_view_settings();
            self.fetch_window_around_focus();
        }
        self.dirty = true;
    }
}

/// The confirmation for deleting an all-rows selection. Without filters the
/// table is cleared except the deselected rows; with filters only the rows of
/// the filtered view are deleted.
fn delete_all_confirmation(
    conn: &rusqlite::Connection,
    view: &ViewQuery,
    columns: &[Column],
    except: &[i64],
    filtered: bool,
) -> anyhow::Result<Option<(String, ConfirmKind)>> {
    let table = view.table.clone();
    if filtered {
        let except: std::collections::BTreeSet<i64> = except.iter().copied().collect();
        let fetched = db::fetch_rows(conn, view, columns, 0, i64::MAX)?;
        let rowids: Vec<i64> = fetched
            .rowids
            .into_iter()
            .enumerate()
            .filter(|(offset, _)| !except.contains(&(*offset as i64)))
            .filter_map(|(_, rowid)| rowid)
            .collect();
        if rowids.is_empty() {
            return Ok(None);
        }
        let message = format!(
            "Delete the {} rows matching the filters? [y/n]",
            rowids.len()
        );
        return Ok(Some((
            message,
            ConfirmKind::DeleteSelectedRows { table, rowids },
        )));
    }
    let keep = db::fetch_rowids_at_offsets(conn, view, except)?;
    let total_rows = db::count_rows(conn, &ViewQuery::table(&table))?;
    let deleting = total_rows - keep.len() as i64;
    if deleting <= 0 {
        return Ok(None);
    }
    let message = if keep.is_empty() {
        format!("Delete all {total_rows} rows from {table}? [y/n]")
    } else {
        format!(
            "Delete {deleting} rows from {table}, keeping {} deselected? [y/n]",
            keep.len()
        )
    };
    Ok(Some((message, ConfirmKind::ClearTable { table, keep })))
}

fn base64_encode(data: &[u8]) -> String {
    const CHARS: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::new();
    for chunk in data.chunks(3) {
        let b0 = chunk[0];
        let b1 = chunk.get(1).copied().unwrap_or(0);
        let b2 = chunk.get(2).copied().unwrap_or(0);
        out.push(CHARS[(b0 >> 2) as usize] as char);
        out.push(CHARS[((b0 & 0x3) << 4 | (b1 >> 4)) as usize] as char);
        if chunk.len() > 1 {
            out.push(CHARS[((b1 & 0xf) << 2 | (b2 >> 6)) as usize] as char);
        } else {
            out.push('=');
        }
        if chunk.len() > 2 {
            out.push(CHARS[(b2 & 0x3f) as usize] as char);
        } else {
            out.push('=');
        }
    }
    out
}

fn should_use_value_picker(values: &[String]) -> bool {
    !values.is_empty() && values.len() <= VALUE_PICKER_DISTINCT_LIMIT
}

fn next_active_tab_after_close(
    active_tab: Option<usize>,
    closed_idx: usize,
    remaining_tabs: usize,
) -> Option<usize> {
    if remaining_tabs == 0 {
        return None;
    }

    match active_tab {
        Some(active_idx) if active_idx == closed_idx => {
            Some(closed_idx.saturating_sub(1).min(remaining_tabs - 1))
        }
        Some(active_idx) if closed_idx < active_idx => Some(active_idx - 1),
        Some(active_idx) => Some(active_idx.min(remaining_tabs - 1)),
        None => None,
    }
}

#[cfg(test)]
mod tests;
