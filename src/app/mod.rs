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
        types::{affinity, temporal_kind, ColAffinity, SqlValue, TemporalKind},
        DbPool,
    },
    export::ExportFormat,
    grid::{GridState, RowSelection, SortDir, SortSpec},
    symbols::Symbols,
    theme::Theme,
    ui::{
        popup::{
            CommandPaletteState, DatePickerState, FilterPopupState, FindState, FkPickerState,
            HelpState, InsertRowState, PaletteCommand, PopupKind, TextEditorState,
        },
        sidebar::SidebarState,
        toast::{ToastKind, ToastState},
    },
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

pub struct TableTab {
    pub table_name: String,
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
    grab_offset: i64,
}

const VALUE_PICKER_DISTINCT_LIMIT: usize = 100;
const ENUM_COLOR_DISTINCT_LIMIT: usize = 20;
const UNDO_HISTORY_LIMIT: usize = 100;
/// Rows loaded when a table opens; they also seed column widths and enum colors.
const INITIAL_ROW_LIMIT: i64 = 50;
const FK_PICKER_ROW_LIMIT: i64 = 200;
const FIND_ROW_LIMIT: i64 = 10_000;
const CONFIRM_TIMEOUT_SECS: u64 = 5;

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
    pub grid: Option<crate::grid::GridState>,
    pub mode: AppMode,
    pub popup: Option<PopupKind>,
    pub toast: ToastState,
    pub readonly: bool,
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
}

#[derive(Debug)]
pub enum Message {
    Quit,
    Key(crossterm::event::KeyEvent),
    Mouse(crossterm::event::MouseEvent),
    Resize,
    Tick,
    OpenTable(String),
    CloseTab(usize),
    ActivateTab(usize),
    GridDataReady {
        request_id: u64,
        table: String,
        columns: Vec<Column>,
        fk_cols: Vec<bool>,
        fetched: db::FetchedRows,
        total_rows: i64,
    },
    WindowReady {
        request_id: u64,
        table: String,
        offset: i64,
        rows: Vec<Vec<SqlValue>>,
        rowids: Vec<Option<i64>>,
        total_rows: i64,
    },
    GridReadFailed {
        request_id: u64,
        table: String,
        error: String,
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
    JumpToFk,
    FkRowsReady {
        request_id: u64,
        target_table: String,
        rows: Vec<Vec<SqlValue>>,
    },
    FkRowsFailed {
        request_id: u64,
        target_table: String,
        error: String,
    },
    JumpBack,
    JumpToTargetRow {
        table: String,
        rowid: i64,
        col: Option<usize>,
    },
    CycleSort,
    JumpToLetter(char),
    JumpToSortedOffset {
        request_id: u64,
        table: String,
        offset: i64,
    },
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
    OpenFilterPopup,
    ClearFilters,
    InsertRow,
    DeleteRow,
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
    ExecuteCommand(PaletteCommand),
    ExportDone {
        path: String,
        count: u64,
    },
    ExportFailed(String),
    ReloadSchema,
    SchemaReady(Schema),
    SchemaLoadFailed {
        external: bool,
        error: String,
    },
    CopyCell,
    CopyRowJson,
    FileChanged,
    ExternalRefresh(Schema),
    OpenFind,
    FindReady {
        request_id: u64,
        table: String,
        rows: Vec<Vec<SqlValue>>,
    },
    FindFailed {
        request_id: u64,
        table: String,
        error: String,
    },
    CommitFind,
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
            toast: ToastState::new(),
            readonly,
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
            Message::Tick => {
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
                    self.dirty = true;
                }
            }
            Message::OpenTable(name) => self.open_table(name),
            Message::CloseTab(idx) => self.close_tab(idx),
            Message::ActivateTab(idx) => self.activate_tab(idx),
            Message::GridDataReady {
                request_id,
                table,
                columns,
                fk_cols,
                fetched,
                total_rows,
            } => {
                if request_id != self.grid_request_serial {
                    return;
                }
                self.on_grid_data_ready(table.clone(), columns, fk_cols, fetched, total_rows);
                if let Some(pending_target) = self.pending_jump_target.clone() {
                    if pending_target.table == table {
                        self.pending_jump_target = None;
                        self.jump_to_rowid(table, pending_target.rowid, pending_target.col);
                    }
                }
            }
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
                if let Some(grid) = self.grid.as_mut().filter(|grid| grid.table_name == table) {
                    let fetch_was_queued = grid.needs_fetch;
                    grid.window.offset = offset;
                    grid.window.rows = rows;
                    grid.window.rowids = rowids;
                    grid.window.total_rows = total_rows;
                    grid.window.fetch_in_flight = false;
                    grid.clamp_to_total_rows();
                    grid.needs_fetch =
                        fetch_was_queued && grid.window.needs_prefetch(grid.focused_row as i64);
                }
                self.dirty = true;
            }
            Message::GridReadFailed {
                request_id,
                table,
                error,
            } => {
                if request_id == self.grid_request_serial {
                    if let Some(grid) = self.grid.as_mut().filter(|grid| grid.table_name == table) {
                        grid.window.fetch_in_flight = false;
                        grid.needs_fetch = false;
                    }
                    self.toast.push(error, ToastKind::Error);
                    self.dirty = true;
                }
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
                if !self.ensure_writable() {
                    return;
                }
                self.popup_request_serial = self.popup_request_serial.wrapping_add(1);
                if let Some(cell) = self.focused_cell_context() {
                    if !(cell.is_fk && self.open_fk_picker(&cell)) {
                        self.open_cell_editor(cell);
                    }
                }
                self.dirty = true;
            }
            Message::OpenDirectEdit => {
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
                self.dirty = true;
            }
            Message::SetFocusedCellNull => {
                if !self.ensure_writable() {
                    return;
                }
                if let Some(cell) = self.focused_cell_context() {
                    if cell.col.not_null {
                        self.toast.push("Column is NOT NULL", ToastKind::Error);
                    } else if cell.cell_value == SqlValue::Null {
                        self.toast.push("Cell is already NULL", ToastKind::Error);
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
                self.dirty = true;
            }
            Message::ClosePopup => {
                self.finish_popup();
                self.dirty = true;
            }
            Message::CommitEdit => {
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
                    PopupKind::FkPicker(s) => s.selected_value().cloned().map(|v| {
                        (
                            s.source_table.clone(),
                            s.source_col.clone(),
                            s.source_rowid,
                            v,
                            s.original.clone(),
                        )
                    }),
                    PopupKind::InsertRow(_)
                    | PopupKind::FilterPopup(_)
                    | PopupKind::CommandPalette(_)
                    | PopupKind::Help(_)
                    | PopupKind::Find(_) => None,
                });
                if let Some((table, col, rowid, value, original)) = write_info {
                    self.submit_cell_edit(table, col, rowid, value, original);
                } else {
                    self.toast
                        .push("No value selected to save", ToastKind::Error);
                }
            }
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
                    if let Some(grid) = self.grid.as_mut() {
                        grid.window.rows.clear();
                        grid.window.rowids.clear();
                    }
                    self.fetch_window_around_focus();
                }
                self.toast.push("Cell updated", ToastKind::Success);
                self.dirty = true;
            }
            Message::EditFailed(err) => {
                self.write_in_flight = false;
                self.toast.push(format!("Error: {}", err), ToastKind::Error);
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
                        self.open_popup(PopupKind::ValuePicker(
                            crate::ui::popup::ValuePickerState::new(
                                table,
                                rowid,
                                col.name,
                                col.col_type,
                                values,
                                original,
                            ),
                        ));
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
            Message::FkRowsReady {
                request_id,
                target_table,
                rows,
            } => {
                if request_id != self.popup_request_serial {
                    return;
                }
                if let Some(PopupKind::FkPicker(state)) = &mut self.popup {
                    if state.target_table == target_table {
                        state.rows = rows;
                        state.loading = false;
                    }
                }
                self.dirty = true;
            }
            Message::FkRowsFailed {
                request_id,
                target_table,
                error,
            } => {
                if request_id != self.popup_request_serial {
                    return;
                }
                if let Some(PopupKind::FkPicker(state)) = &mut self.popup {
                    if state.target_table == target_table {
                        state.loading = false;
                        self.toast.push(
                            format!("Foreign-key lookup failed: {error}"),
                            ToastKind::Error,
                        );
                    }
                }
                self.dirty = true;
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
                    self.pending_jump_target = Some(PendingJumpTarget {
                        table: table.clone(),
                        rowid,
                        col: None,
                    });
                    self.open_table(table);
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
            Message::JumpToTargetRow { table, rowid, col } => {
                self.next_navigation_request();
                self.jump_to_rowid(table, rowid, col)
            }
            Message::CycleSort => {
                self.next_navigation_request();
                if let Some(grid) = self.grid.as_mut() {
                    let col_idx = grid.focused_col;
                    grid.sort = match &grid.sort {
                        Some(s) if s.col_idx == col_idx && s.direction == SortDir::Asc => {
                            Some(SortSpec {
                                col_idx,
                                direction: SortDir::Desc,
                            })
                        }
                        Some(s) if s.col_idx == col_idx => None,
                        _ => Some(SortSpec {
                            col_idx,
                            direction: SortDir::Asc,
                        }),
                    };
                    grid.reset_to_top();
                    self.fetch_window_around_focus();
                }
                self.dirty = true;
            }
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
                self.dirty = true;
            }
            Message::JumpToLetter(letter) => {
                let view = self
                    .grid
                    .as_ref()
                    .filter(|grid| grid.sort.is_some())
                    .map(GridState::view_query);
                match view {
                    Some(Ok(view)) => {
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
                    Some(Err(error)) => self.toast.push(error.to_string(), ToastKind::Error),
                    None => {}
                }
                self.dirty = true;
            }
            Message::JumpBack => {
                if let Some(frame) = self.jump_stack.pop() {
                    let _ = self.tx.send(Message::OpenTable(frame.table.clone()));
                    let _ = self.tx.send(Message::JumpToTargetRow {
                        table: frame.table,
                        rowid: frame.rowid,
                        col: Some(frame.col),
                    });
                }
                self.dirty = true;
            }
            Message::OpenFilterPopup => {
                self.popup_request_serial = self.popup_request_serial.wrapping_add(1);
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
                if let Some(grid) = self.grid.as_mut() {
                    grid.filter = crate::filter::FilterSet::default();
                    grid.reset_to_top();
                    let table = grid.table_name.clone();
                    if let Err(error) =
                        crate::filter::save_filter(&grid.filter, &self.db_path, &table)
                    {
                        self.toast.push(
                            format!("Could not clear saved filter: {error}"),
                            ToastKind::Error,
                        );
                    }
                    self.fetch_window_around_focus();
                }
                self.finish_popup();
                self.dirty = true;
            }
            Message::InsertRow => {
                if !self.ensure_writable() {
                    return;
                }
                let insert_is_undoable = self.grid.as_ref().is_some_and(|grid| {
                    self.schema
                        .table(&grid.table_name)
                        .is_some_and(|table| table.has_mutable_rowid())
                });
                if !insert_is_undoable {
                    self.toast.push(
                        "This table has no rowid that can support insert undo",
                        ToastKind::Error,
                    );
                    self.dirty = true;
                    return;
                }
                if let Some(grid) = self.grid.as_mut() {
                    let insert_position = if grid.window.total_rows <= 0 {
                        0
                    } else {
                        (grid.focused_row + 1).min(grid.window.total_rows as usize)
                    };
                    grid.clear_row_selection();
                    let mut state = InsertRowState::new(
                        grid.table_name.clone(),
                        grid.columns.clone(),
                        insert_position,
                    );
                    state.start_editing();
                    grid.focus_cell(grid.focused_row, state.selected);
                    Self::ensure_inline_insert_visible(grid, insert_position);
                    self.open_popup(PopupKind::InsertRow(state));
                    self.toast.push("Alt-Enter commits", ToastKind::Info);
                }
                self.dirty = true;
            }
            Message::CommitInsertRow => {
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
                if let Some((table, values)) = insert_spec {
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
            }
            Message::RowInserted { table, rowid } => {
                self.write_in_flight = false;
                self.invalidate_grid_requests();
                if let Some(grid) = self.grid.as_mut().filter(|grid| grid.table_name == table) {
                    grid.window.total_rows += 1;
                    grid.invalidate_window();
                    self.push_undo(UndoFrame {
                        op: UndoOp::Insert,
                        table,
                        rowid,
                        cols: Vec::new(),
                    });
                }
                self.finish_popup();
                self.toast.push("Row inserted", ToastKind::Success);
                self.dirty = true;
            }
            Message::DeleteRow => {
                if !self.ensure_writable() {
                    return;
                }
                self.request_delete_confirmation();
                self.dirty = true;
            }
            Message::ConfirmDelete => {
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
                            .grid
                            .as_ref()
                            .map(|g| g.columns.clone())
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
            Message::RowDeleted { table, rowid, cols } => {
                self.write_in_flight = false;
                self.invalidate_grid_requests();
                if let Some(grid) = self.grid.as_mut().filter(|grid| grid.table_name == table) {
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
                if let Some(grid) = self.grid.as_mut().filter(|grid| grid.table_name == table) {
                    grid.rows_removed(count);
                }
                let message = match count {
                    0 => "No rows deleted".to_string(),
                    1 => "1 row deleted".to_string(),
                    _ => format!("{} rows deleted", count),
                };
                self.toast.push(message, ToastKind::Success);
                self.dirty = true;
            }
            Message::CancelConfirm => {
                self.pending_confirm = None;
                self.dirty = true;
            }
            Message::UndoAction => {
                if self.readonly {
                    self.toast.push("Read-only: cannot undo", ToastKind::Error);
                    return;
                }
                if self.undo_stack.is_empty() {
                    self.toast.push("Nothing to undo", ToastKind::Info);
                    self.dirty = true;
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
                                db::write::commit_cell_edit(
                                    conn,
                                    &work.table,
                                    column,
                                    work.rowid,
                                    value,
                                )?;
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
                self.dirty = true;
            }
            Message::UndoCompleted { table, message } => {
                self.write_in_flight = false;
                self.invalidate_grid_requests();
                if let Some(grid) = self.grid.as_mut().filter(|grid| grid.table_name == table) {
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
                let table_names = self.schema.tables.iter().map(|t| t.name.clone()).collect();
                self.open_popup(PopupKind::CommandPalette(CommandPaletteState::new(
                    table_names,
                )));
                self.dirty = true;
            }
            Message::OpenHelp => {
                self.open_popup(PopupKind::Help(HelpState::new()));
                self.dirty = true;
            }
            Message::ExecuteCommand(cmd) => {
                self.execute_palette_command(cmd);
                self.dirty = true;
            }
            Message::ExportDone { path, count } => {
                self.toast.push(
                    format!("Exported {} rows to {}", count, path),
                    ToastKind::Success,
                );
                self.dirty = true;
            }
            Message::ExportFailed(error) => {
                self.toast
                    .push(format!("Export failed: {error}"), ToastKind::Error);
                self.dirty = true;
            }
            Message::ReloadSchema => {
                self.spawn_schema_load(false);
                self.dirty = true;
            }
            Message::SchemaReady(schema) => {
                self.schema = schema;
                self.refresh_active_grid_schema();
                self.sidebar.tables_expanded = true;
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
                let text = self.grid.as_ref().and_then(|g| {
                    g.window
                        .get_row(g.focused_row as i64)?
                        .get(g.focused_col)
                        .map(|value| value.to_text().into_owned())
                });
                if let Some(text) = text {
                    self.copy_to_clipboard(&text, "Copied to clipboard");
                }
                self.dirty = true;
            }
            Message::CopyRowJson => {
                self.copy_row_as_json();
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
                } else if let Some(grid) = self.grid.as_mut() {
                    grid.window.fetch_in_flight = false;
                    grid.needs_fetch = true;
                    self.invalidate_grid_requests();
                } else if let Some(table) = self.active_table_name() {
                    self.request_table_view(&table);
                }
                self.dirty = true;
            }
            Message::ExternalRefresh(new_schema) => {
                if self.schema != new_schema {
                    self.schema = new_schema;
                    let total = self.sidebar.visible_count(&self.schema);
                    if self.sidebar.selected >= total {
                        self.sidebar.selected = total.saturating_sub(1);
                    }
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
                let Some(grid) = self.grid.as_ref() else {
                    return;
                };
                let columns = grid.columns.clone();
                let view = match grid.view_query() {
                    Ok(view) => view,
                    Err(error) => {
                        self.toast.push(error.to_string(), ToastKind::Error);
                        return;
                    }
                };
                let table = view.table.clone();
                let request_id = self.open_popup(PopupKind::Find(FindState::new(
                    table.clone(),
                    columns.clone(),
                )));
                self.dirty = true;
                self.spawn_db(
                    move |conn| Ok(db::fetch_rows(conn, &view, &columns, 0, FIND_ROW_LIMIT)?.rows),
                    move |result| match result {
                        Ok(rows) => Message::FindReady {
                            request_id,
                            table,
                            rows,
                        },
                        Err(error) => Message::FindFailed {
                            request_id,
                            table,
                            error,
                        },
                    },
                );
            }
            Message::FindReady {
                request_id,
                table,
                rows,
            } => {
                if request_id != self.popup_request_serial {
                    return;
                }
                if let Some(PopupKind::Find(state)) = &mut self.popup {
                    if state.table_name == table {
                        state.set_rows(rows);
                        self.dirty = true;
                    }
                }
            }
            Message::FindFailed {
                request_id,
                table,
                error,
            } => {
                if request_id != self.popup_request_serial {
                    return;
                }
                if let Some(PopupKind::Find(state)) = &mut self.popup {
                    if state.table_name == table {
                        state.loading = false;
                        self.toast
                            .push(format!("Find failed: {error}"), ToastKind::Error);
                    }
                }
                self.dirty = true;
            }
            Message::CommitFind => {
                let hit = match &self.popup {
                    Some(PopupKind::Find(state)) => state
                        .hits()
                        .get(state.selected)
                        .map(|h| (h.abs_row_index, h.first_match_col)),
                    _ => None,
                };
                if let (Some((abs_row, col)), Some(grid)) = (hit, self.grid.as_mut()) {
                    grid.focus_cell(abs_row, col);
                }
                if !self.finish_popup() {
                    self.fetch_window_if_needed();
                }
                self.dirty = true;
            }
        }
    }

    pub fn view(&mut self, frame: &mut ratatui::Frame) {
        crate::ui::render(frame, self);
    }

    fn execute_palette_command(&mut self, cmd: PaletteCommand) {
        match cmd {
            PaletteCommand::ExportCsv => self.export_view(ExportFormat::Csv),
            PaletteCommand::ExportJson => self.export_view(ExportFormat::Json),
            PaletteCommand::ExportSql => self.export_view(ExportFormat::Sql),
            PaletteCommand::SwitchTable(name) => {
                let _ = self.tx.send(Message::OpenTable(name));
            }
            PaletteCommand::ToggleSidebar => {
                self.sidebar_visible = !self.sidebar_visible;
            }
            PaletteCommand::ToggleReadonly => {
                self.readonly = !self.readonly;
                let msg = if self.readonly {
                    "Read-only mode enabled"
                } else {
                    "Read-only mode disabled"
                };
                self.toast.push(msg, ToastKind::Info);
            }
            PaletteCommand::ClearFilters => {
                let _ = self.tx.send(Message::ClearFilters);
                self.toast.push("Filters cleared", ToastKind::Info);
            }
            PaletteCommand::ReloadSchema => {
                let _ = self.tx.send(Message::ReloadSchema);
            }
            PaletteCommand::CopyCell => {
                let _ = self.tx.send(Message::CopyCell);
            }
            PaletteCommand::CopyRowJson => {
                let _ = self.tx.send(Message::CopyRowJson);
            }
            PaletteCommand::Quit => {
                self.should_quit = true;
            }
        }
    }

    fn copy_row_as_json(&mut self) {
        match self.row_json_text() {
            Ok(Some((text, copied_selected_rows))) => {
                let success_message = if copied_selected_rows {
                    "Copied selected rows JSON to clipboard"
                } else {
                    "Copied row JSON to clipboard"
                };
                self.copy_to_clipboard(&text, success_message);
            }
            Ok(None) => {}
            Err(err) => {
                self.toast.push(err.to_string(), ToastKind::Error);
            }
        }
    }

    fn row_json_text(&self) -> anyhow::Result<Option<(String, bool)>> {
        let Some(grid) = self.grid.as_ref() else {
            return Ok(None);
        };
        let view = grid.view_query()?;
        let conn = self.pool.get()?;
        let copying_selection = grid.has_row_selection();
        let rows = match &grid.row_selection {
            RowSelection::All { except } => db::fetch_rows(
                &conn,
                &view,
                &grid.columns,
                0,
                grid.window.total_rows.max(0),
            )?
            .rows
            .into_iter()
            .enumerate()
            .filter(|(offset, _)| !except.contains(offset))
            .map(|(_, row)| row)
            .collect(),
            _ if copying_selection => {
                let offsets = grid
                    .selected_rows()
                    .into_iter()
                    .map(|offset| offset as i64)
                    .collect::<Vec<_>>();
                db::fetch_rows_at_offsets(&conn, &view, &grid.columns, &offsets)?
            }
            _ => db::fetch_rows(&conn, &view, &grid.columns, grid.focused_row as i64, 1)?.rows,
        };
        if rows.is_empty() {
            return Ok(None);
        }

        let to_json = |row: &Vec<SqlValue>| crate::export::row_to_json(&grid.columns, row);
        let json = if copying_selection {
            serde_json::to_string(&rows.iter().map(to_json).collect::<Vec<_>>())?
        } else {
            serde_json::to_string(&to_json(&rows[0]))?
        };
        Ok(Some((json, copying_selection)))
    }

    fn copy_to_clipboard(&mut self, text: &str, success_message: &str) {
        use std::io::Write;
        let encoded = base64_encode(text.as_bytes());
        let osc52 = format!("\x1b]52;c;{}\x07", encoded);
        let _ = std::io::stdout().write_all(osc52.as_bytes());
        let _ = std::io::stdout().flush();
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

    fn ensure_inline_insert_visible(grid: &mut crate::grid::GridState, insert_position: usize) {
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
        if self.readonly {
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

    /// Opens the editor that fits the column type and current value. Plain values
    /// first look up the column's distinct values to decide on a value picker.
    fn open_cell_editor(&mut self, cell: FocusedCellContext) {
        let FocusedCellContext {
            col,
            table_name,
            rowid,
            cell_value: original,
            ..
        } = cell;
        let upper = col.col_type.to_uppercase();
        let looks_like_epoch_datetime = (upper.contains("INT") || upper.contains("NUM")) && {
            let name = col.name.to_lowercase();
            name.ends_with("_at")
                || name.contains("timestamp")
                || name.contains("created_at")
                || name.contains("updated_at")
        };
        let temporal = temporal_kind(&col.col_type);
        if temporal == Some(TemporalKind::Datetime)
            || looks_like_epoch_datetime
            || DatePickerState::supports_datetime(&original)
        {
            self.open_popup(PopupKind::DatePicker(DatePickerState::datetime(
                table_name, rowid, col.name, original,
            )));
        } else if temporal == Some(TemporalKind::Date) || DatePickerState::supports_date(&original)
        {
            self.open_popup(PopupKind::DatePicker(DatePickerState::date(
                table_name, rowid, col.name, original,
            )));
        } else if matches!(&original, SqlValue::Blob(_))
            || matches!(affinity(&col.col_type), ColAffinity::Blob)
        {
            self.open_text_editor(table_name, rowid, col.name, col.col_type, original);
        } else {
            self.popup_request_serial = self.popup_request_serial.wrapping_add(1);
            let request_id = self.popup_request_serial;
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
            self.toast.push("Loading distinct values", ToastKind::Info);
        }
    }

    /// Opens the foreign-key picker for `cell` and loads candidate rows; false when
    /// the column's reference cannot be resolved in the schema.
    fn open_fk_picker(&mut self, cell: &FocusedCellContext) -> bool {
        let Some(fk) = self
            .schema
            .table(&cell.table_name)
            .and_then(|table| table.foreign_key(&cell.col.name))
            .cloned()
        else {
            return false;
        };
        let target_columns = self
            .schema
            .table(&fk.to_table)
            .map(|table| table.columns.as_slice())
            .unwrap_or_default();
        let Some(key_column) = target_columns.iter().find(|c| c.name == fk.to_col) else {
            return false;
        };
        let display_columns = target_columns
            .iter()
            .filter(|c| c.name != fk.to_col)
            .cloned()
            .collect::<Vec<_>>();
        // The picker expects the referenced key first, then the descriptive columns.
        let fetch_columns = std::iter::once(key_column.clone())
            .chain(display_columns.iter().cloned())
            .collect::<Vec<_>>();
        let request_id = self.open_popup(PopupKind::FkPicker(FkPickerState::new(
            fk.to_table.clone(),
            fk.to_col.clone(),
            display_columns.into_iter().map(|c| c.name).collect(),
            cell.table_name.clone(),
            cell.col.name.clone(),
            cell.rowid,
            cell.cell_value.clone(),
        )));
        let target_table = fk.to_table;
        let view = ViewQuery::table(&target_table);
        self.spawn_db(
            move |conn| {
                Ok(db::fetch_rows(conn, &view, &fetch_columns, 0, FK_PICKER_ROW_LIMIT)?.rows)
            },
            move |result| match result {
                Ok(rows) => Message::FkRowsReady {
                    request_id,
                    target_table,
                    rows,
                },
                Err(error) => Message::FkRowsFailed {
                    request_id,
                    target_table,
                    error,
                },
            },
        );
        true
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
            return;
        };
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
        let (message, kind) = match &grid.row_selection {
            RowSelection::None => {
                let Some(rowid) = grid.window.get_rowid(grid.focused_row as i64) else {
                    return;
                };
                (
                    format!("Delete row #{}? [y/n]", grid.focused_row + 1),
                    ConfirmKind::DeleteRow { table, rowid },
                )
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
                let rowids = match self.resolve_selected_rowids(grid, &offsets) {
                    Ok(rowids) if rowids.len() == offsets.len() => rowids,
                    Ok(_) => {
                        self.toast
                            .push("Some selected rows no longer exist", ToastKind::Error);
                        return;
                    }
                    Err(error) => {
                        self.toast.push(error.to_string(), ToastKind::Error);
                        return;
                    }
                };
                let noun = if rowids.len() == 1 { "row" } else { "rows" };
                (
                    format!("Delete {} selected {}? [y/n]", rowids.len(), noun),
                    ConfirmKind::DeleteSelectedRows { table, rowids },
                )
            }
            RowSelection::All { except } => {
                let except = except.iter().map(|&row| row as i64).collect::<Vec<_>>();
                let counted = self
                    .resolve_selected_rowids(grid, &except)
                    .and_then(|keep| {
                        let conn = self.pool.get()?;
                        Ok((db::count_rows(&conn, &ViewQuery::table(&table))?, keep))
                    });
                let (total_rows, keep) = match counted {
                    Ok(counted) => counted,
                    Err(error) => {
                        self.toast.push(error.to_string(), ToastKind::Error);
                        return;
                    }
                };
                let deleting = total_rows - keep.len() as i64;
                if deleting <= 0 {
                    self.toast.push("Nothing to delete", ToastKind::Info);
                    return;
                }
                let message = if keep.is_empty() {
                    format!("Delete all {total_rows} rows from {table}? [y/n]")
                } else {
                    format!(
                        "Delete {deleting} rows from {table}, keeping {} deselected? [y/n]",
                        keep.len()
                    )
                };
                (message, ConfirmKind::ClearTable { table, keep })
            }
        };
        self.pending_confirm = Some(PendingConfirm {
            message,
            kind,
            created: std::time::Instant::now(),
        });
    }

    fn resolve_selected_rowids(
        &self,
        grid: &GridState,
        offsets: &[i64],
    ) -> anyhow::Result<Vec<i64>> {
        let view = grid.view_query()?;
        let conn = self.pool.get()?;
        db::fetch_rowids_at_offsets(&conn, &view, offsets)
    }

    fn export_view(&mut self, format: ExportFormat) {
        let Some(grid) = self.grid.as_ref() else {
            return;
        };
        let columns = grid.columns.clone();
        let view = match grid.view_query() {
            Ok(view) => view,
            Err(error) => {
                self.toast
                    .push(format!("Export failed: {error}"), ToastKind::Error);
                return;
            }
        };
        let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
        let safe_table = view
            .table
            .chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                    c
                } else {
                    '_'
                }
            })
            .collect::<String>();
        let export_id = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |duration| duration.as_nanos());
        let path = format!(
            "{home}/sqview_{safe_table}_{export_id}.{}",
            format.extension()
        );
        let job_path = path.clone();
        self.spawn_db(
            move |conn| {
                crate::export::export(
                    conn,
                    format,
                    &view,
                    &columns,
                    std::path::Path::new(&job_path),
                )
            },
            move |result| match result {
                Ok(count) => Message::ExportDone { path, count },
                Err(error) => Message::ExportFailed(error),
            },
        );
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

    /// Shows `popup`, invalidating lookups started for any previous popup, and
    /// returns the request id for loading its content.
    fn open_popup(&mut self, popup: PopupKind) -> u64 {
        self.popup_request_serial = self.popup_request_serial.wrapping_add(1);
        self.popup = Some(popup);
        self.mode = AppMode::Edit;
        self.popup_request_serial
    }

    fn ensure_writable(&mut self) -> bool {
        if self.readonly {
            self.toast.push("Read-only database", ToastKind::Error);
        }
        !self.readonly
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

    fn finish_popup(&mut self) -> bool {
        self.popup_request_serial = self.popup_request_serial.wrapping_add(1);
        self.popup = None;
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
        let readonly = self.readonly;
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

    fn active_table_name(&self) -> Option<String> {
        self.active_tab
            .and_then(|index| self.open_tabs.get(index))
            .map(|tab| tab.table_name.clone())
    }

    fn open_table(&mut self, name: String) {
        self.focus = FocusPane::Grid;
        let index = match self.open_tabs.iter().position(|t| t.table_name == name) {
            Some(index) => index,
            None => {
                self.open_tabs.push(TableTab {
                    table_name: name.clone(),
                });
                self.open_tabs.len() - 1
            }
        };
        self.active_tab = Some(index);
        self.request_table_view(&name);
        self.dirty = true;
    }

    fn request_table_view(&mut self, name: &str) {
        self.next_navigation_request();
        self.invalidate_grid_requests();
        self.grid = None;
        if let Some(table) = self.schema.table(name) {
            let (columns, fk_cols) = (table.columns.clone(), table.foreign_key_flags());
            self.spawn_grid_fetch(name.to_string(), columns, fk_cols);
        }
    }

    fn refresh_active_grid_schema(&mut self) {
        self.next_navigation_request();
        let Some(current_grid) = self.grid.as_ref() else {
            if let Some(table) = self.active_table_name() {
                self.request_table_view(&table);
            }
            return;
        };
        let table = current_grid.table_name.clone();
        let Some(table_meta) = self.schema.table(&table) else {
            self.invalidate_grid_requests();
            self.grid = None;
            self.toast.push(
                format!("Table {table:?} no longer exists"),
                ToastKind::Error,
            );
            return;
        };

        let columns = table_meta.columns.clone();
        let fk_cols = table_meta.foreign_key_flags();
        let sort = current_grid.order_by().and_then(|order| {
            let col_idx = columns
                .iter()
                .position(|column| column.name == order.column)?;
            let direction = if order.ascending {
                SortDir::Asc
            } else {
                SortDir::Desc
            };
            Some(SortSpec { col_idx, direction })
        });
        let mut filter = current_grid.filter.clone();
        filter
            .columns
            .retain(|name, _| columns.iter().any(|column| &column.name == name));

        let Some(grid) = self.grid.as_mut() else {
            return;
        };
        grid.columns = columns;
        grid.fk_cols = fk_cols;
        grid.enumerated_values = vec![Vec::new(); grid.columns.len()];
        grid.width_sample_rows.clear();
        grid.focused_col = grid.focused_col.min(grid.columns.len().saturating_sub(1));
        grid.h_scroll = 0;
        grid.sort = sort;
        grid.filter = filter;
        grid.reset_to_top();
        grid.window.total_rows = 0;
        grid.recompute_col_widths(grid.avail_col_width);
        self.fetch_window_around_focus();
    }

    fn spawn_grid_fetch(&mut self, table: String, columns: Vec<Column>, fk_cols: Vec<bool>) {
        let request_id = self.next_grid_request();
        let job_table = table.clone();
        let job_columns = columns.clone();
        self.spawn_db(
            move |conn| {
                let view = ViewQuery::table(&job_table);
                let total = db::count_rows(conn, &view)?;
                let fetched = db::fetch_rows(conn, &view, &job_columns, 0, INITIAL_ROW_LIMIT)?;
                Ok((fetched, total))
            },
            move |result| match result {
                Ok((fetched, total_rows)) => Message::GridDataReady {
                    request_id,
                    table,
                    columns,
                    fk_cols,
                    fetched,
                    total_rows,
                },
                Err(error) => Message::GridReadFailed {
                    request_id,
                    table,
                    error,
                },
            },
        );
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

    /// Reads the window around the focused row, superseding any read in flight.
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
        let columns = grid.columns.clone();
        let table = view.table.clone();
        let request_id = self.next_grid_request();
        self.spawn_db(
            move |conn| {
                let total = db::count_rows(conn, &view)?;
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

    fn on_grid_data_ready(
        &mut self,
        table: String,
        columns: Vec<Column>,
        fk_cols: Vec<bool>,
        fetched: db::FetchedRows,
        total_rows: i64,
    ) {
        if self.active_table_name().as_deref() == Some(table.as_str()) {
            let mut grid = GridState::new(crate::grid::GridInit {
                table_name: table.clone(),
                enumerated_values: (0..columns.len())
                    .map(|column| inferred_enumerated_values(&fetched.rows, column, total_rows))
                    .collect(),
                columns,
                fk_cols,
                width_sample_rows: fetched.rows.clone(),
                rows: fetched.rows,
                total_rows,
                area_width: self.grid_inner_area.map_or(0, |area| area.width),
            });
            grid.window.rowids = fetched.rowids;
            let saved_filter = crate::filter::load_filter(&self.db_path, &table)
                .ok()
                .filter(|filter| !filter.is_empty());
            let filtered = saved_filter.is_some();
            if let Some(filter) = saved_filter {
                grid.filter = filter;
            }
            self.grid = Some(grid);
            if filtered {
                self.fetch_window_around_focus();
            }
        }
        self.dirty = true;
    }

    fn jump_to_rowid(&mut self, table: String, rowid: i64, col: Option<usize>) {
        let target = self
            .grid
            .as_ref()
            .filter(|grid| grid.table_name == table)
            .map(|grid| (grid.view_query(), col.unwrap_or(grid.focused_col)));
        let Some((view, target_col)) = target else {
            self.pending_jump_target = Some(PendingJumpTarget { table, rowid, col });
            self.dirty = true;
            return;
        };
        let offset = view.and_then(|view| {
            let conn = self.pool.get()?;
            db::fetch_offset_for_rowid(&conn, &view, rowid)
        });
        match offset {
            Ok(Some(target_row)) => {
                self.update_grid(|grid| grid.focus_cell(target_row as usize, target_col));
            }
            Ok(None) => self
                .toast
                .push("Row not found in current view", ToastKind::Error),
            Err(err) => self
                .toast
                .push(format!("Row lookup failed: {}", err), ToastKind::Error),
        }
        self.dirty = true;
    }

    fn apply_column_filter(&mut self, col_name: String, col_filter: crate::filter::ColumnFilter) {
        self.next_navigation_request();
        if let Some(grid) = self.grid.as_mut() {
            if col_filter.rules.iter().any(|rule| rule.enabled) {
                grid.filter.columns.insert(col_name, col_filter);
            } else {
                grid.filter.columns.remove(&col_name);
            }
            grid.reset_to_top();
            if let Err(error) =
                crate::filter::save_filter(&grid.filter, &self.db_path, &grid.table_name)
            {
                self.toast
                    .push(format!("Could not save filter: {error}"), ToastKind::Error);
            }
            self.fetch_window_around_focus();
        }
        self.dirty = true;
    }

    fn close_tab(&mut self, idx: usize) {
        if idx < self.open_tabs.len() {
            let next_active =
                next_active_tab_after_close(self.active_tab, idx, self.open_tabs.len() - 1);
            self.open_tabs.remove(idx);
            self.active_tab = next_active;
            if let Some(table) = self.active_table_name() {
                self.request_table_view(&table);
            } else {
                self.grid = None;
            }
            self.dirty = true;
        }
    }

    fn activate_tab(&mut self, idx: usize) {
        if idx < self.open_tabs.len() {
            self.active_tab = Some(idx);
            let table = self.open_tabs[idx].table_name.clone();
            self.request_table_view(&table);
            self.dirty = true;
        }
    }
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

fn normalize_enumerated_values(values: Vec<String>, total_rows: i64) -> Vec<String> {
    if values.is_empty()
        || values.len() >= ENUM_COLOR_DISTINCT_LIMIT
        || values.iter().any(|value| value.chars().count() > 20)
        || (total_rows > 0 && values.len() as i64 == total_rows)
    {
        Vec::new()
    } else {
        values
    }
}

fn inferred_enumerated_values(
    rows: &[Vec<SqlValue>],
    column: usize,
    total_rows: i64,
) -> Vec<String> {
    let mut values = rows
        .iter()
        .filter_map(|row| row.get(column))
        .filter_map(|value| match value {
            SqlValue::Null | SqlValue::Blob(_) => None,
            SqlValue::Integer(value) => Some(value.to_string()),
            SqlValue::Real(value) => Some(value.to_string()),
            SqlValue::Text(value) => Some(value.clone()),
        })
        .collect::<Vec<_>>();
    values.sort();
    values.dedup();
    normalize_enumerated_values(values, total_rows)
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
