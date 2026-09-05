use std::sync::{
    atomic::{AtomicU64, Ordering},
    Arc,
};

use ratatui::layout::Rect;
use rusqlite::OptionalExtension;
use tokio::sync::mpsc::UnboundedSender;

use crate::{
    config::Config,
    db::{
        self,
        schema::Column,
        schema::Schema,
        types::{affinity, ColAffinity, SqlValue},
        DbPool,
    },
    filter::predicate::filter_to_sql,
    grid::{RowSelection, SortDir, SortSpec},
    symbols::Symbols,
    theme::Theme,
    ui::{
        popup::{
            CommandPaletteState, DateFocus, DatePickerState, DatetimeFocus, DatetimePickerState,
            FilterPopupState, FindState, FkPickerState, HelpState, InsertRowState, PaletteCommand,
            PopupKind, TextEditorState,
        },
        sidebar::{SidebarAction, SidebarState},
        tabbar::TabMouseAction,
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
    DeleteRow { table: String, rowid: i64 },
    DeleteSelectedRows { table: String, rowids: Vec<i64> },
    ClearTable { table: String },
}

pub struct PendingConfirm {
    pub message: String,
    pub kind: ConfirmKind,
    pub created: std::time::Instant,
    pub timeout_secs: u64,
}

struct GridScrollbarDrag {
    grab_offset: i64,
}

type GridFetchResult = (
    Vec<Vec<SqlValue>>,
    Vec<Option<i64>>,
    i64,
    Vec<Vec<String>>,
    Vec<Vec<SqlValue>>,
);

const VALUE_PICKER_DISTINCT_LIMIT: usize = 100;
const ENUM_COLOR_DISTINCT_LIMIT: usize = 20;

struct GridDataReadyPayload {
    table: String,
    columns: Vec<Column>,
    fk_cols: Vec<bool>,
    enumerated_values: Vec<Vec<String>>,
    width_sample_rows: Vec<Vec<SqlValue>>,
    rows: Vec<Vec<SqlValue>>,
    rowids: Vec<Option<i64>>,
    total_rows: i64,
}

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
    pub undo_stack: Vec<UndoFrame>,
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
    grid_request_serial: Arc<AtomicU64>,
    navigation_request_serial: u64,
    write_in_flight: bool,
    popup_request_serial: u64,
}

#[allow(dead_code)]
#[derive(Debug)]
pub enum Message {
    Quit,
    Key(crossterm::event::KeyEvent),
    Mouse(crossterm::event::MouseEvent),
    Resize(u16, u16),
    Tick,
    OpenTable(String),
    CloseTab(usize),
    ActivateTab(usize),
    NextTab,
    PrevTab,
    GridDataReady {
        request_id: u64,
        table: String,
        columns: Vec<Column>,
        fk_cols: Vec<bool>,
        enumerated_values: Vec<Vec<String>>,
        width_sample_rows: Vec<Vec<SqlValue>>,
        rows: Vec<Vec<SqlValue>>,
        rowids: Vec<Option<i64>>,
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
    ScrollToRow(i64),
    ScrollToEnd,
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
    ApplyFilter,
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
        format: String,
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
            undo_stack: Vec::new(),
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
            grid_request_serial: Arc::new(AtomicU64::new(0)),
            navigation_request_serial: 0,
            write_in_flight: false,
            popup_request_serial: 0,
        }
    }

    pub fn update(&mut self, msg: Message) {
        match msg {
            Message::Quit => self.should_quit = true,
            Message::Resize(_w, _h) => self.dirty = true,
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
                let maybe_fetch = if let Some(ref mut grid) = self.grid {
                    grid.window.tick_count = grid.window.tick_count.wrapping_add(1);
                    if grid.needs_fetch && !grid.window.fetch_in_flight {
                        grid.window.fetch_in_flight = true;
                        grid.needs_fetch = false;
                        let (off, lim) = grid.window.fetch_params(grid.focused_row as i64);
                        let sort = grid.sort.as_ref().and_then(|s| {
                            grid.columns
                                .get(s.col_idx)
                                .map(|c| (c.name.clone(), s.direction == SortDir::Asc))
                        });
                        Some((
                            grid.table_name.clone(),
                            grid.columns.clone(),
                            sort,
                            off,
                            lim,
                        ))
                    } else {
                        None
                    }
                } else {
                    None
                };
                if let Some((table, cols, sort, off, lim)) = maybe_fetch {
                    self.spawn_window_fetch(&table, &cols, sort, off, lim);
                    self.dirty = true;
                }
                if self.grid.as_ref().is_some_and(|g| g.window.fetch_in_flight) {
                    self.dirty = true;
                }
                let expired = self
                    .pending_confirm
                    .as_ref()
                    .is_some_and(|c| c.created.elapsed().as_secs() >= c.timeout_secs);
                if expired {
                    self.pending_confirm = None;
                    self.dirty = true;
                }
            }
            Message::OpenTable(name) => self.open_table(name),
            Message::CloseTab(idx) => self.close_tab(idx),
            Message::ActivateTab(idx) => self.activate_tab(idx),
            Message::NextTab => self.next_tab(),
            Message::PrevTab => self.prev_tab(),
            Message::GridDataReady {
                request_id,
                table,
                columns,
                fk_cols,
                enumerated_values,
                width_sample_rows,
                rows,
                rowids,
                total_rows,
            } => {
                if request_id != self.grid_request_serial.load(Ordering::Acquire) {
                    return;
                }
                self.on_grid_data_ready(GridDataReadyPayload {
                    columns,
                    fk_cols,
                    enumerated_values,
                    width_sample_rows,
                    rows,
                    rowids,
                    table: table.clone(),
                    total_rows,
                });
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
                if request_id != self.grid_request_serial.load(Ordering::Acquire) {
                    return;
                }
                if let Some(ref mut grid) = self.grid {
                    if grid.table_name == table {
                        let fetch_was_queued = grid.needs_fetch;
                        grid.window.offset = offset;
                        grid.window.rows = rows;
                        grid.window.rowids = rowids;
                        grid.window.total_rows = total_rows;
                        grid.window.fetch_in_flight = false;
                        if total_rows > 0 {
                            let max_row = (total_rows - 1) as usize;
                            if grid.focused_row > max_row {
                                grid.focused_row = max_row;
                            }
                        } else {
                            grid.focused_row = 0;
                        }
                        let vp = grid.window.viewport_rows as i64;
                        let max_start = (total_rows - vp).max(0);
                        if grid.viewport_start > max_start {
                            grid.viewport_start = max_start;
                        }
                        grid.needs_fetch =
                            fetch_was_queued && grid.window.needs_prefetch(grid.focused_row as i64);
                    }
                }
                self.dirty = true;
            }
            Message::GridReadFailed {
                request_id,
                table,
                error,
            } => {
                if request_id == self.grid_request_serial.load(Ordering::Acquire) {
                    if let Some(grid) = self.grid.as_mut().filter(|grid| grid.table_name == table) {
                        grid.window.fetch_in_flight = false;
                        grid.needs_fetch = false;
                    }
                    self.toast.push(error, ToastKind::Error);
                    self.dirty = true;
                }
            }
            Message::ScrollDown(n) => {
                self.scroll_grid_down(n);
                self.dirty = true;
            }
            Message::ScrollUp(n) => {
                self.scroll_grid_up(n);
                self.dirty = true;
            }
            Message::ScrollToRow(i) => {
                self.scroll_grid_to_row(i);
                self.dirty = true;
            }
            Message::ScrollToEnd => {
                let maybe_fetch = if let Some(ref mut grid) = self.grid {
                    grid.commit_row_selection();
                    grid.scroll_to_end();
                    if grid.needs_fetch && !grid.window.fetch_in_flight {
                        grid.window.fetch_in_flight = true;
                        grid.needs_fetch = false;
                        let (off, lim) = grid.window.fetch_params(grid.focused_row as i64);
                        let sort = grid.sort.as_ref().and_then(|s| {
                            grid.columns
                                .get(s.col_idx)
                                .map(|c| (c.name.clone(), s.direction == SortDir::Asc))
                        });
                        Some((
                            grid.table_name.clone(),
                            grid.columns.clone(),
                            sort,
                            off,
                            lim,
                        ))
                    } else {
                        None
                    }
                } else {
                    None
                };
                if let Some((table, cols, sort, off, lim)) = maybe_fetch {
                    self.spawn_window_fetch(&table, &cols, sort, off, lim);
                }
                self.dirty = true;
            }
            Message::MoveDown => {
                self.scroll_grid_down(1);
                self.dirty = true;
            }
            Message::MoveUp => {
                self.scroll_grid_up(1);
                self.dirty = true;
            }
            Message::MoveRight => {
                if let Some(ref mut grid) = self.grid {
                    grid.move_col_right();
                }
                self.dirty = true;
            }
            Message::MoveLeft => {
                if let Some(ref mut grid) = self.grid {
                    grid.move_col_left();
                }
                self.dirty = true;
            }
            Message::MoveColFirst => {
                if let Some(ref mut grid) = self.grid {
                    grid.move_col_first();
                }
                self.dirty = true;
            }
            Message::MoveColLast => {
                if let Some(ref mut grid) = self.grid {
                    grid.move_col_last();
                }
                self.dirty = true;
            }
            Message::MoveFirstCell => {
                let maybe_fetch = if let Some(ref mut grid) = self.grid {
                    grid.focused_col = 0;
                    grid.h_scroll = 0;
                    grid.commit_row_selection();
                    grid.scroll_to_row(0);
                    if grid.needs_fetch && !grid.window.fetch_in_flight {
                        grid.window.fetch_in_flight = true;
                        grid.needs_fetch = false;
                        let (off, lim) = grid.window.fetch_params(grid.focused_row as i64);
                        let sort = grid.sort.as_ref().and_then(|s| {
                            grid.columns
                                .get(s.col_idx)
                                .map(|c| (c.name.clone(), s.direction == SortDir::Asc))
                        });
                        Some((
                            grid.table_name.clone(),
                            grid.columns.clone(),
                            sort,
                            off,
                            lim,
                        ))
                    } else {
                        None
                    }
                } else {
                    None
                };
                if let Some((table, cols, sort, off, lim)) = maybe_fetch {
                    self.spawn_window_fetch(&table, &cols, sort, off, lim);
                }
                self.dirty = true;
            }
            Message::MoveLastCell => {
                let maybe_fetch = if let Some(ref mut grid) = self.grid {
                    grid.move_col_last();
                    grid.commit_row_selection();
                    grid.scroll_to_end();
                    if grid.needs_fetch && !grid.window.fetch_in_flight {
                        grid.window.fetch_in_flight = true;
                        grid.needs_fetch = false;
                        let (off, lim) = grid.window.fetch_params(grid.focused_row as i64);
                        let sort = grid.sort.as_ref().and_then(|s| {
                            grid.columns
                                .get(s.col_idx)
                                .map(|c| (c.name.clone(), s.direction == SortDir::Asc))
                        });
                        Some((
                            grid.table_name.clone(),
                            grid.columns.clone(),
                            sort,
                            off,
                            lim,
                        ))
                    } else {
                        None
                    }
                } else {
                    None
                };
                if let Some((table, cols, sort, off, lim)) = maybe_fetch {
                    self.spawn_window_fetch(&table, &cols, sort, off, lim);
                }
                self.dirty = true;
            }
            Message::OpenPopup => {
                if self.readonly {
                    self.toast.push("Read-only database", ToastKind::Error);
                    return;
                }
                self.popup_request_serial = self.popup_request_serial.wrapping_add(1);
                if let Some(FocusedCellContext {
                    col,
                    table_name,
                    rowid: actual_rowid,
                    cell_value,
                    is_fk,
                }) = self.focused_cell_context()
                {
                    if is_fk {
                        let table_meta = self.schema.tables.iter().find(|t| t.name == table_name);
                        let fk_opt = table_meta.and_then(|tm| {
                            tm.foreign_keys
                                .iter()
                                .find(|fk| fk.from_col == col.name)
                                .cloned()
                        });
                        if let Some(fk) = fk_opt {
                            let target_meta =
                                self.schema.tables.iter().find(|t| t.name == fk.to_table);
                            let display_cols = target_meta
                                .map(|tm| {
                                    tm.columns
                                        .iter()
                                        .filter(|c| c.name != fk.to_col)
                                        .map(|c| c.name.clone())
                                        .collect::<Vec<_>>()
                                })
                                .unwrap_or_default();

                            let picker_state = FkPickerState::new(
                                fk.to_table.clone(),
                                fk.to_col.clone(),
                                display_cols.clone(),
                                table_name.clone(),
                                col.name.clone(),
                                actual_rowid,
                                cell_value.clone(),
                            );
                            self.popup = Some(PopupKind::FkPicker(picker_state));
                            self.mode = AppMode::Edit;
                            self.popup_request_serial = self.popup_request_serial.wrapping_add(1);
                            let request_id = self.popup_request_serial;

                            let pool = Arc::clone(&self.pool);
                            let tx = self.tx.clone();
                            let to_table = fk.to_table.clone();
                            let to_col = fk.to_col.clone();
                            let disp_cols = display_cols;
                            tokio::task::spawn(async move {
                                let to_table_c = to_table.clone();
                                let result = tokio::task::spawn_blocking(
                                    move || -> anyhow::Result<Vec<Vec<SqlValue>>> {
                                        let conn = pool.get()?;
                                        let col_list = std::iter::once(&to_col)
                                            .chain(disp_cols.iter())
                                            .map(|c| db::query::quote_identifier(c))
                                            .collect::<Vec<_>>()
                                            .join(", ");
                                        let sql = format!(
                                            "SELECT {} FROM {} LIMIT 200",
                                            col_list,
                                            db::query::quote_identifier(&to_table_c)
                                        );
                                        let mut stmt = conn.prepare(&sql)?;
                                        let col_count = 1 + disp_cols.len();
                                        let rows = stmt
                                            .query_map([], |row| {
                                                let mut vals = Vec::new();
                                                for i in 0..col_count {
                                                    let v = match row.get_ref(i)? {
                                                        rusqlite::types::ValueRef::Null => {
                                                            SqlValue::Null
                                                        }
                                                        rusqlite::types::ValueRef::Integer(n) => {
                                                            SqlValue::Integer(n)
                                                        }
                                                        rusqlite::types::ValueRef::Real(f) => {
                                                            SqlValue::Real(f)
                                                        }
                                                        rusqlite::types::ValueRef::Text(b) => {
                                                            SqlValue::Text(
                                                                String::from_utf8_lossy(b)
                                                                    .into_owned(),
                                                            )
                                                        }
                                                        rusqlite::types::ValueRef::Blob(b) => {
                                                            SqlValue::Blob(b.to_vec())
                                                        }
                                                    };
                                                    vals.push(v);
                                                }
                                                Ok(vals)
                                            })?
                                            .collect::<Result<Vec<_>, _>>()?;
                                        Ok(rows)
                                    },
                                )
                                .await;
                                match result {
                                    Ok(Ok(rows)) => {
                                        let _ = tx.send(Message::FkRowsReady {
                                            request_id,
                                            target_table: to_table,
                                            rows,
                                        });
                                    }
                                    Ok(Err(error)) => {
                                        let _ = tx.send(Message::FkRowsFailed {
                                            request_id,
                                            target_table: to_table,
                                            error: error.to_string(),
                                        });
                                    }
                                    Err(error) => {
                                        let _ = tx.send(Message::FkRowsFailed {
                                            request_id,
                                            target_table: to_table,
                                            error: error.to_string(),
                                        });
                                    }
                                }
                            });

                            self.dirty = true;
                            return;
                        }
                    }

                    let upper = col.col_type.to_uppercase();
                    let original = cell_value;
                    let looks_like_epoch_datetime =
                        (upper.contains("INT") || upper.contains("NUM")) && {
                            let name = col.name.to_lowercase();
                            name.ends_with("_at")
                                || name.contains("timestamp")
                                || name.contains("created_at")
                                || name.contains("updated_at")
                        };
                    if upper.contains("TIMESTAMP")
                        || upper.contains("DATETIME")
                        || (upper.contains("DATE") && upper.contains("TIME"))
                        || looks_like_epoch_datetime
                        || DatetimePickerState::supports_value(&original)
                    {
                        self.popup = Some(PopupKind::DatetimePicker(DatetimePickerState::new(
                            table_name,
                            actual_rowid,
                            col.name,
                            original,
                        )));
                    } else if upper.contains("DATE") || DatePickerState::supports_value(&original) {
                        self.popup = Some(PopupKind::DatePicker(DatePickerState::new(
                            table_name,
                            actual_rowid,
                            col.name,
                            original,
                        )));
                    } else if matches!(&original, SqlValue::Blob(_))
                        || matches!(affinity(&col.col_type), ColAffinity::Blob)
                    {
                        self.open_text_editor(
                            table_name,
                            actual_rowid,
                            col.name,
                            col.col_type,
                            original,
                        );
                    } else {
                        self.popup_request_serial = self.popup_request_serial.wrapping_add(1);
                        let request_id = self.popup_request_serial;
                        let pool = Arc::clone(&self.pool);
                        let tx = self.tx.clone();
                        let response_table = table_name.clone();
                        let response_col = col.clone();
                        let response_original = original.clone();
                        tokio::task::spawn(async move {
                            let query_table = table_name.clone();
                            let query_column = col.name.clone();
                            let result = tokio::task::spawn_blocking(
                                move || -> anyhow::Result<Vec<String>> {
                                    let conn = pool.get()?;
                                    db::load_distinct_values(
                                        &conn,
                                        &query_table,
                                        &query_column,
                                        VALUE_PICKER_DISTINCT_LIMIT + 1,
                                    )
                                },
                            )
                            .await;
                            match result {
                                Ok(Ok(values)) => {
                                    let _ = tx.send(Message::DistinctValuesReady {
                                        request_id,
                                        table: response_table,
                                        rowid: actual_rowid,
                                        col: response_col,
                                        original: response_original,
                                        values,
                                    });
                                }
                                Ok(Err(error)) => {
                                    let _ = tx.send(Message::DistinctValuesFailed {
                                        request_id,
                                        table: response_table,
                                        rowid: actual_rowid,
                                        col: response_col,
                                        original: response_original,
                                        error: error.to_string(),
                                    });
                                }
                                Err(error) => {
                                    let _ = tx.send(Message::DistinctValuesFailed {
                                        request_id,
                                        table: response_table,
                                        rowid: actual_rowid,
                                        col: response_col,
                                        original: response_original,
                                        error: error.to_string(),
                                    });
                                }
                            }
                        });
                        self.toast.push("Loading distinct values", ToastKind::Info);
                        self.dirty = true;
                        return;
                    }
                    self.mode = AppMode::Edit;
                }
                self.dirty = true;
            }
            Message::OpenDirectEdit => {
                if self.readonly {
                    self.toast.push("Read-only database", ToastKind::Error);
                    return;
                }
                if let Some(FocusedCellContext {
                    table_name,
                    rowid,
                    cell_value,
                    col,
                    ..
                }) = self.focused_cell_context()
                {
                    self.open_text_editor(table_name, rowid, col.name, col.col_type, cell_value);
                }
                self.dirty = true;
            }
            Message::SetFocusedCellNull => {
                if self.readonly {
                    self.toast.push("Read-only database", ToastKind::Error);
                    return;
                }
                if let Some(FocusedCellContext {
                    col,
                    table_name,
                    rowid,
                    cell_value,
                    ..
                }) = self.focused_cell_context()
                {
                    if col.not_null {
                        self.toast.push("Column is NOT NULL", ToastKind::Error);
                    } else if cell_value == SqlValue::Null {
                        self.toast.push("Cell is already NULL", ToastKind::Error);
                    } else {
                        self.submit_cell_edit(
                            table_name,
                            col.name,
                            rowid,
                            SqlValue::Null,
                            cell_value,
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
                if self.readonly {
                    self.toast.push("Read-only database", ToastKind::Error);
                    self.finish_popup();
                    self.dirty = true;
                    return;
                }
                if let Some(PopupKind::TextEditor(state)) = self.popup.as_ref() {
                    if !state.valid {
                        self.toast.push(
                            format!("Invalid value for {}", state.col_type),
                            ToastKind::Error,
                        );
                        self.dirty = true;
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
                    PopupKind::DatetimePicker(s) => Some((
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
                    PopupKind::InsertRow(_) => None,
                    PopupKind::FilterPopup(_) => None,
                    PopupKind::CommandPalette(_) => None,
                    PopupKind::Help(_) => None,
                    PopupKind::Find(_) => None,
                });
                if let Some((table, col, rowid, value, original)) = write_info {
                    self.submit_cell_edit(table, col, rowid, value, original);
                } else {
                    self.toast
                        .push("No value selected to save", ToastKind::Error);
                    self.dirty = true;
                }
            }
            Message::EditCommitted {
                rowid,
                table,
                col,
                original,
            } => {
                self.write_in_flight = false;
                let maybe_fetch = if let Some(ref mut grid) = self.grid {
                    grid.window.rows.clear();
                    grid.window.fetch_in_flight = true;
                    grid.needs_fetch = false;
                    let (off, lim) = grid.window.fetch_params(grid.focused_row as i64);
                    let sort = grid.sort.as_ref().and_then(|s| {
                        grid.columns
                            .get(s.col_idx)
                            .map(|c| (c.name.clone(), s.direction == SortDir::Asc))
                    });
                    Some((
                        grid.table_name.clone(),
                        grid.columns.clone(),
                        sort,
                        off,
                        lim,
                    ))
                } else {
                    None
                };
                let schema_refreshed = self.finish_popup();
                self.undo_stack.push(UndoFrame {
                    op: UndoOp::Update,
                    table,
                    rowid,
                    cols: vec![(col, original)],
                });
                if self.undo_stack.len() > 100 {
                    self.undo_stack.remove(0);
                }
                if !schema_refreshed {
                    if let Some((table, cols, sort, off, lim)) = maybe_fetch {
                        self.spawn_window_fetch(&table, &cols, sort, off, lim);
                    }
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
                let still_active = self.grid.as_ref().is_some_and(|grid| {
                    grid.table_name == table
                        && grid.window.get_rowid(grid.focused_row as i64) == Some(rowid)
                        && grid
                            .columns
                            .get(grid.focused_col)
                            .is_some_and(|column| column.name == col.name)
                });
                if still_active && should_use_value_picker(&values) {
                    self.popup_request_serial = self.popup_request_serial.wrapping_add(1);
                    self.popup = Some(PopupKind::ValuePicker(
                        crate::ui::popup::ValuePickerState::new(
                            table,
                            rowid,
                            col.name,
                            col.col_type,
                            values,
                            original,
                        ),
                    ));
                    self.mode = AppMode::Edit;
                } else if still_active {
                    self.open_text_editor(table, rowid, col.name, col.col_type, original);
                    self.mode = AppMode::Edit;
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
                if self.grid.as_ref().is_some_and(|grid| {
                    grid.table_name == table
                        && grid.window.get_rowid(grid.focused_row as i64) == Some(rowid)
                        && grid
                            .columns
                            .get(grid.focused_col)
                            .is_some_and(|column| column.name == col.name)
                }) {
                    self.toast
                        .push(format!("Distinct lookup failed: {error}"), ToastKind::Error);
                    self.open_text_editor(table, rowid, col.name, col.col_type, original);
                    self.mode = AppMode::Edit;
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
                if let Some(PopupKind::FkPicker(ref mut state)) = self.popup {
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
                let jump_info = self.grid.as_ref().and_then(|g| {
                    let col_idx = g.focused_col;
                    if !g.fk_cols.get(col_idx).copied().unwrap_or(false) {
                        return None;
                    }
                    let table_meta = self.schema.tables.iter().find(|t| t.name == g.table_name)?;
                    let col = g.columns.get(col_idx)?;
                    let fk = table_meta
                        .foreign_keys
                        .iter()
                        .find(|fk| fk.from_col == col.name)?;
                    let source_rowid = g.window.get_rowid(g.focused_row as i64)?;
                    let cell_val = g
                        .window
                        .get_row(g.focused_row as i64)?
                        .get(col_idx)?
                        .clone();
                    Some((
                        g.table_name.clone(),
                        col_idx,
                        fk.to_table.clone(),
                        fk.to_col.clone(),
                        cell_val,
                        source_rowid,
                    ))
                });

                if let Some((from_table, from_col, to_table, to_col, cell_val, source_rowid)) =
                    jump_info
                {
                    let frame = JumpFrame {
                        table: from_table,
                        rowid: source_rowid,
                        col: from_col,
                    };
                    self.navigation_request_serial = self.navigation_request_serial.wrapping_add(1);
                    let request_id = self.navigation_request_serial;
                    let pool = Arc::clone(&self.pool);
                    let tx = self.tx.clone();
                    tokio::task::spawn(async move {
                        let to_table_c = to_table.clone();
                        let to_col_c = to_col.clone();
                        let result =
                            tokio::task::spawn_blocking(move || -> anyhow::Result<Option<i64>> {
                                let conn = pool.get()?;
                                let val = match &cell_val {
                                    SqlValue::Integer(n) => rusqlite::types::Value::Integer(*n),
                                    SqlValue::Text(s) => rusqlite::types::Value::Text(s.clone()),
                                    SqlValue::Real(f) => rusqlite::types::Value::Real(*f),
                                    SqlValue::Blob(bytes) => {
                                        rusqlite::types::Value::Blob(bytes.clone())
                                    }
                                    SqlValue::Null => rusqlite::types::Value::Null,
                                };
                                let columns = crate::db::load_columns(&conn, &to_table_c)?;
                                let identity =
                                    crate::db::load_row_identity(&conn, &to_table_c, &columns)?;
                                let Some(crate::db::schema::RowIdentity::RowidAlias(alias)) =
                                    identity
                                else {
                                    return Ok(None);
                                };
                                let rowid: Option<i64> = conn
                                    .query_row(
                                        &format!(
                                            "SELECT {} FROM {} WHERE {} = ?1 LIMIT 1",
                                            db::query::quote_identifier(&alias),
                                            db::query::quote_identifier(&to_table_c),
                                            db::query::quote_identifier(&to_col_c)
                                        ),
                                        rusqlite::params![val],
                                        |row| row.get(0),
                                    )
                                    .optional()?;
                                Ok(rowid)
                            })
                            .await;
                        match result {
                            Ok(Ok(Some(rowid))) => {
                                let _ = tx.send(Message::FkJumpReady {
                                    request_id,
                                    frame,
                                    table: to_table,
                                    rowid,
                                });
                            }
                            Ok(Ok(None)) => {
                                let _ = tx.send(Message::NavigationFailed {
                                    request_id,
                                    error: "Referenced row was not found or has no navigable rowid"
                                        .to_string(),
                                });
                            }
                            Ok(Err(error)) => {
                                let _ = tx.send(Message::NavigationFailed {
                                    request_id,
                                    error: error.to_string(),
                                });
                            }
                            Err(error) => {
                                let _ = tx.send(Message::NavigationFailed {
                                    request_id,
                                    error: error.to_string(),
                                });
                            }
                        }
                    });
                }
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
                self.navigation_request_serial = self.navigation_request_serial.wrapping_add(1);
                self.jump_to_rowid(table, rowid, col)
            }
            Message::CycleSort => {
                self.navigation_request_serial = self.navigation_request_serial.wrapping_add(1);
                let maybe_fetch = if let Some(ref mut grid) = self.grid {
                    let col_idx = grid.focused_col;
                    grid.sort = match &grid.sort {
                        None => Some(SortSpec {
                            col_idx,
                            direction: SortDir::Asc,
                        }),
                        Some(s) if s.col_idx == col_idx => match s.direction {
                            SortDir::Asc => Some(SortSpec {
                                col_idx,
                                direction: SortDir::Desc,
                            }),
                            SortDir::Desc => None,
                        },
                        Some(_) => Some(SortSpec {
                            col_idx,
                            direction: SortDir::Asc,
                        }),
                    };
                    grid.viewport_start = 0;
                    grid.focused_row = 0;
                    grid.window.rows.clear();
                    grid.window.offset = 0;
                    grid.window.fetch_in_flight = true;
                    grid.needs_fetch = false;
                    let (off, lim) = grid.window.fetch_params(0);
                    let sort = grid.sort.as_ref().and_then(|s| {
                        grid.columns
                            .get(s.col_idx)
                            .map(|c| (c.name.clone(), s.direction == SortDir::Asc))
                    });
                    Some((
                        grid.table_name.clone(),
                        grid.columns.clone(),
                        sort,
                        off,
                        lim,
                    ))
                } else {
                    None
                };
                if let Some((table, cols, sort, off, lim)) = maybe_fetch {
                    self.spawn_window_fetch(&table, &cols, sort, off, lim);
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
                let maybe_fetch = if let Some(ref mut grid) = self.grid {
                    if grid.table_name == table {
                        grid.commit_row_selection();
                        grid.scroll_to_row(offset);
                        if grid.needs_fetch && !grid.window.fetch_in_flight {
                            grid.window.fetch_in_flight = true;
                            grid.needs_fetch = false;
                            let (off, lim) = grid.window.fetch_params(grid.focused_row as i64);
                            let sort = grid.sort.as_ref().and_then(|s| {
                                grid.columns
                                    .get(s.col_idx)
                                    .map(|c| (c.name.clone(), s.direction == SortDir::Asc))
                            });
                            Some((
                                grid.table_name.clone(),
                                grid.columns.clone(),
                                sort,
                                off,
                                lim,
                            ))
                        } else {
                            None
                        }
                    } else {
                        None
                    }
                } else {
                    None
                };
                if let Some((t, cols, sort, off, lim)) = maybe_fetch {
                    self.spawn_window_fetch(&t, &cols, sort, off, lim);
                }
                self.dirty = true;
            }
            Message::JumpToLetter(letter) => {
                if let Some(ref grid) = self.grid {
                    let sort = grid.sort.as_ref().cloned();
                    if let Some(sort) = sort {
                        if let Some(col) = grid.columns.get(sort.col_idx).cloned() {
                            let pool = Arc::clone(&self.pool);
                            let tx = self.tx.clone();
                            let table = grid.table_name.clone();
                            let col_name = col.name.clone();
                            let dir_asc = sort.direction == SortDir::Asc;
                            let filter = grid.filter.clone();
                            let letter_uc = letter.to_uppercase().next().unwrap_or(letter);
                            let table_inner = table.clone();
                            self.navigation_request_serial =
                                self.navigation_request_serial.wrapping_add(1);
                            let request_id = self.navigation_request_serial;
                            tokio::task::spawn(async move {
                                let result =
                                    tokio::task::spawn_blocking(move || -> anyhow::Result<i64> {
                                        let conn = pool.get()?;
                                        count_rows_before_letter(
                                            &conn,
                                            &table_inner,
                                            &col_name,
                                            dir_asc,
                                            letter,
                                            letter_uc,
                                            &filter,
                                        )
                                    })
                                    .await;
                                match result {
                                    Ok(Ok(offset)) => {
                                        let _ = tx.send(Message::JumpToSortedOffset {
                                            request_id,
                                            table,
                                            offset,
                                        });
                                    }
                                    Ok(Err(error)) => {
                                        let _ = tx.send(Message::NavigationFailed {
                                            request_id,
                                            error: error.to_string(),
                                        });
                                    }
                                    Err(error) => {
                                        let _ = tx.send(Message::NavigationFailed {
                                            request_id,
                                            error: error.to_string(),
                                        });
                                    }
                                }
                            });
                        }
                    }
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
                if let Some(ref grid) = self.grid {
                    let col_idx = grid.focused_col;
                    if let Some(col) = grid.columns.get(col_idx) {
                        let col_name = col.name.clone();
                        let col_type = col.col_type.clone();
                        let col_filter = grid
                            .filter
                            .columns
                            .get(&col_name)
                            .cloned()
                            .unwrap_or_default();
                        self.popup = Some(PopupKind::FilterPopup(FilterPopupState::new(
                            col_name, col_type, col_filter,
                        )));
                        self.mode = AppMode::Edit;
                    }
                }
                self.dirty = true;
            }
            Message::ApplyFilter => {
                self.navigation_request_serial = self.navigation_request_serial.wrapping_add(1);
                if let Some(PopupKind::FilterPopup(state)) = self.popup.take() {
                    if let Some(ref mut grid) = self.grid {
                        grid.filter
                            .columns
                            .insert(state.col_name.clone(), state.col_filter);
                        grid.viewport_start = 0;
                        grid.focused_row = 0;
                        grid.window.rows.clear();
                        grid.window.offset = 0;
                        let table = grid.table_name.clone();
                        let cols = grid.columns.clone();
                        let sort = grid.sort.as_ref().and_then(|s| {
                            grid.columns
                                .get(s.col_idx)
                                .map(|c| (c.name.clone(), s.direction == SortDir::Asc))
                        });
                        let (off, lim) = grid.window.fetch_params(0);
                        grid.window.fetch_in_flight = true;
                        let filter = grid.filter.clone();
                        let db_path = self.db_path.clone();
                        if let Err(error) = crate::filter::save_filter(&filter, &db_path, &table) {
                            self.toast
                                .push(format!("Could not save filter: {error}"), ToastKind::Error);
                        }
                        self.spawn_window_fetch_with_filter(&table, &cols, sort, off, lim, filter);
                    }
                    self.finish_popup();
                }
                self.dirty = true;
            }
            Message::ClearFilters => {
                self.navigation_request_serial = self.navigation_request_serial.wrapping_add(1);
                if let Some(ref mut grid) = self.grid {
                    grid.filter = crate::filter::FilterSet::default();
                    grid.viewport_start = 0;
                    grid.focused_row = 0;
                    grid.window.rows.clear();
                    grid.window.offset = 0;
                    let table = grid.table_name.clone();
                    let cols = grid.columns.clone();
                    let sort = grid.sort.as_ref().and_then(|s| {
                        grid.columns
                            .get(s.col_idx)
                            .map(|c| (c.name.clone(), s.direction == SortDir::Asc))
                    });
                    let (off, lim) = grid.window.fetch_params(0);
                    grid.window.fetch_in_flight = true;
                    let db_path = self.db_path.clone();
                    let empty_filter = crate::filter::FilterSet::default();
                    if let Err(error) = crate::filter::save_filter(&empty_filter, &db_path, &table)
                    {
                        self.toast.push(
                            format!("Could not clear saved filter: {error}"),
                            ToastKind::Error,
                        );
                    }
                    self.spawn_window_fetch_with_filter(
                        &table,
                        &cols,
                        sort,
                        off,
                        lim,
                        empty_filter,
                    );
                }
                self.finish_popup();
                self.dirty = true;
            }
            Message::InsertRow => {
                if self.readonly {
                    self.toast.push("Read-only database", ToastKind::Error);
                    return;
                }
                let insert_is_undoable = self.grid.as_ref().is_some_and(|grid| {
                    self.schema
                        .tables
                        .iter()
                        .find(|table| table.name == grid.table_name)
                        .and_then(|table| table.row_identity.as_ref())
                        .is_some_and(|identity| {
                            matches!(identity, crate::db::schema::RowIdentity::RowidAlias(_))
                        })
                });
                if !insert_is_undoable {
                    self.toast.push(
                        "This table has no rowid that can support insert undo",
                        ToastKind::Error,
                    );
                    self.dirty = true;
                    return;
                }
                self.popup_request_serial = self.popup_request_serial.wrapping_add(1);
                if let Some(ref mut grid) = self.grid {
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
                    let focused_row = grid.focused_row;
                    grid.focus_cell(focused_row, state.selected);
                    Self::ensure_inline_insert_visible(grid, insert_position);
                    self.popup = Some(PopupKind::InsertRow(state));
                    self.mode = AppMode::Edit;
                    self.toast.push("Alt-Enter commits", ToastKind::Info);
                }
                self.dirty = true;
            }
            Message::CommitInsertRow => {
                if self.readonly {
                    self.toast.push("Read-only database", ToastKind::Error);
                    return;
                }
                if self.write_in_flight {
                    self.toast
                        .push("Insert is already running", ToastKind::Info);
                    self.dirty = true;
                    return;
                }
                let insert_spec = self.popup.as_ref().and_then(|popup| match popup {
                    PopupKind::InsertRow(state) => match state.build_insert_values() {
                        Ok(values) => Some((state.table.clone(), values)),
                        Err(err) => {
                            self.toast.push(err.to_string(), ToastKind::Error);
                            None
                        }
                    },
                    _ => None,
                });
                if let Some((table, values)) = insert_spec {
                    self.write_in_flight = true;
                    let pool = Arc::clone(&self.pool);
                    let tx = self.tx.clone();
                    let table_c = table.clone();
                    tokio::task::spawn(async move {
                        let tx_err = tx.clone();
                        let result = tokio::task::spawn_blocking(move || -> anyhow::Result<i64> {
                            let conn = pool.get()?;
                            crate::db::write::insert_row(&conn, &table_c, &values)
                        })
                        .await;
                        match result {
                            Ok(Ok(rowid)) => {
                                let _ = tx.send(Message::RowInserted { table, rowid });
                            }
                            Ok(Err(err)) => {
                                let _ = tx_err.send(Message::EditFailed(err.to_string()));
                            }
                            Err(err) => {
                                let _ = tx_err.send(Message::EditFailed(err.to_string()));
                            }
                        }
                    });
                }
                self.dirty = true;
            }
            Message::RowInserted { table, rowid } => {
                self.write_in_flight = false;
                self.grid_request_serial.fetch_add(1, Ordering::AcqRel);
                if let Some(ref mut grid) = self.grid {
                    if grid.table_name == table {
                        self.undo_stack.push(UndoFrame {
                            op: UndoOp::Insert,
                            table: table.clone(),
                            rowid,
                            cols: Vec::new(),
                        });
                        if self.undo_stack.len() > 100 {
                            self.undo_stack.remove(0);
                        }
                        grid.window.rows.clear();
                        grid.window.rowids.clear();
                        grid.window.fetch_in_flight = false;
                        grid.window.total_rows += 1;
                        grid.needs_fetch = true;
                    }
                }
                self.finish_popup();
                self.toast.push("Row inserted", ToastKind::Success);
                self.dirty = true;
            }
            Message::DeleteRow => {
                if self.readonly {
                    self.toast.push("Read-only database", ToastKind::Error);
                    return;
                }
                enum DeleteTarget {
                    CurrentRow {
                        row_num: usize,
                        table: String,
                        rowid: i64,
                    },
                    SelectedRows {
                        table: String,
                        row_offsets: Vec<i64>,
                        count: usize,
                        sort: Option<(String, bool)>,
                        filter: crate::filter::FilterSet,
                    },
                    AllRows {
                        table: String,
                    },
                }

                let delete_target = self.grid.as_ref().and_then(|grid| {
                    let table = grid.table_name.clone();
                    let sort = grid.sort.as_ref().and_then(|s| {
                        grid.columns
                            .get(s.col_idx)
                            .map(|c| (c.name.clone(), s.direction == SortDir::Asc))
                    });
                    match &grid.row_selection {
                        RowSelection::All => Some(DeleteTarget::AllRows { table }),
                        RowSelection::Rows(_) => {
                            let row_offsets = grid
                                .selected_rows()
                                .into_iter()
                                .map(|row| row as i64)
                                .collect::<Vec<_>>();
                            (!row_offsets.is_empty()).then_some(DeleteTarget::SelectedRows {
                                table,
                                count: row_offsets.len(),
                                row_offsets,
                                sort,
                                filter: grid.filter.clone(),
                            })
                        }
                        RowSelection::None => Some(DeleteTarget::CurrentRow {
                            row_num: grid.focused_row + 1,
                            table,
                            rowid: grid.window.get_rowid(grid.focused_row as i64)?,
                        }),
                    }
                });

                if let Some(delete_target) = delete_target {
                    match delete_target {
                        DeleteTarget::CurrentRow {
                            row_num,
                            table,
                            rowid,
                        } => {
                            let msg = format!("Delete row #{}? [y/n]", row_num);
                            self.pending_confirm = Some(PendingConfirm {
                                message: msg,
                                kind: ConfirmKind::DeleteRow { table, rowid },
                                created: std::time::Instant::now(),
                                timeout_secs: 5,
                            });
                        }
                        DeleteTarget::SelectedRows {
                            table,
                            row_offsets,
                            count,
                            sort,
                            filter,
                        } => {
                            let (where_clause, where_params) = match filter_to_sql(&filter) {
                                Ok(predicate) => predicate,
                                Err(error) => {
                                    self.toast.push(error.to_string(), ToastKind::Error);
                                    return;
                                }
                            };
                            let conn = match self.pool.get() {
                                Ok(conn) => conn,
                                Err(error) => {
                                    self.toast.push(error.to_string(), ToastKind::Error);
                                    return;
                                }
                            };
                            let rowids = match db::fetch_rowids_at_offsets(
                                &conn,
                                &table,
                                &row_offsets,
                                sort.as_ref().map(|(column, asc)| (column.as_str(), *asc)),
                                &where_clause,
                                &where_params,
                            ) {
                                Ok(rowids) if rowids.len() == count => rowids,
                                Ok(_) => {
                                    self.toast.push(
                                        "Some selected rows no longer exist",
                                        ToastKind::Error,
                                    );
                                    return;
                                }
                                Err(error) => {
                                    self.toast.push(error.to_string(), ToastKind::Error);
                                    return;
                                }
                            };
                            let noun = if count == 1 { "row" } else { "rows" };
                            let msg = format!("Delete {} selected {}? [y/n]", count, noun);
                            self.pending_confirm = Some(PendingConfirm {
                                message: msg,
                                kind: ConfirmKind::DeleteSelectedRows { table, rowids },
                                created: std::time::Instant::now(),
                                timeout_secs: 5,
                            });
                        }
                        DeleteTarget::AllRows { table } => {
                            let Some(total_rows) = self.count_table_rows(&table) else {
                                self.dirty = true;
                                return;
                            };
                            if total_rows <= 0 {
                                self.toast.push("Table is empty", ToastKind::Info);
                                self.dirty = true;
                                return;
                            }
                            let msg =
                                format!("Delete all {} rows from {}? [y/n]", total_rows, table);
                            self.pending_confirm = Some(PendingConfirm {
                                message: msg,
                                kind: ConfirmKind::ClearTable { table },
                                created: std::time::Instant::now(),
                                timeout_secs: 5,
                            });
                        }
                    }
                }
                self.dirty = true;
            }
            Message::ConfirmDelete => {
                if self.write_in_flight {
                    self.toast
                        .push("A database write is already running", ToastKind::Info);
                    self.dirty = true;
                    return;
                }
                if let Some(confirm) = self.pending_confirm.take() {
                    self.write_in_flight = true;
                    match confirm.kind {
                        ConfirmKind::DeleteRow { table, rowid } => {
                            let pool = Arc::clone(&self.pool);
                            let tx_ch = self.tx.clone();
                            let columns = self
                                .grid
                                .as_ref()
                                .map(|g| g.columns.clone())
                                .unwrap_or_default();
                            let table_c = table.clone();
                            tokio::task::spawn(async move {
                                let tx_err = tx_ch.clone();
                                let result =
                                    tokio::task::spawn_blocking(move || -> anyhow::Result<()> {
                                        let conn = pool.get()?;
                                        let cols = crate::db::write::delete_row_with_backup(
                                            &conn, &table_c, &columns, rowid,
                                        )?;
                                        let _ = tx_ch.send(Message::RowDeleted {
                                            table: table_c,
                                            rowid,
                                            cols,
                                        });
                                        Ok(())
                                    })
                                    .await;
                                match result {
                                    Ok(Ok(())) => {}
                                    Ok(Err(error)) => {
                                        let _ = tx_err.send(Message::EditFailed(error.to_string()));
                                    }
                                    Err(error) => {
                                        let _ = tx_err.send(Message::EditFailed(error.to_string()));
                                    }
                                }
                            });
                        }
                        ConfirmKind::DeleteSelectedRows { table, rowids } => {
                            let pool = Arc::clone(&self.pool);
                            let tx = self.tx.clone();
                            let table_c = table.clone();
                            tokio::task::spawn(async move {
                                let tx_err = tx.clone();
                                let result = tokio::task::spawn_blocking(
                                    move || -> anyhow::Result<usize> {
                                        let conn = pool.get()?;
                                        crate::db::write::delete_rows_by_rowids(
                                            &conn, &table_c, &rowids,
                                        )
                                    },
                                )
                                .await;
                                match result {
                                    Ok(Ok(count)) => {
                                        let _ = tx.send(Message::RowsDeleted { table, count });
                                    }
                                    Ok(Err(err)) => {
                                        let _ = tx_err.send(Message::EditFailed(err.to_string()));
                                    }
                                    Err(err) => {
                                        let _ = tx_err.send(Message::EditFailed(err.to_string()));
                                    }
                                }
                            });
                        }
                        ConfirmKind::ClearTable { table } => {
                            let pool = Arc::clone(&self.pool);
                            let tx = self.tx.clone();
                            let table_c = table.clone();
                            tokio::task::spawn(async move {
                                let tx_err = tx.clone();
                                let result = tokio::task::spawn_blocking(
                                    move || -> anyhow::Result<usize> {
                                        let conn = pool.get()?;
                                        crate::db::write::clear_table(&conn, &table_c)
                                    },
                                )
                                .await;
                                match result {
                                    Ok(Ok(count)) => {
                                        let _ = tx.send(Message::RowsDeleted { table, count });
                                    }
                                    Ok(Err(err)) => {
                                        let _ = tx_err.send(Message::EditFailed(err.to_string()));
                                    }
                                    Err(err) => {
                                        let _ = tx_err.send(Message::EditFailed(err.to_string()));
                                    }
                                }
                            });
                        }
                    }
                }
                self.dirty = true;
            }
            Message::RowDeleted { table, rowid, cols } => {
                self.write_in_flight = false;
                self.grid_request_serial.fetch_add(1, Ordering::AcqRel);
                self.undo_stack.push(UndoFrame {
                    op: UndoOp::Delete,
                    table: table.clone(),
                    rowid,
                    cols,
                });
                if self.undo_stack.len() > 100 {
                    self.undo_stack.remove(0);
                }
                if let Some(ref mut grid) = self.grid {
                    if grid.table_name == table {
                        grid.clear_row_selection();
                        grid.window.rows.clear();
                        grid.window.rowids.clear();
                        grid.window.fetch_in_flight = false;
                        grid.window.total_rows = grid.window.total_rows.saturating_sub(1);
                        if grid.window.total_rows <= 0 {
                            grid.focused_row = 0;
                            grid.viewport_start = 0;
                        } else {
                            let max_row = (grid.window.total_rows - 1) as usize;
                            if grid.focused_row > max_row {
                                grid.focused_row = max_row;
                            }
                            let max_start =
                                (grid.window.total_rows - grid.window.viewport_rows as i64).max(0);
                            if grid.viewport_start > max_start {
                                grid.viewport_start = max_start;
                            }
                        }
                        grid.needs_fetch = true;
                    }
                }
                self.toast.push("Row deleted", ToastKind::Success);
                self.dirty = true;
            }
            Message::RowsDeleted { table, count } => {
                self.write_in_flight = false;
                self.grid_request_serial.fetch_add(1, Ordering::AcqRel);
                if let Some(ref mut grid) = self.grid {
                    if grid.table_name == table {
                        grid.clear_row_selection();
                        grid.window.rows.clear();
                        grid.window.rowids.clear();
                        grid.window.fetch_in_flight = false;
                        grid.window.total_rows =
                            grid.window.total_rows.saturating_sub(count as i64);
                        if grid.window.total_rows <= 0 {
                            grid.focused_row = 0;
                            grid.viewport_start = 0;
                        } else {
                            let max_row = (grid.window.total_rows - 1) as usize;
                            if grid.focused_row > max_row {
                                grid.focused_row = max_row;
                            }
                            let max_start =
                                (grid.window.total_rows - grid.window.viewport_rows as i64).max(0);
                            if grid.viewport_start > max_start {
                                grid.viewport_start = max_start;
                            }
                        }
                        grid.needs_fetch = true;
                    }
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
                if self.write_in_flight {
                    self.toast
                        .push("A database write is already running", ToastKind::Info);
                    return;
                }
                let Some(frame) = self.undo_stack.pop() else {
                    self.toast.push("Nothing to undo", ToastKind::Info);
                    self.dirty = true;
                    return;
                };
                self.write_in_flight = true;
                let pool = Arc::clone(&self.pool);
                let tx = self.tx.clone();
                let work = frame.clone();
                tokio::task::spawn(async move {
                    let failure_frame = frame.clone();
                    let table = frame.table.clone();
                    let message = match &frame.op {
                        UndoOp::Update => format!("Undo: restored row {}", frame.rowid),
                        UndoOp::Insert => format!("Undo: deleted inserted row {}", frame.rowid),
                        UndoOp::Delete => format!("Undo: restored deleted row {}", frame.rowid),
                    };
                    let result = tokio::task::spawn_blocking(move || -> anyhow::Result<()> {
                        let conn = pool.get()?;
                        match work.op {
                            UndoOp::Update => {
                                for (column, value) in &work.cols {
                                    crate::db::write::commit_cell_edit(
                                        &conn,
                                        &work.table,
                                        column,
                                        work.rowid,
                                        value,
                                    )?;
                                }
                            }
                            UndoOp::Insert => {
                                crate::db::write::delete_row(&conn, &work.table, work.rowid)?;
                            }
                            UndoOp::Delete => {
                                crate::db::write::reinsert_row(
                                    &conn,
                                    &work.table,
                                    work.rowid,
                                    &work.cols,
                                )?;
                            }
                        }
                        Ok(())
                    })
                    .await;
                    match result {
                        Ok(Ok(())) => {
                            let _ = tx.send(Message::UndoCompleted { table, message });
                        }
                        Ok(Err(error)) => {
                            let _ = tx.send(Message::UndoFailed {
                                frame: failure_frame,
                                error: error.to_string(),
                            });
                        }
                        Err(error) => {
                            let _ = tx.send(Message::UndoFailed {
                                frame: failure_frame,
                                error: error.to_string(),
                            });
                        }
                    }
                });
                self.dirty = true;
            }
            Message::UndoCompleted { table, message } => {
                self.write_in_flight = false;
                self.grid_request_serial.fetch_add(1, Ordering::AcqRel);
                if let Some(grid) = self.grid.as_mut().filter(|grid| grid.table_name == table) {
                    grid.window.rows.clear();
                    grid.window.rowids.clear();
                    grid.window.fetch_in_flight = false;
                    grid.needs_fetch = true;
                }
                self.toast.push(message, ToastKind::Info);
                self.dirty = true;
            }
            Message::UndoFailed { frame, error } => {
                self.write_in_flight = false;
                self.undo_stack.push(frame);
                self.toast
                    .push(format!("Undo failed: {error}"), ToastKind::Error);
                self.dirty = true;
            }
            Message::OpenCommandPalette => {
                let table_names = self.schema.tables.iter().map(|t| t.name.clone()).collect();
                let state = CommandPaletteState::new(table_names);
                self.popup_request_serial = self.popup_request_serial.wrapping_add(1);
                self.popup = Some(PopupKind::CommandPalette(state));
                self.mode = AppMode::Edit;
                self.dirty = true;
            }
            Message::OpenHelp => {
                self.popup_request_serial = self.popup_request_serial.wrapping_add(1);
                self.popup = Some(PopupKind::Help(HelpState::new()));
                self.mode = AppMode::Edit;
                self.dirty = true;
            }
            Message::ExecuteCommand(cmd) => {
                self.execute_palette_command(cmd);
                self.dirty = true;
            }
            Message::ExportDone {
                format: _,
                path,
                count,
            } => {
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
                let pool = Arc::clone(&self.pool);
                let tx = self.tx.clone();
                tokio::task::spawn(async move {
                    let result = tokio::task::spawn_blocking(move || {
                        let conn = pool.get()?;
                        crate::db::load_schema(&conn)
                    })
                    .await;
                    match result {
                        Ok(Ok(schema)) => {
                            let _ = tx.send(Message::SchemaReady(schema));
                        }
                        Ok(Err(error)) => {
                            let _ = tx.send(Message::SchemaLoadFailed {
                                external: false,
                                error: error.to_string(),
                            });
                        }
                        Err(error) => {
                            let _ = tx.send(Message::SchemaLoadFailed {
                                external: false,
                                error: error.to_string(),
                            });
                        }
                    }
                });
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
                    self.file_check_in_flight = false;
                    if self.file_change_pending {
                        self.file_change_pending = false;
                        let _ = self.tx.send(Message::FileChanged);
                    }
                }
                self.toast
                    .push(format!("Schema reload failed: {error}"), ToastKind::Error);
                self.dirty = true;
            }
            Message::CopyCell => {
                let text = self.grid.as_ref().and_then(|g| {
                    let abs_row = g.focused_row as i64;
                    let col_idx = g.focused_col;
                    g.window.get_row(abs_row)?.get(col_idx).map(|v| match v {
                        SqlValue::Null => "NULL".to_string(),
                        SqlValue::Integer(n) => n.to_string(),
                        SqlValue::Real(f) => f.to_string(),
                        SqlValue::Text(s) => s.clone(),
                        SqlValue::Blob(b) => format!("<blob {} bytes>", b.len()),
                    })
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

                // Spawn schema probe.
                let pool = Arc::clone(&self.pool);
                let tx = self.tx.clone();
                tokio::task::spawn(async move {
                    let result = tokio::task::spawn_blocking(move || {
                        let conn = pool.get()?;
                        crate::db::load_schema(&conn)
                    })
                    .await;
                    match result {
                        Ok(Ok(schema)) => {
                            let _ = tx.send(Message::ExternalRefresh(schema));
                        }
                        Ok(Err(error)) => {
                            let _ = tx.send(Message::SchemaLoadFailed {
                                external: true,
                                error: error.to_string(),
                            });
                        }
                        Err(error) => {
                            let _ = tx.send(Message::SchemaLoadFailed {
                                external: true,
                                error: error.to_string(),
                            });
                        }
                    }
                });

                // Schedule data refresh, but not while a popup is open.
                if self.mode == AppMode::Edit {
                    self.pending_external_refresh = true;
                } else if let Some(ref mut grid) = self.grid {
                    self.grid_request_serial.fetch_add(1, Ordering::AcqRel);
                    grid.window.fetch_in_flight = false;
                    grid.needs_fetch = true;
                } else if let Some(table) = self
                    .active_tab
                    .and_then(|index| self.open_tabs.get(index))
                    .map(|tab| tab.table_name.clone())
                {
                    self.request_table_view(&table);
                }
                self.dirty = true;
            }
            Message::ExternalRefresh(new_schema) => {
                self.file_check_in_flight = false;

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
                if self.file_change_pending {
                    self.file_change_pending = false;
                    let _ = self.tx.send(Message::FileChanged);
                }
                self.dirty = true;
            }
            Message::OpenFind => {
                let Some(ref grid) = self.grid else { return };
                let table_name = grid.table_name.clone();
                let columns = grid.columns.clone();
                let sort = grid.sort.as_ref().map(|s| {
                    (
                        grid.columns[s.col_idx].name.clone(),
                        s.direction == crate::grid::SortDir::Asc,
                    )
                });
                let filter = grid.filter.clone();

                let find_state = FindState::new(table_name.clone(), columns.clone());
                self.popup = Some(PopupKind::Find(find_state));
                self.mode = AppMode::Edit;
                self.dirty = true;
                self.popup_request_serial = self.popup_request_serial.wrapping_add(1);
                let request_id = self.popup_request_serial;

                let pool = Arc::clone(&self.pool);
                let tx = self.tx.clone();
                let response_table = table_name.clone();
                tokio::task::spawn(async move {
                    let result = tokio::task::spawn_blocking(
                        move || -> anyhow::Result<Vec<Vec<SqlValue>>> {
                            let conn = pool.get()?;
                            let (where_clause, where_params) = filter_to_sql(&filter)?;
                            let order_by = sort.as_ref().map(|(s, b)| (s.as_str(), *b));
                            let rows = db::fetch_rows(
                                &conn,
                                db::RowFetch {
                                    table: &table_name,
                                    columns: &columns,
                                    offset: 0,
                                    limit: 10_000,
                                    order_by,
                                    where_clause: &where_clause,
                                    where_params: &where_params,
                                },
                            )?;
                            Ok(rows)
                        },
                    )
                    .await;
                    match result {
                        Ok(Ok(rows)) => {
                            let _ = tx.send(Message::FindReady {
                                request_id,
                                table: response_table,
                                rows,
                            });
                        }
                        Ok(Err(error)) => {
                            let _ = tx.send(Message::FindFailed {
                                request_id,
                                table: response_table,
                                error: error.to_string(),
                            });
                        }
                        Err(error) => {
                            let _ = tx.send(Message::FindFailed {
                                request_id,
                                table: response_table,
                                error: error.to_string(),
                            });
                        }
                    }
                });
            }
            Message::FindReady {
                request_id,
                table,
                rows,
            } => {
                if request_id != self.popup_request_serial {
                    return;
                }
                if let Some(PopupKind::Find(ref mut state)) = self.popup {
                    if state.table_name == table {
                        state.rows = rows;
                        state.loading = false;
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
                let hit = if let Some(PopupKind::Find(ref state)) = self.popup {
                    let hits = state.visible_hits();
                    hits.into_iter()
                        .nth(state.selected)
                        .map(|h| (h.abs_row_index, h.first_match_col))
                } else {
                    None
                };

                let maybe_fetch =
                    if let (Some((abs_row, col)), Some(ref mut grid)) = (hit, self.grid.as_mut()) {
                        grid.focus_cell(abs_row, col);
                        if grid.needs_fetch && !grid.window.fetch_in_flight {
                            grid.window.fetch_in_flight = true;
                            grid.needs_fetch = false;
                            let (off, lim) = grid.window.fetch_params(grid.focused_row as i64);
                            let sort = grid.sort.as_ref().and_then(|s| {
                                grid.columns
                                    .get(s.col_idx)
                                    .map(|c| (c.name.clone(), s.direction == SortDir::Asc))
                            });
                            Some((
                                grid.table_name.clone(),
                                grid.columns.clone(),
                                sort,
                                off,
                                lim,
                            ))
                        } else {
                            None
                        }
                    } else {
                        None
                    };

                let schema_refreshed = self.finish_popup();
                self.dirty = true;

                if !schema_refreshed {
                    if let Some((table, cols, sort, off, lim)) = maybe_fetch {
                        self.spawn_window_fetch(&table, &cols, sort, off, lim);
                    }
                }
            }
        }
    }

    pub fn view(&mut self, frame: &mut ratatui::Frame) {
        crate::ui::render(frame, self);
    }

    fn execute_palette_command(&mut self, cmd: PaletteCommand) {
        match cmd {
            PaletteCommand::ExportCsv | PaletteCommand::ExportJson | PaletteCommand::ExportSql => {
                if let Some(ref grid) = self.grid {
                    let table = grid.table_name.clone();
                    let columns = grid.columns.clone();
                    let sort = grid.sort.clone();
                    let filter = grid.filter.clone();
                    let pool = Arc::clone(&self.pool);
                    let tx = self.tx.clone();
                    let format = match &cmd {
                        PaletteCommand::ExportCsv => "csv",
                        PaletteCommand::ExportJson => "json",
                        PaletteCommand::ExportSql => "sql",
                        _ => "csv",
                    }
                    .to_string();
                    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
                    let safe_table = table
                        .chars()
                        .map(|character| {
                            if character.is_ascii_alphanumeric()
                                || character == '-'
                                || character == '_'
                            {
                                character
                            } else {
                                '_'
                            }
                        })
                        .collect::<String>();
                    let export_id = std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .map_or(0, |duration| duration.as_nanos());
                    let export_path = format!("{home}/sqview_{safe_table}_{export_id}.{format}");
                    let export_path_clone = export_path.clone();
                    tokio::task::spawn(async move {
                        let format_c = format.clone();
                        let result = tokio::task::spawn_blocking(move || -> anyhow::Result<u64> {
                            let conn = pool.get()?;
                            let path = std::path::Path::new(&export_path);
                            match format_c.as_str() {
                                "csv" => crate::export::export_csv(
                                    &conn, &table, &columns, &filter, &sort, path,
                                ),
                                "json" => crate::export::export_json(
                                    &conn, &table, &columns, &filter, &sort, path,
                                ),
                                "sql" => crate::export::export_sql(
                                    &conn, &table, &columns, &filter, &sort, path,
                                ),
                                _ => Err(anyhow::anyhow!("unknown format")),
                            }
                        })
                        .await;
                        if let Ok(Ok(count)) = result {
                            let _ = tx.send(Message::ExportDone {
                                format,
                                path: export_path_clone,
                                count,
                            });
                        } else {
                            let error = match result {
                                Ok(Err(error)) => error.to_string(),
                                Err(error) => error.to_string(),
                                Ok(Ok(_)) => unreachable!(),
                            };
                            let _ = tx.send(Message::ExportFailed(error));
                        }
                    });
                }
            }
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
            PaletteCommand::ResetColumnWidths => {
                if let Some(ref mut grid) = self.grid {
                    grid.manual_widths.clear();
                    grid.recompute_col_widths(grid.avail_col_width);
                }
                self.toast.push("Column widths reset", ToastKind::Info);
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

        let sort = grid.sort.as_ref().and_then(|s| {
            grid.columns
                .get(s.col_idx)
                .map(|c| (c.name.clone(), s.direction == SortDir::Asc))
        });
        let (where_clause, where_params) = filter_to_sql(&grid.filter)?;
        let conn = self.pool.get()?;

        let selected_offsets = grid.selected_rows();
        let copying_selection = grid.has_row_selection();
        let rows = if matches!(&grid.row_selection, RowSelection::All) {
            db::fetch_rows(
                &conn,
                db::RowFetch {
                    table: &grid.table_name,
                    columns: &grid.columns,
                    offset: 0,
                    limit: grid.window.total_rows.max(0),
                    order_by: sort.as_ref().map(|(col, asc)| (col.as_str(), *asc)),
                    where_clause: &where_clause,
                    where_params: &where_params,
                },
            )?
        } else if copying_selection {
            let offsets = selected_offsets
                .into_iter()
                .map(|offset| offset as i64)
                .collect::<Vec<_>>();
            db::fetch_rows_at_offsets(
                &conn,
                &grid.table_name,
                &grid.columns,
                &offsets,
                sort.as_ref().map(|(col, asc)| (col.as_str(), *asc)),
                &where_clause,
                &where_params,
            )?
        } else {
            db::fetch_rows(
                &conn,
                db::RowFetch {
                    table: &grid.table_name,
                    columns: &grid.columns,
                    offset: grid.focused_row as i64,
                    limit: 1,
                    order_by: sort.as_ref().map(|(col, asc)| (col.as_str(), *asc)),
                    where_clause: &where_clause,
                    where_params: &where_params,
                },
            )?
        };

        if rows.is_empty() {
            return Ok(None);
        }

        let json = if copying_selection {
            let json_rows = rows
                .iter()
                .map(|row| row_to_json_value(&grid.columns, row))
                .collect::<Vec<_>>();
            serde_json::to_string(&json_rows)?
        } else {
            serde_json::to_string(&row_to_json_value(&grid.columns, &rows[0]))?
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

    fn handle_mouse(&mut self, mouse: crossterm::event::MouseEvent) {
        use crossterm::event::{KeyModifiers, MouseButton, MouseEventKind};

        if matches!(self.popup, Some(PopupKind::FilterPopup(_))) {
            self.handle_filter_popup_mouse(mouse);
            return;
        }

        if matches!(self.popup, Some(PopupKind::TextEditor(_))) {
            self.handle_text_editor_mouse(mouse);
            return;
        }

        if self.popup.is_some() || matches!(self.mode, AppMode::Edit) {
            return;
        }

        let x = mouse.column;
        let y = mouse.row;
        match mouse.kind {
            MouseEventKind::ScrollDown => {
                if self.mouse_scroll_panel(x, y, 3, true) {
                    self.dirty = true;
                }
                return;
            }
            MouseEventKind::ScrollUp => {
                if self.mouse_scroll_panel(x, y, 3, false) {
                    self.dirty = true;
                }
                return;
            }
            MouseEventKind::Drag(MouseButton::Left) => {
                if self.drag_grid_scrollbar(y) {
                    self.dirty = true;
                }
                return;
            }
            MouseEventKind::Up(MouseButton::Left) => {
                self.grid_scrollbar_drag = None;
                return;
            }
            _ => {}
        }

        let middle_click = matches!(mouse.kind, MouseEventKind::Down(MouseButton::Middle));
        let left_click = matches!(mouse.kind, MouseEventKind::Down(MouseButton::Left));
        let ctrl_click = mouse.modifiers.contains(KeyModifiers::CONTROL);
        if !left_click && !middle_click {
            return;
        }

        if self.tabbar_area.height > 0 {
            if let Some(action) =
                crate::ui::tabbar::hit_test(self.tabbar_area, self, x, y, middle_click)
            {
                self.focus = FocusPane::Grid;
                match action {
                    TabMouseAction::Activate(idx) => {
                        let _ = self.tx.send(Message::ActivateTab(idx));
                    }
                    TabMouseAction::Close(idx) => {
                        let _ = self.tx.send(Message::CloseTab(idx));
                    }
                }
                return;
            }
        }

        if let Some(area) = self.sidebar_area {
            if self.sidebar_visible
                && x >= area.x
                && x < area.x + area.width
                && y >= area.y
                && y < area.y + area.height
            {
                self.focus = FocusPane::Sidebar;
                if left_click {
                    if let Some(action) = self.sidebar.click_at(area, &self.schema, x, y) {
                        match action {
                            SidebarAction::OpenTable(name) => self.open_table_with_mode(name, true),
                            SidebarAction::Toggle => {}
                        }
                    }
                }
                return;
            }
        }

        if let Some(area) = self.grid_inner_area {
            if x >= area.x && x < area.x + area.width && y >= area.y && y < area.y + area.height {
                self.focus = FocusPane::Grid;
                if !left_click {
                    return;
                }
                let mut cycle_sort = false;
                if let Some(grid) = self.grid.as_mut() {
                    if let Some(hit) = crate::grid::hit_test(area, grid, x, y) {
                        match hit {
                            crate::grid::GridHit::Header(col) => {
                                grid.focus_cell(grid.focused_row, col);
                                cycle_sort = true;
                            }
                            crate::grid::GridHit::RowGutter(row) => {
                                let focused_col = grid.focused_col;
                                if ctrl_click {
                                    grid.toggle_row_selected(row);
                                } else {
                                    grid.select_only_row(row);
                                }
                                grid.focus_cell_preserve_selection(row, focused_col);
                                self.dirty = true;
                            }
                            crate::grid::GridHit::Cell { row, col } => {
                                grid.focus_cell(row, col);
                            }
                            crate::grid::GridHit::AlphabetRail(letter) => {
                                let _ = self.tx.send(Message::JumpToLetter(letter));
                            }
                            crate::grid::GridHit::Scrollbar => {
                                if self.begin_grid_scrollbar_drag(area, y) {
                                    self.dirty = true;
                                }
                                return;
                            }
                        }
                    }
                }
                if cycle_sort {
                    let _ = self.tx.send(Message::CycleSort);
                }
            }
        }
    }

    fn handle_text_editor_mouse(&mut self, mouse: crossterm::event::MouseEvent) {
        use crossterm::event::{MouseButton, MouseEventKind};

        let Some(PopupKind::TextEditor(state)) = self.popup.as_mut() else {
            return;
        };
        let x = mouse.column;
        let y = mouse.row;
        let changed = match mouse.kind {
            MouseEventKind::ScrollDown if state.mouse_scroll_area_contains(x, y) => {
                let previous = state.scroll_y;
                state.scroll_down(3);
                state.scroll_y != previous
            }
            MouseEventKind::ScrollUp if state.mouse_scroll_area_contains(x, y) => {
                let previous = state.scroll_y;
                state.scroll_up(3);
                state.scroll_y != previous
            }
            MouseEventKind::Down(MouseButton::Left) => {
                state.end_scrollbar_drag();
                state.begin_scrollbar_drag(x, y)
            }
            MouseEventKind::Drag(MouseButton::Left) => state.drag_scrollbar(y),
            MouseEventKind::Up(MouseButton::Left) => {
                state.end_scrollbar_drag();
                false
            }
            _ => false,
        };
        if changed {
            self.dirty = true;
        }
    }

    fn handle_filter_popup_mouse(&mut self, mouse: crossterm::event::MouseEvent) {
        use crate::ui::popup::filter::FilterPopupHit;
        use crossterm::event::{MouseButton, MouseEventKind};

        if !matches!(mouse.kind, MouseEventKind::Down(MouseButton::Left)) {
            return;
        }

        let mut apply = None;
        {
            let Some(PopupKind::FilterPopup(state)) = self.popup.as_mut() else {
                return;
            };
            let Some(hit) = crate::ui::popup::filter::hit_test(
                self.screen_area,
                state,
                mouse.column,
                mouse.row,
            ) else {
                return;
            };

            match hit {
                FilterPopupHit::RuleRow(index) => {
                    state.set_selected_rule(index);
                    state.focus_rule_list();
                }
                FilterPopupHit::RuleToggle(index) => {
                    state.set_selected_rule(index);
                    state.focus_rule_list();
                    if state.toggle_selected_rule_enabled() {
                        apply = Some((state.col_name.clone(), state.col_filter.clone()));
                    }
                }
                FilterPopupHit::RuleDelete(index) => {
                    state.set_selected_rule(index);
                    if state.delete_selected_rule() {
                        apply = Some((state.col_name.clone(), state.col_filter.clone()));
                    }
                    state.focus_rule_list();
                }
                FilterPopupHit::Operator => {
                    state.focus_operator();
                }
                FilterPopupHit::OperatorChevron => {
                    state.focus_operator();
                    state.next_op();
                }
                FilterPopupHit::Value(offset) => {
                    state.focus_value();
                    state.set_cursor_from_display_x(offset);
                }
            }
        }

        if let Some((col_name, col_filter)) = apply {
            self.apply_column_filter(col_name, col_filter);
        }
        self.dirty = true;
    }

    fn mouse_scroll_panel(&mut self, x: u16, y: u16, amount: usize, down: bool) -> bool {
        if let Some(area) = self.sidebar_area {
            if self.sidebar_visible
                && x >= area.x
                && x < area.x + area.width
                && y >= area.y
                && y < area.y + area.height
            {
                self.focus = FocusPane::Sidebar;
                let viewport_rows = area.height.saturating_sub(2) as usize;
                if down {
                    self.sidebar
                        .scroll_down(&self.schema, viewport_rows, amount);
                } else {
                    self.sidebar.scroll_up(&self.schema, viewport_rows, amount);
                }
                return true;
            }
        }

        if let Some(area) = self.grid_outer_area.or(self.grid_inner_area) {
            if x >= area.x && x < area.x + area.width && y >= area.y && y < area.y + area.height {
                self.focus = FocusPane::Grid;
                if down {
                    self.scroll_grid_down(amount);
                } else {
                    self.scroll_grid_up(amount);
                }
                return true;
            }
        }

        false
    }

    fn scroll_grid_down(&mut self, n: usize) {
        let maybe_fetch = if let Some(ref mut grid) = self.grid {
            grid.commit_row_selection();
            grid.scroll_down(n);
            if grid.needs_fetch && !grid.window.fetch_in_flight {
                grid.window.fetch_in_flight = true;
                grid.needs_fetch = false;
                let (off, lim) = grid.window.fetch_params(grid.focused_row as i64);
                let sort = grid.sort.as_ref().and_then(|s| {
                    grid.columns
                        .get(s.col_idx)
                        .map(|c| (c.name.clone(), s.direction == SortDir::Asc))
                });
                Some((
                    grid.table_name.clone(),
                    grid.columns.clone(),
                    sort,
                    off,
                    lim,
                ))
            } else {
                None
            }
        } else {
            None
        };
        if let Some((table, cols, sort, off, lim)) = maybe_fetch {
            self.spawn_window_fetch(&table, &cols, sort, off, lim);
        }
    }

    fn scroll_grid_to_row(&mut self, row: i64) {
        let maybe_fetch = if let Some(ref mut grid) = self.grid {
            grid.commit_row_selection();
            grid.scroll_to_row(row);
            if grid.needs_fetch && !grid.window.fetch_in_flight {
                grid.window.fetch_in_flight = true;
                grid.needs_fetch = false;
                let (off, lim) = grid.window.fetch_params(grid.focused_row as i64);
                let sort = grid.sort.as_ref().and_then(|s| {
                    grid.columns
                        .get(s.col_idx)
                        .map(|c| (c.name.clone(), s.direction == SortDir::Asc))
                });
                Some((
                    grid.table_name.clone(),
                    grid.columns.clone(),
                    sort,
                    off,
                    lim,
                ))
            } else {
                None
            }
        } else {
            None
        };
        if let Some((table, cols, sort, off, lim)) = maybe_fetch {
            self.spawn_window_fetch(&table, &cols, sort, off, lim);
        }
    }

    fn scroll_grid_up(&mut self, n: usize) {
        let maybe_fetch = if let Some(ref mut grid) = self.grid {
            grid.commit_row_selection();
            grid.scroll_up(n);
            if grid.needs_fetch && !grid.window.fetch_in_flight {
                grid.window.fetch_in_flight = true;
                grid.needs_fetch = false;
                let (off, lim) = grid.window.fetch_params(grid.focused_row as i64);
                let sort = grid.sort.as_ref().and_then(|s| {
                    grid.columns
                        .get(s.col_idx)
                        .map(|c| (c.name.clone(), s.direction == SortDir::Asc))
                });
                Some((
                    grid.table_name.clone(),
                    grid.columns.clone(),
                    sort,
                    off,
                    lim,
                ))
            } else {
                None
            }
        } else {
            None
        };
        if let Some((table, cols, sort, off, lim)) = maybe_fetch {
            self.spawn_window_fetch(&table, &cols, sort, off, lim);
        }
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

    fn clamp_grid_viewport(grid: &mut crate::grid::GridState) {
        let viewport_rows = grid.window.viewport_rows.max(1) as i64;
        let max_start = (grid.window.total_rows - viewport_rows).max(0);
        if grid.viewport_start > max_start {
            grid.viewport_start = max_start;
        }
        if grid.viewport_start < 0 {
            grid.viewport_start = 0;
        }
    }

    fn sync_inline_insert_column(&mut self, selected_col: usize) {
        if let Some(ref mut grid) = self.grid {
            let focused_row = grid.focused_row;
            grid.focus_cell(focused_row, selected_col);
        }
    }

    fn begin_grid_scrollbar_drag(&mut self, area: Rect, y: u16) -> bool {
        let Some((grab_offset, target_row)) = self
            .grid
            .as_ref()
            .and_then(|grid| crate::grid::scrollbar_drag_start(area, grid, y))
        else {
            return false;
        };

        self.focus = FocusPane::Grid;
        self.grid_scrollbar_drag = Some(GridScrollbarDrag { grab_offset });
        self.scroll_grid_to_row(target_row);
        true
    }

    fn drag_grid_scrollbar(&mut self, y: u16) -> bool {
        let Some(area) = self.grid_inner_area else {
            self.grid_scrollbar_drag = None;
            return false;
        };
        let Some(grab_offset) = self
            .grid_scrollbar_drag
            .as_ref()
            .map(|drag| drag.grab_offset)
        else {
            return false;
        };
        let Some(target_row) = self
            .grid
            .as_ref()
            .and_then(|grid| crate::grid::scrollbar_drag_target_row(area, grid, y, grab_offset))
        else {
            return false;
        };

        self.focus = FocusPane::Grid;
        self.scroll_grid_to_row(target_row);
        true
    }

    fn handle_key(&mut self, key: crossterm::event::KeyEvent) {
        use crossterm::event::{KeyCode, KeyModifiers};

        // Handle pending confirmation dialog first
        if self.pending_confirm.is_some() {
            match key.code {
                KeyCode::Char('y') | KeyCode::Char('Y') => {
                    let _ = self.tx.send(Message::ConfirmDelete);
                }
                KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => {
                    let _ = self.tx.send(Message::CancelConfirm);
                }
                _ => {}
            }
            return;
        }

        // ? and Ctrl-H toggle help (from any mode, unless confirming)
        if key.code == KeyCode::Char('?')
            || ((key.code == KeyCode::Char('h') || key.code == KeyCode::Char('H'))
                && key.modifiers.contains(KeyModifiers::CONTROL))
        {
            if matches!(self.popup, Some(PopupKind::Help(_))) {
                let _ = self.tx.send(Message::ClosePopup);
            } else {
                let _ = self.tx.send(Message::OpenHelp);
            }
            return;
        }

        match self.mode {
            AppMode::Edit => {
                self.handle_edit_key(key);
                return;
            }
            AppMode::Browse => {}
        }

        // Ctrl-P / Ctrl-Shift-P opens command palette (checked after edit handling)
        if (key.code == KeyCode::Char('p') || key.code == KeyCode::Char('P'))
            && key.modifiers.contains(KeyModifiers::CONTROL)
        {
            let _ = self.tx.send(Message::OpenCommandPalette);
            return;
        }

        // Ctrl-F opens Find popup when a table is open
        if key.code == KeyCode::Char('f')
            && key.modifiers.contains(KeyModifiers::CONTROL)
            && self.grid.is_some()
        {
            let _ = self.tx.send(Message::OpenFind);
            return;
        }

        match (key.code, key.modifiers) {
            (KeyCode::Char('q'), KeyModifiers::CONTROL) => {
                self.should_quit = true;
            }
            (KeyCode::Char('w'), KeyModifiers::CONTROL) if self.active_tab.is_some() => {
                let _ = self
                    .tx
                    .send(Message::CloseTab(self.active_tab.unwrap_or_default()));
            }
            (KeyCode::Char('z'), KeyModifiers::CONTROL) => {
                let _ = self.tx.send(Message::UndoAction);
            }
            (KeyCode::Char('b'), KeyModifiers::CONTROL) => {
                self.sidebar_visible = !self.sidebar_visible;
            }
            (KeyCode::Char(c @ '1'..='9'), KeyModifiers::NONE) if !self.open_tabs.is_empty() => {
                let idx = (c as usize) - ('1' as usize);
                if idx < self.open_tabs.len() {
                    let _ = self.tx.send(Message::ActivateTab(idx));
                }
            }
            (KeyCode::Char('0'), KeyModifiers::NONE) if self.open_tabs.len() >= 10 => {
                let _ = self.tx.send(Message::ActivateTab(9));
            }
            (KeyCode::Tab, KeyModifiers::NONE) => {
                self.focus = if self.sidebar_visible {
                    match self.focus {
                        FocusPane::Sidebar => FocusPane::Grid,
                        FocusPane::Grid => FocusPane::Sidebar,
                    }
                } else {
                    FocusPane::Grid
                };
            }
            (KeyCode::BackTab, _) => {
                self.focus = if self.sidebar_visible {
                    match self.focus {
                        FocusPane::Sidebar => FocusPane::Grid,
                        FocusPane::Grid => FocusPane::Sidebar,
                    }
                } else {
                    FocusPane::Grid
                };
            }
            _ => match self.focus {
                FocusPane::Sidebar => self.handle_sidebar_key(key),
                FocusPane::Grid => self.handle_grid_key(key),
            },
        }
    }

    fn handle_edit_key(&mut self, key: crossterm::event::KeyEvent) {
        use crossterm::event::{KeyCode, KeyModifiers};
        if key.code == KeyCode::Enter
            && key.modifiers.contains(KeyModifiers::ALT)
            && matches!(self.popup, Some(PopupKind::InsertRow(_)))
        {
            let _ = self.tx.send(Message::CommitInsertRow);
            return;
        }
        if key.code == KeyCode::Enter && key.modifiers.contains(KeyModifiers::CONTROL) {
            match self.popup {
                Some(PopupKind::ValuePicker(_))
                | Some(PopupKind::DatePicker(_))
                | Some(PopupKind::DatetimePicker(_))
                | Some(PopupKind::FkPicker(_)) => {
                    let _ = self.tx.send(Message::CommitEdit);
                    return;
                }
                _ => {}
            }
        }
        match &mut self.popup {
            None => {
                self.mode = AppMode::Browse;
            }
            Some(PopupKind::TextEditor(state)) => match key.code {
                KeyCode::Esc => {
                    let _ = self.tx.send(Message::ClosePopup);
                }
                KeyCode::Enter
                    if key.modifiers.contains(KeyModifiers::ALT) && state.is_multiline =>
                {
                    state.insert_char('\n');
                    self.dirty = true;
                }
                KeyCode::Enter
                    if key.modifiers.is_empty() || key.modifiers == KeyModifiers::SHIFT =>
                {
                    let _ = self.tx.send(Message::CommitEdit);
                }
                KeyCode::Tab if state.is_multiline => {
                    state.insert_char(' ');
                    state.insert_char(' ');
                    self.dirty = true;
                }
                KeyCode::Char(c)
                    if key.modifiers == KeyModifiers::NONE
                        || key.modifiers == KeyModifiers::SHIFT =>
                {
                    state.insert_char(c);
                    self.dirty = true;
                }
                KeyCode::Backspace => {
                    state.delete_backward();
                    self.dirty = true;
                }
                KeyCode::Left => {
                    state.move_cursor_left();
                    self.dirty = true;
                }
                KeyCode::Right => {
                    state.move_cursor_right();
                    self.dirty = true;
                }
                KeyCode::Up if state.is_multiline => {
                    state.move_cursor_up();
                    self.dirty = true;
                }
                KeyCode::Down if state.is_multiline => {
                    state.move_cursor_down();
                    self.dirty = true;
                }
                KeyCode::PageUp if state.is_multiline => {
                    state.scroll_up(6);
                    self.dirty = true;
                }
                KeyCode::PageDown if state.is_multiline => {
                    state.scroll_down(6);
                    self.dirty = true;
                }
                _ => {}
            },
            Some(PopupKind::ValuePicker(state)) => match key.code {
                KeyCode::Esc => {
                    let _ = self.tx.send(Message::ClosePopup);
                }
                KeyCode::Enter => {
                    let _ = self.tx.send(Message::CommitEdit);
                }
                KeyCode::Up => {
                    state.move_up();
                    self.dirty = true;
                }
                KeyCode::Down => {
                    state.move_down();
                    self.dirty = true;
                }
                KeyCode::Char(c)
                    if key.modifiers == KeyModifiers::NONE
                        || key.modifiers == KeyModifiers::SHIFT =>
                {
                    state.push_filter_char(c);
                    self.dirty = true;
                }
                KeyCode::Backspace => {
                    state.pop_filter_char();
                    self.dirty = true;
                }
                _ => {}
            },
            Some(PopupKind::DatePicker(state)) => match key.code {
                KeyCode::Esc => {
                    let _ = self.tx.send(Message::ClosePopup);
                }
                KeyCode::Enter => {
                    let _ = self.tx.send(Message::CommitEdit);
                }
                KeyCode::PageUp => {
                    state.prev_month();
                    self.dirty = true;
                }
                KeyCode::PageDown => {
                    state.next_month();
                    self.dirty = true;
                }
                KeyCode::Left => {
                    if state.focus == DateFocus::Calendar {
                        state.calendar_left();
                    } else {
                        state.focus_prev();
                    }
                    self.dirty = true;
                }
                KeyCode::Right => {
                    if state.focus == DateFocus::Calendar {
                        state.calendar_right();
                    } else {
                        state.focus_next();
                    }
                    self.dirty = true;
                }
                KeyCode::Up => {
                    if state.focus == DateFocus::Calendar {
                        state.move_day(-7);
                    } else {
                        state.adjust_focused(1);
                    }
                    self.dirty = true;
                }
                KeyCode::Down => {
                    if state.focus == DateFocus::Calendar {
                        state.move_day(7);
                    } else {
                        state.adjust_focused(-1);
                    }
                    self.dirty = true;
                }
                KeyCode::Tab => {
                    state.focus_next();
                    self.dirty = true;
                }
                KeyCode::BackTab => {
                    state.focus_prev();
                    self.dirty = true;
                }
                KeyCode::Delete => {
                    state.clear();
                    self.dirty = true;
                }
                _ => {}
            },
            Some(PopupKind::DatetimePicker(state)) => match key.code {
                KeyCode::Esc => {
                    let _ = self.tx.send(Message::ClosePopup);
                }
                KeyCode::Enter => {
                    let _ = self.tx.send(Message::CommitEdit);
                }
                KeyCode::PageUp => {
                    state.prev_month();
                    self.dirty = true;
                }
                KeyCode::PageDown => {
                    state.next_month();
                    self.dirty = true;
                }
                KeyCode::Left => {
                    if state.focus == DatetimeFocus::Calendar {
                        state.calendar_left();
                    } else {
                        state.focus_prev();
                    }
                    self.dirty = true;
                }
                KeyCode::Right => {
                    if state.focus == DatetimeFocus::Calendar {
                        state.calendar_right();
                    } else {
                        state.focus_next();
                    }
                    self.dirty = true;
                }
                KeyCode::Up => {
                    if state.focus == DatetimeFocus::Calendar {
                        state.move_day(-7);
                    } else {
                        state.adjust_focused(1);
                    }
                    self.dirty = true;
                }
                KeyCode::Down => {
                    if state.focus == DatetimeFocus::Calendar {
                        state.move_day(7);
                    } else {
                        state.adjust_focused(-1);
                    }
                    self.dirty = true;
                }
                KeyCode::Tab => {
                    state.focus_next();
                    self.dirty = true;
                }
                KeyCode::BackTab => {
                    state.focus_prev();
                    self.dirty = true;
                }
                KeyCode::Delete => {
                    state.clear();
                    self.dirty = true;
                }
                _ => {}
            },
            Some(PopupKind::InsertRow(state)) => {
                let mut sync_col = None;
                match key.code {
                    KeyCode::Esc => {
                        let _ = self.tx.send(Message::ClosePopup);
                    }
                    KeyCode::Enter => {
                        state.move_next_field();
                        state.start_editing();
                        sync_col = Some(state.selected);
                        self.dirty = true;
                    }
                    KeyCode::Tab => {
                        state.move_next_field();
                        state.start_editing();
                        sync_col = Some(state.selected);
                        self.dirty = true;
                    }
                    KeyCode::BackTab => {
                        state.move_prev_field();
                        state.start_editing();
                        sync_col = Some(state.selected);
                        self.dirty = true;
                    }
                    KeyCode::Up => {
                        state.move_prev_field();
                        state.start_editing();
                        sync_col = Some(state.selected);
                        self.dirty = true;
                    }
                    KeyCode::Down => {
                        state.move_next_field();
                        state.start_editing();
                        sync_col = Some(state.selected);
                        self.dirty = true;
                    }
                    KeyCode::Left => {
                        state.move_cursor_left();
                        self.dirty = true;
                    }
                    KeyCode::Right => {
                        state.move_cursor_right();
                        self.dirty = true;
                    }
                    KeyCode::Backspace => {
                        state.delete_backward();
                        self.dirty = true;
                    }
                    KeyCode::Delete => {
                        state.reset_selected();
                        state.start_editing();
                        self.dirty = true;
                    }
                    KeyCode::Char(c)
                        if key.modifiers == KeyModifiers::NONE
                            || key.modifiers == KeyModifiers::SHIFT =>
                    {
                        state.insert_char(c);
                        self.dirty = true;
                    }
                    _ => {}
                }
                if let Some(selected_col) = sync_col {
                    self.sync_inline_insert_column(selected_col);
                }
            }
            Some(PopupKind::FkPicker(state)) => match key.code {
                KeyCode::Esc => {
                    let _ = self.tx.send(Message::ClosePopup);
                }
                KeyCode::Enter => {
                    let _ = self.tx.send(Message::CommitEdit);
                }
                KeyCode::Up => {
                    state.move_up();
                    self.dirty = true;
                }
                KeyCode::Down => {
                    state.move_down();
                    self.dirty = true;
                }
                KeyCode::Char(c)
                    if key.modifiers == KeyModifiers::NONE
                        || key.modifiers == KeyModifiers::SHIFT =>
                {
                    state.push_filter_char(c);
                    self.dirty = true;
                }
                KeyCode::Backspace => {
                    state.pop_filter_char();
                    self.dirty = true;
                }
                _ => {}
            },
            Some(PopupKind::FilterPopup(state)) => match key.code {
                KeyCode::Esc => {
                    self.finish_popup();
                    self.dirty = true;
                }
                KeyCode::Enter => {
                    let mut toast = None;
                    let mut apply = None;
                    {
                        if let Some(PopupKind::FilterPopup(state)) = self.popup.as_mut() {
                            match state.focus {
                                crate::ui::popup::filter::FilterPopupFocus::RuleList => {
                                    state.focus_value();
                                }
                                crate::ui::popup::filter::FilterPopupFocus::Operator => {
                                    state.focus_value();
                                }
                                crate::ui::popup::filter::FilterPopupFocus::Value => {
                                    match state.add_rule() {
                                        Ok(()) => {
                                            apply = Some((
                                                state.col_name.clone(),
                                                state.col_filter.clone(),
                                            ));
                                        }
                                        Err(message) => {
                                            toast = Some(message);
                                        }
                                    }
                                }
                            }
                            self.dirty = true;
                        }
                    }
                    if let Some(message) = toast {
                        self.toast.push(message, ToastKind::Error);
                    }
                    if let Some((col_name, col_filter)) = apply {
                        self.apply_column_filter(col_name, col_filter);
                    }
                }
                KeyCode::Up => {
                    match state.focus {
                        crate::ui::popup::filter::FilterPopupFocus::RuleList => {
                            state.select_prev_rule();
                        }
                        crate::ui::popup::filter::FilterPopupFocus::Operator => {
                            state.prev_op();
                        }
                        crate::ui::popup::filter::FilterPopupFocus::Value => {
                            state.prev_op();
                        }
                    }
                    self.dirty = true;
                }
                KeyCode::Down => {
                    match state.focus {
                        crate::ui::popup::filter::FilterPopupFocus::RuleList => {
                            state.select_next_rule();
                        }
                        crate::ui::popup::filter::FilterPopupFocus::Operator => {
                            state.next_op();
                        }
                        crate::ui::popup::filter::FilterPopupFocus::Value => {
                            state.next_op();
                        }
                    }
                    self.dirty = true;
                }
                KeyCode::Left => {
                    if state.focus == crate::ui::popup::filter::FilterPopupFocus::Value {
                        state.move_cursor_left();
                    }
                    self.dirty = true;
                }
                KeyCode::Right => {
                    if state.focus == crate::ui::popup::filter::FilterPopupFocus::Value {
                        state.move_cursor_right();
                    }
                    self.dirty = true;
                }
                KeyCode::Tab => {
                    state.next_focus();
                    self.dirty = true;
                }
                KeyCode::BackTab => {
                    state.prev_focus();
                    self.dirty = true;
                }
                KeyCode::Char(' ')
                    if state.focus == crate::ui::popup::filter::FilterPopupFocus::RuleList =>
                {
                    let apply = if state.toggle_selected_rule_enabled() {
                        Some((state.col_name.clone(), state.col_filter.clone()))
                    } else {
                        None
                    };
                    if let Some((col_name, col_filter)) = apply {
                        self.apply_column_filter(col_name, col_filter);
                    }
                    self.dirty = true;
                }
                KeyCode::Char(' ')
                    if state.focus == crate::ui::popup::filter::FilterPopupFocus::Operator =>
                {
                    state.next_op();
                    self.dirty = true;
                }
                KeyCode::Delete | KeyCode::Backspace
                    if state.focus == crate::ui::popup::filter::FilterPopupFocus::RuleList =>
                {
                    let apply = if state.delete_selected_rule() {
                        Some((state.col_name.clone(), state.col_filter.clone()))
                    } else {
                        None
                    };
                    if let Some((col_name, col_filter)) = apply {
                        self.apply_column_filter(col_name, col_filter);
                    }
                    self.dirty = true;
                }
                KeyCode::Char(c)
                    if state.focus == crate::ui::popup::filter::FilterPopupFocus::Value
                        && (key.modifiers == KeyModifiers::NONE
                            || key.modifiers == KeyModifiers::SHIFT) =>
                {
                    state.push_char(c);
                    self.dirty = true;
                }
                KeyCode::Backspace
                    if state.focus == crate::ui::popup::filter::FilterPopupFocus::Value =>
                {
                    state.pop_char();
                    self.dirty = true;
                }
                _ => {}
            },
            Some(PopupKind::CommandPalette(state)) => match key.code {
                KeyCode::Esc => {
                    let _ = self.tx.send(Message::ClosePopup);
                }
                KeyCode::Enter => {
                    if let Some(cmd) = state.selected_command() {
                        let _ = self.tx.send(Message::ClosePopup);
                        let _ = self.tx.send(Message::ExecuteCommand(cmd));
                    }
                }
                KeyCode::Up => {
                    state.move_up();
                    self.dirty = true;
                }
                KeyCode::Down => {
                    state.move_down();
                    self.dirty = true;
                }
                KeyCode::Char(c) => {
                    state.push_char(c);
                    self.dirty = true;
                }
                KeyCode::Backspace => {
                    state.pop_char();
                    self.dirty = true;
                }
                _ => {}
            },
            Some(PopupKind::Help(state)) => match key.code {
                KeyCode::Esc => {
                    let _ = self.tx.send(Message::ClosePopup);
                }
                KeyCode::Up => {
                    state.scroll_up(3);
                    self.dirty = true;
                }
                KeyCode::Down => {
                    state.scroll_down(3);
                    self.dirty = true;
                }
                KeyCode::PageUp => {
                    state.scroll_up(10);
                    self.dirty = true;
                }
                KeyCode::PageDown => {
                    state.scroll_down(10);
                    self.dirty = true;
                }
                _ => {}
            },
            Some(PopupKind::Find(state)) => match key.code {
                KeyCode::Esc => {
                    let _ = self.tx.send(Message::ClosePopup);
                }
                KeyCode::Enter => {
                    let _ = self.tx.send(Message::CommitFind);
                }
                KeyCode::Up => {
                    state.move_up();
                    self.dirty = true;
                }
                KeyCode::Down => {
                    state.move_down();
                    self.dirty = true;
                }
                KeyCode::Char(c)
                    if key.modifiers == KeyModifiers::NONE
                        || key.modifiers == KeyModifiers::SHIFT =>
                {
                    state.push_char(c);
                    self.dirty = true;
                }
                KeyCode::Backspace => {
                    state.pop_char();
                    self.dirty = true;
                }
                _ => {}
            },
        }
    }

    fn handle_grid_key(&mut self, key: crossterm::event::KeyEvent) {
        use crossterm::event::{KeyCode, KeyModifiers};
        let vp = self.grid.as_ref().map_or(20, |g| g.window.viewport_rows);
        match (key.code, key.modifiers) {
            (KeyCode::Char('a'), KeyModifiers::CONTROL) => {
                if let Some(ref mut grid) = self.grid {
                    grid.select_all_rows();
                    if grid.has_row_selection() {
                        self.toast.push(
                            "All rows selected; Delete will clear the table",
                            ToastKind::Info,
                        );
                    }
                    self.dirty = true;
                }
            }
            (KeyCode::Down, KeyModifiers::SHIFT) => {
                if let Some(ref mut grid) = self.grid {
                    grid.extend_row_selection_down(1);
                    self.dirty = true;
                }
            }
            (KeyCode::Up, KeyModifiers::SHIFT) => {
                if let Some(ref mut grid) = self.grid {
                    grid.extend_row_selection_up(1);
                    self.dirty = true;
                }
            }
            (KeyCode::Down, KeyModifiers::CONTROL) => {
                let _ = self.tx.send(Message::ScrollDown(vp.saturating_sub(1)));
            }
            (KeyCode::Up, KeyModifiers::CONTROL) => {
                let _ = self.tx.send(Message::ScrollUp(vp.saturating_sub(1)));
            }
            (KeyCode::Down, _) => {
                let _ = self.tx.send(Message::MoveDown);
            }
            (KeyCode::Char('j'), KeyModifiers::NONE) => {
                let is_fk = self
                    .grid
                    .as_ref()
                    .and_then(|g| g.fk_cols.get(g.focused_col).copied())
                    .unwrap_or(false);
                if is_fk {
                    let _ = self.tx.send(Message::JumpToFk);
                } else {
                    let _ = self.tx.send(Message::MoveDown);
                }
            }
            (KeyCode::Up, _) | (KeyCode::Char('k'), KeyModifiers::NONE) => {
                let _ = self.tx.send(Message::MoveUp);
            }
            (KeyCode::Right, _) | (KeyCode::Char('l'), KeyModifiers::NONE) => {
                let _ = self.tx.send(Message::MoveRight);
            }
            (KeyCode::Left, _) | (KeyCode::Char('h'), KeyModifiers::NONE) => {
                let _ = self.tx.send(Message::MoveLeft);
            }
            (KeyCode::Home, KeyModifiers::CONTROL) => {
                let _ = self.tx.send(Message::MoveFirstCell);
            }
            (KeyCode::End, KeyModifiers::CONTROL) => {
                let _ = self.tx.send(Message::MoveLastCell);
            }
            (KeyCode::Home, _) => {
                let _ = self.tx.send(Message::MoveColFirst);
            }
            (KeyCode::End, _) => {
                let _ = self.tx.send(Message::MoveColLast);
            }
            (KeyCode::PageDown, _) => {
                let _ = self.tx.send(Message::ScrollDown(vp.saturating_sub(1)));
            }
            (KeyCode::PageUp, _) => {
                let _ = self.tx.send(Message::ScrollUp(vp.saturating_sub(1)));
            }
            (KeyCode::Enter, KeyModifiers::NONE) => {
                let _ = self.tx.send(Message::OpenPopup);
            }
            (KeyCode::Char('e'), KeyModifiers::NONE) => {
                let _ = self.tx.send(Message::OpenDirectEdit);
            }
            (KeyCode::Char('n'), KeyModifiers::NONE) if self.focused_cell_can_be_set_null() => {
                let _ = self.tx.send(Message::SetFocusedCellNull);
            }
            (KeyCode::Esc, _) => {
                if let Some(ref mut grid) = self.grid {
                    if grid.has_row_selection() {
                        grid.clear_row_selection();
                        self.dirty = true;
                    }
                }
            }
            (KeyCode::Backspace, _) if !self.jump_stack.is_empty() => {
                let _ = self.tx.send(Message::JumpBack);
            }
            (KeyCode::Char('s'), KeyModifiers::NONE) => {
                let _ = self.tx.send(Message::CycleSort);
            }
            (KeyCode::Char('f'), KeyModifiers::NONE) => {
                let _ = self.tx.send(Message::OpenFilterPopup);
            }
            (KeyCode::Char('F'), KeyModifiers::SHIFT) => {
                let _ = self.tx.send(Message::ClearFilters);
            }
            (KeyCode::Insert, KeyModifiers::NONE) => {
                let _ = self.tx.send(Message::InsertRow);
            }
            (KeyCode::Char('i'), KeyModifiers::NONE) => {
                let _ = self.tx.send(Message::InsertRow);
            }
            (KeyCode::Delete, KeyModifiers::NONE) => {
                let _ = self.tx.send(Message::DeleteRow);
            }
            (KeyCode::Char('d'), KeyModifiers::NONE) => {
                let _ = self.tx.send(Message::DeleteRow);
            }
            (KeyCode::Char('c'), KeyModifiers::CONTROL) => {
                let _ = self.tx.send(Message::CopyCell);
            }
            (KeyCode::Char('y'), KeyModifiers::NONE) => {
                let _ = self.tx.send(Message::CopyCell);
            }
            (KeyCode::Char('Y'), KeyModifiers::SHIFT) => {
                let _ = self.tx.send(Message::CopyRowJson);
            }
            (KeyCode::Char(c), KeyModifiers::NONE)
                if c.is_alphabetic()
                    && !matches!(
                        c,
                        'j' | 'k' | 'h' | 'l' | 's' | 'f' | 'i' | 'd' | 'e' | 'n' | 'y'
                    ) =>
            {
                let is_text_sort = self.grid.as_ref().is_some_and(|g| {
                    if let Some(sort) = &g.sort {
                        if let Some(col) = g.columns.get(sort.col_idx) {
                            return matches!(affinity(&col.col_type), ColAffinity::Text);
                        }
                    }
                    false
                });
                if is_text_sort {
                    let _ = self.tx.send(Message::JumpToLetter(c));
                }
            }
            (KeyCode::Char('#'), KeyModifiers::NONE) => {
                let is_text_sort = self.grid.as_ref().is_some_and(|g| {
                    if let Some(sort) = &g.sort {
                        if let Some(col) = g.columns.get(sort.col_idx) {
                            return matches!(affinity(&col.col_type), ColAffinity::Text);
                        }
                    }
                    false
                });
                if is_text_sort {
                    let _ = self.tx.send(Message::JumpToLetter('#'));
                }
            }
            _ => {}
        }
    }

    fn handle_sidebar_key(&mut self, key: crossterm::event::KeyEvent) {
        use crossterm::event::KeyCode;

        match key.code {
            KeyCode::Char('q') => self.should_quit = true,
            KeyCode::Up | KeyCode::Char('k') => self.sidebar.move_up(&self.schema),
            KeyCode::Down | KeyCode::Char('j') => self.sidebar.move_down(&self.schema),
            KeyCode::Left | KeyCode::Char('h') => {
                self.sidebar.collapse_selected_section(&self.schema);
            }
            KeyCode::Right | KeyCode::Char('l') => {
                self.sidebar.expand_selected_section(&self.schema);
            }
            KeyCode::Enter => {
                if let Some(SidebarAction::OpenTable(name)) = self.sidebar.enter(&self.schema) {
                    let _ = self.tx.send(Message::OpenTable(name));
                }
            }
            _ => {}
        }
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

    fn finish_popup(&mut self) -> bool {
        self.popup_request_serial = self.popup_request_serial.wrapping_add(1);
        self.popup = None;
        self.mode = AppMode::Browse;
        if let Some(grid) = self.grid.as_mut() {
            Self::clamp_grid_viewport(grid);
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
        self.popup_request_serial = self.popup_request_serial.wrapping_add(1);
        self.popup = Some(PopupKind::TextEditor(TextEditorState::new(
            table,
            rowid,
            col_name,
            col_type,
            original,
            self.readonly,
        )));
        self.mode = AppMode::Edit;
    }

    fn submit_cell_edit(
        &mut self,
        table: String,
        col: String,
        rowid: i64,
        value: SqlValue,
        original: SqlValue,
    ) {
        if self.write_in_flight {
            self.toast
                .push("A database write is already running", ToastKind::Info);
            self.dirty = true;
            return;
        }
        self.write_in_flight = true;
        let pool = Arc::clone(&self.pool);
        let tx = self.tx.clone();
        let table_c = table.clone();
        let col_c = col.clone();
        tokio::task::spawn(async move {
            let tx_err = tx.clone();
            let result = tokio::task::spawn_blocking(move || -> anyhow::Result<()> {
                let conn = pool.get()?;
                crate::db::write::commit_cell_edit(&conn, &table_c, &col_c, rowid, &value)?;
                let _ = tx.send(Message::EditCommitted {
                    rowid,
                    table: table_c,
                    col: col_c,
                    original,
                });
                Ok(())
            })
            .await;

            match result {
                Ok(Ok(())) => {}
                Ok(Err(err)) => {
                    let _ = tx_err.send(Message::EditFailed(err.to_string()));
                }
                Err(err) => {
                    let _ = tx_err.send(Message::EditFailed(err.to_string()));
                }
            }
        });
        self.dirty = true;
    }

    fn open_table(&mut self, name: String) {
        self.open_table_with_mode(name, true);
    }

    fn open_table_with_mode(&mut self, name: String, new_tab: bool) {
        self.focus = FocusPane::Grid;
        if let Some(idx) = self.open_tabs.iter().position(|t| t.table_name == name) {
            self.active_tab = Some(idx);
            self.request_table_view(&name);
        } else {
            if new_tab || self.active_tab.is_none() {
                self.open_tabs.push(TableTab {
                    table_name: name.clone(),
                });
                self.active_tab = Some(self.open_tabs.len() - 1);
            } else if let Some(idx) = self.active_tab {
                if let Some(tab) = self.open_tabs.get_mut(idx) {
                    tab.table_name = name.clone();
                }
            }
            self.request_table_view(&name);
        }
        self.dirty = true;
    }

    fn request_table_view(&mut self, name: &str) {
        self.navigation_request_serial = self.navigation_request_serial.wrapping_add(1);
        self.grid_request_serial.fetch_add(1, Ordering::AcqRel);
        self.grid = None;
        let cols_and_fks = self
            .schema
            .tables
            .iter()
            .find(|t| t.name == name)
            .map(|tm| {
                let columns = tm.columns.clone();
                let fk_names: Vec<String> = tm
                    .foreign_keys
                    .iter()
                    .map(|fk| fk.from_col.clone())
                    .collect();
                let fk_cols: Vec<bool> =
                    columns.iter().map(|c| fk_names.contains(&c.name)).collect();
                (columns, fk_cols)
            });

        if let Some((columns, fk_cols)) = cols_and_fks {
            self.spawn_grid_fetch(name.to_string(), columns, fk_cols);
        }
    }

    fn refresh_active_grid_schema(&mut self) {
        self.navigation_request_serial = self.navigation_request_serial.wrapping_add(1);
        let Some(current_grid) = self.grid.as_ref() else {
            if let Some(table) = self
                .active_tab
                .and_then(|index| self.open_tabs.get(index))
                .map(|tab| tab.table_name.clone())
            {
                self.request_table_view(&table);
            }
            return;
        };
        let table = current_grid.table_name.clone();
        let Some(table_meta) = self.schema.tables.iter().find(|item| item.name == table) else {
            self.grid_request_serial.fetch_add(1, Ordering::AcqRel);
            self.grid = None;
            self.toast.push(
                format!("Table {table:?} no longer exists"),
                ToastKind::Error,
            );
            return;
        };

        let previous_sort = current_grid.sort.as_ref().and_then(|sort| {
            current_grid
                .columns
                .get(sort.col_idx)
                .map(|column| (column.name.clone(), sort.direction.clone()))
        });
        let columns = table_meta.columns.clone();
        let foreign_key_names = table_meta
            .foreign_keys
            .iter()
            .map(|foreign_key| foreign_key.from_col.as_str())
            .collect::<std::collections::HashSet<_>>();
        let foreign_key_columns = columns
            .iter()
            .map(|column| foreign_key_names.contains(column.name.as_str()))
            .collect::<Vec<_>>();
        let column_names = columns
            .iter()
            .map(|column| column.name.as_str())
            .collect::<std::collections::HashSet<_>>();
        let mut filter = current_grid.filter.clone();
        filter
            .columns
            .retain(|column, _| column_names.contains(column.as_str()));
        let sort = previous_sort.and_then(|(name, direction)| {
            columns
                .iter()
                .position(|column| column.name == name)
                .map(|col_idx| SortSpec { col_idx, direction })
        });

        let fetch = if let Some(grid) = self.grid.as_mut() {
            grid.columns = columns;
            grid.fk_cols = foreign_key_columns;
            grid.enumerated_values = vec![Vec::new(); grid.columns.len()];
            grid.width_sample_rows.clear();
            grid.focused_col = grid.focused_col.min(grid.columns.len().saturating_sub(1));
            grid.focused_row = 0;
            grid.viewport_start = 0;
            grid.h_scroll = 0;
            grid.sort = sort;
            grid.filter = filter;
            grid.window.rows.clear();
            grid.window.rowids.clear();
            grid.window.offset = 0;
            grid.window.total_rows = 0;
            grid.window.fetch_in_flight = true;
            grid.needs_fetch = false;
            grid.recompute_col_widths(grid.avail_col_width);
            let sort = grid.sort.as_ref().and_then(|sort| {
                grid.columns.get(sort.col_idx).map(|column| {
                    (
                        column.name.clone(),
                        sort.direction == crate::grid::SortDir::Asc,
                    )
                })
            });
            let (offset, limit) = grid.window.fetch_params(0);
            Some((
                grid.columns.clone(),
                sort,
                offset,
                limit,
                grid.filter.clone(),
            ))
        } else {
            None
        };

        if let Some((columns, sort, offset, limit, filter)) = fetch {
            self.spawn_window_fetch_with_filter(&table, &columns, sort, offset, limit, filter);
        }
    }

    fn spawn_grid_fetch(&self, table: String, columns: Vec<Column>, fk_cols: Vec<bool>) {
        let request_id = self.grid_request_serial.fetch_add(1, Ordering::AcqRel) + 1;
        let tx = self.tx.clone();
        let pool = Arc::clone(&self.pool);
        tokio::task::spawn(async move {
            let table_c = table.clone();
            let cols_c = columns.clone();
            let result = tokio::task::spawn_blocking(move || -> anyhow::Result<GridFetchResult> {
                let conn = pool.get()?;
                let total = db::count_rows(&conn, &table_c, "", &[])?;
                let fetched = db::fetch_rows_with_rowids(
                    &conn,
                    db::RowFetch {
                        table: &table_c,
                        columns: &cols_c,
                        offset: 0,
                        limit: 50,
                        order_by: None,
                        where_clause: "",
                        where_params: &[],
                    },
                )?;
                let enumerated_values = (0..cols_c.len())
                    .map(|column| inferred_enumerated_values(&fetched.rows, column, total))
                    .collect();
                let width_sample_rows = fetched.rows.iter().take(50).cloned().collect();
                Ok((
                    fetched.rows,
                    fetched.rowids,
                    total,
                    enumerated_values,
                    width_sample_rows,
                ))
            })
            .await;
            match result {
                Ok(Ok((rows, rowids, total_rows, enumerated_values, width_sample_rows))) => {
                    let _ = tx.send(Message::GridDataReady {
                        request_id,
                        table,
                        columns,
                        fk_cols,
                        enumerated_values,
                        width_sample_rows,
                        rows,
                        rowids,
                        total_rows,
                    });
                }
                Ok(Err(error)) => {
                    let _ = tx.send(Message::GridReadFailed {
                        request_id,
                        table,
                        error: error.to_string(),
                    });
                }
                Err(error) => {
                    let _ = tx.send(Message::GridReadFailed {
                        request_id,
                        table,
                        error: error.to_string(),
                    });
                }
            }
        });
    }

    fn spawn_window_fetch(
        &self,
        table: &str,
        columns: &[Column],
        sort: Option<(String, bool)>,
        offset: i64,
        limit: i64,
    ) {
        let filter = self
            .grid
            .as_ref()
            .map(|g| g.filter.clone())
            .unwrap_or_default();
        self.spawn_window_fetch_with_filter(table, columns, sort, offset, limit, filter);
    }

    fn spawn_window_fetch_with_filter(
        &self,
        table: &str,
        columns: &[Column],
        sort: Option<(String, bool)>,
        offset: i64,
        limit: i64,
        filter: crate::filter::FilterSet,
    ) {
        let request_id = self.grid_request_serial.fetch_add(1, Ordering::AcqRel) + 1;
        let pool = Arc::clone(&self.pool);
        let tx = self.tx.clone();
        let table = table.to_string();
        let columns = columns.to_vec();
        tokio::task::spawn(async move {
            let table_c = table.clone();
            let result =
                tokio::task::spawn_blocking(move || -> anyhow::Result<(db::FetchedRows, i64)> {
                    let conn = pool.get()?;
                    let (where_clause, where_params) = filter_to_sql(&filter)?;
                    let total = db::count_rows(&conn, &table_c, &where_clause, &where_params)?;
                    let order_by = sort.as_ref().map(|(s, b)| (s.as_str(), *b));
                    let rows = db::fetch_rows_with_rowids(
                        &conn,
                        db::RowFetch {
                            table: &table_c,
                            columns: &columns,
                            offset,
                            limit,
                            order_by,
                            where_clause: &where_clause,
                            where_params: &where_params,
                        },
                    )?;
                    Ok((rows, total))
                })
                .await;
            match result {
                Ok(Ok((rows, total_rows))) => {
                    let _ = tx.send(Message::WindowReady {
                        request_id,
                        table,
                        offset,
                        rows: rows.rows,
                        rowids: rows.rowids,
                        total_rows,
                    });
                }
                Ok(Err(error)) => {
                    let _ = tx.send(Message::GridReadFailed {
                        request_id,
                        table,
                        error: error.to_string(),
                    });
                }
                Err(error) => {
                    let _ = tx.send(Message::GridReadFailed {
                        request_id,
                        table,
                        error: error.to_string(),
                    });
                }
            }
        });
    }

    fn on_grid_data_ready(&mut self, payload: GridDataReadyPayload) {
        let GridDataReadyPayload {
            table,
            columns,
            fk_cols,
            enumerated_values,
            width_sample_rows,
            rows,
            rowids,
            total_rows,
        } = payload;
        let is_active = self
            .active_tab
            .and_then(|i| self.open_tabs.get(i))
            .is_some_and(|t| t.table_name == table);
        if is_active {
            let grid_width = self
                .grid_inner_area
                .map(|area| area.width)
                .unwrap_or_default();
            let mut grid = crate::grid::GridState::new(crate::grid::GridInit {
                table_name: table.clone(),
                columns,
                fk_cols,
                enumerated_values,
                rows,
                width_sample_rows,
                total_rows,
                area_width: grid_width,
            });
            grid.window.rowids = rowids;
            if let Ok(saved_filter) = crate::filter::load_filter(&self.db_path, &table) {
                if !saved_filter.is_empty() {
                    grid.filter = saved_filter.clone();
                    let cols = grid.columns.clone();
                    let (off, lim) = grid.window.fetch_params(0);
                    grid.window.fetch_in_flight = true;
                    self.spawn_window_fetch_with_filter(
                        &table,
                        &cols,
                        None,
                        off,
                        lim,
                        saved_filter,
                    );
                }
            }
            self.grid = Some(grid);
        }
        self.dirty = true;
    }

    fn jump_to_rowid(&mut self, table: String, rowid: i64, col: Option<usize>) {
        let same_table_context = self.grid.as_ref().and_then(|grid| {
            if grid.table_name == table {
                let sort = grid.sort.as_ref().and_then(|s| {
                    grid.columns
                        .get(s.col_idx)
                        .map(|c| (c.name.clone(), s.direction == SortDir::Asc))
                });
                Some((sort, grid.filter.clone(), col.unwrap_or(grid.focused_col)))
            } else {
                None
            }
        });

        let Some((sort, filter, target_col)) = same_table_context else {
            self.pending_jump_target = Some(PendingJumpTarget { table, rowid, col });
            self.dirty = true;
            return;
        };

        let Some(target_row) = self.resolve_offset_for_rowid(&table, rowid, sort, filter) else {
            self.dirty = true;
            return;
        };

        let maybe_fetch = if let Some(ref mut grid) = self.grid {
            grid.focus_cell(target_row as usize, target_col);
            if grid.needs_fetch && !grid.window.fetch_in_flight {
                grid.window.fetch_in_flight = true;
                grid.needs_fetch = false;
                let (off, lim) = grid.window.fetch_params(grid.focused_row as i64);
                let sort = grid.sort.as_ref().and_then(|s| {
                    grid.columns
                        .get(s.col_idx)
                        .map(|c| (c.name.clone(), s.direction == SortDir::Asc))
                });
                Some((
                    grid.table_name.clone(),
                    grid.columns.clone(),
                    sort,
                    off,
                    lim,
                ))
            } else {
                None
            }
        } else {
            None
        };

        if let Some((table, cols, sort, off, lim)) = maybe_fetch {
            self.spawn_window_fetch(&table, &cols, sort, off, lim);
        }
        self.dirty = true;
    }

    fn count_table_rows(&mut self, table: &str) -> Option<i64> {
        let conn = match self.pool.get() {
            Ok(conn) => conn,
            Err(err) => {
                self.toast
                    .push(format!("DB connection failed: {}", err), ToastKind::Error);
                return None;
            }
        };
        match db::count_rows(&conn, table, "", &[]) {
            Ok(count) => Some(count),
            Err(err) => {
                self.toast
                    .push(format!("Row count failed: {}", err), ToastKind::Error);
                None
            }
        }
    }

    fn resolve_offset_for_rowid(
        &mut self,
        table: &str,
        rowid: i64,
        sort: Option<(String, bool)>,
        filter: crate::filter::FilterSet,
    ) -> Option<i64> {
        let (where_clause, where_params) = match filter_to_sql(&filter) {
            Ok(predicate) => predicate,
            Err(error) => {
                self.toast.push(error.to_string(), ToastKind::Error);
                return None;
            }
        };
        let conn = match self.pool.get() {
            Ok(conn) => conn,
            Err(err) => {
                self.toast
                    .push(format!("DB connection failed: {}", err), ToastKind::Error);
                return None;
            }
        };
        match db::fetch_offset_for_rowid(
            &conn,
            table,
            rowid,
            sort.as_ref().map(|(col, asc)| (col.as_str(), *asc)),
            &where_clause,
            &where_params,
        ) {
            Ok(Some(offset)) => Some(offset),
            Ok(None) => {
                self.toast
                    .push("Row not found in current view", ToastKind::Error);
                None
            }
            Err(err) => {
                self.toast
                    .push(format!("Row lookup failed: {}", err), ToastKind::Error);
                None
            }
        }
    }

    fn apply_column_filter(&mut self, col_name: String, col_filter: crate::filter::ColumnFilter) {
        self.navigation_request_serial = self.navigation_request_serial.wrapping_add(1);
        if let Some(ref mut grid) = self.grid {
            if col_filter.rules.iter().any(|rule| rule.enabled) {
                grid.filter.columns.insert(col_name, col_filter);
            } else {
                grid.filter.columns.remove(&col_name);
            }
            grid.viewport_start = 0;
            grid.focused_row = 0;
            grid.window.rows.clear();
            grid.window.offset = 0;
            let table = grid.table_name.clone();
            let cols = grid.columns.clone();
            let sort = grid.sort.as_ref().and_then(|s| {
                grid.columns
                    .get(s.col_idx)
                    .map(|c| (c.name.clone(), s.direction == SortDir::Asc))
            });
            let (off, lim) = grid.window.fetch_params(0);
            grid.window.fetch_in_flight = true;
            let filter = grid.filter.clone();
            let db_path = self.db_path.clone();
            if let Err(error) = crate::filter::save_filter(&filter, &db_path, &table) {
                self.toast
                    .push(format!("Could not save filter: {error}"), ToastKind::Error);
            }
            self.spawn_window_fetch_with_filter(&table, &cols, sort, off, lim, filter);
        }
        self.dirty = true;
    }

    fn close_tab(&mut self, idx: usize) {
        if idx < self.open_tabs.len() {
            let next_active =
                next_active_tab_after_close(self.active_tab, idx, self.open_tabs.len() - 1);
            self.open_tabs.remove(idx);
            self.active_tab = next_active;
            if let Some(active_idx) = self.active_tab {
                let table = self.open_tabs[active_idx].table_name.clone();
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

    fn next_tab(&mut self) {
        if self.open_tabs.is_empty() {
            return;
        }
        let current = self.active_tab.unwrap_or(0);
        self.activate_tab((current + 1) % self.open_tabs.len());
    }

    fn prev_tab(&mut self) {
        if self.open_tabs.is_empty() {
            return;
        }
        let current = self.active_tab.unwrap_or(0);
        self.activate_tab(current.checked_sub(1).unwrap_or(self.open_tabs.len() - 1));
    }
}

fn count_rows_before_letter(
    conn: &rusqlite::Connection,
    table: &str,
    column: &str,
    ascending: bool,
    letter: char,
    uppercase_letter: char,
    filter: &crate::filter::FilterSet,
) -> anyhow::Result<i64> {
    let (filter_clause, mut params) = filter_to_sql(filter)?;
    let column = db::query::quote_identifier(column);
    let predicate = if ascending {
        if letter == '#' {
            format!("{column} IS NULL")
        } else {
            let value_parameter = params.len() + 1;
            params.push(rusqlite::types::Value::Text(uppercase_letter.to_string()));
            format!("({column} IS NULL OR {column} < ?{value_parameter})")
        }
    } else if letter == '#' {
        format!("({column} IS NOT NULL AND {column} NOT GLOB '[0-9]*')")
    } else {
        let value_parameter = params.len() + 1;
        params.push(rusqlite::types::Value::Text(uppercase_letter.to_string()));
        let pattern_parameter = params.len() + 1;
        params.push(rusqlite::types::Value::Text(format!("{uppercase_letter}%")));
        format!("({column} > ?{value_parameter} AND {column} NOT LIKE ?{pattern_parameter})")
    };
    let where_clause = if filter_clause.is_empty() {
        predicate
    } else {
        format!("({filter_clause}) AND {predicate}")
    };
    let query = format!(
        "SELECT COUNT(*) FROM {} WHERE {where_clause}",
        db::query::quote_identifier(table)
    );
    conn.query_row(&query, rusqlite::params_from_iter(params.iter()), |row| {
        row.get(0)
    })
    .map_err(Into::into)
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

fn sql_value_to_json_value(value: &SqlValue) -> serde_json::Value {
    match value {
        SqlValue::Null => serde_json::Value::Null,
        SqlValue::Integer(n) => serde_json::Value::Number((*n).into()),
        SqlValue::Real(f) => serde_json::Number::from_f64(*f)
            .map(serde_json::Value::Number)
            .unwrap_or(serde_json::Value::Null),
        SqlValue::Text(text) => serde_json::Value::String(text.clone()),
        SqlValue::Blob(_) => serde_json::Value::Null,
    }
}

fn row_to_json_value(columns: &[Column], row: &[SqlValue]) -> serde_json::Value {
    let mut map = serde_json::Map::with_capacity(columns.len());
    for (column, value) in columns.iter().zip(row.iter()) {
        map.insert(column.name.clone(), sql_value_to_json_value(value));
    }
    serde_json::Value::Object(map)
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
    if values.len() >= ENUM_COLOR_DISTINCT_LIMIT {
        return Vec::new();
    }
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
mod tests {
    use std::{collections::BTreeSet, sync::Arc};

    use crossterm::event::{KeyCode, KeyModifiers, MouseButton, MouseEventKind};
    use r2d2_sqlite::SqliteConnectionManager;
    use ratatui::{backend::TestBackend, Terminal};
    use tokio::sync::mpsc;

    use super::*;
    use crate::{
        config::Config,
        db::{self, schema::Column, types::SqlValue},
        ui::popup::HelpState,
    };

    // ---------- helpers ----------

    fn make_test_app() -> (App, mpsc::UnboundedReceiver<Message>) {
        let manager = SqliteConnectionManager::memory();
        let pool = Arc::new(
            r2d2::Pool::builder()
                .max_size(1)
                .build(manager)
                .expect("test pool"),
        );
        let conn = pool.get().expect("test conn");
        conn.execute_batch(
            "CREATE TABLE users (id INTEGER PRIMARY KEY, name TEXT, age INTEGER, email TEXT);",
        )
        .expect("seed schema");
        let schema = db::load_schema(&conn).expect("load schema");
        drop(conn);

        let (tx, rx) = mpsc::unbounded_channel();
        let config = Config::default();
        let app = App::new(schema, config, pool, tx, false, ":memory:".to_string());
        (app, rx)
    }

    fn dummy_column(cid: i64, name: &str, col_type: &str) -> Column {
        Column {
            cid,
            name: name.to_string(),
            col_type: col_type.to_string(),
            not_null: false,
            default_value: None,
            is_pk: cid == 0,
            pk_position: if cid == 0 { 1 } else { 0 },
            writable: true,
        }
    }

    fn make_grid() -> crate::grid::GridState {
        let columns = vec![
            dummy_column(0, "id", "INTEGER"),
            dummy_column(1, "name", "TEXT"),
            dummy_column(2, "age", "INTEGER"),
            dummy_column(3, "email", "TEXT"),
        ];
        let rows: Vec<Vec<SqlValue>> = (0..50)
            .map(|i| {
                vec![
                    SqlValue::Integer(i),
                    SqlValue::Text(format!("user-{i}")),
                    SqlValue::Integer(20 + i % 40),
                    SqlValue::Text(format!("user{}@example.com", i)),
                ]
            })
            .collect();
        let mut grid = crate::grid::GridState::new(crate::grid::GridInit {
            table_name: "users".to_string(),
            columns,
            fk_cols: vec![false; 4],
            enumerated_values: vec![Vec::new(); 4],
            rows,
            width_sample_rows: vec![Vec::new(); 4],
            total_rows: 50,
            area_width: 80,
        });
        grid.window.rowids = (1..=50).map(Some).collect();
        grid
    }

    fn make_viewport_rows(app: &App) -> usize {
        app.grid.as_ref().map_or(20, |g| g.window.viewport_rows)
    }

    fn seed_user_row(app: &App) {
        let conn = app.pool.get().expect("test conn");
        conn.execute(
            "INSERT INTO users (id, name, age, email) VALUES (?1, ?2, ?3, ?4)",
            rusqlite::params![1i64, "Alice", 30i64, "alice@example.com"],
        )
        .expect("seed user row");
    }

    fn seed_user_rows(app: &App, count: usize) {
        let conn = app.pool.get().expect("test conn");
        for index in 0..count {
            let id = index as i64 + 1;
            conn.execute(
                "INSERT INTO users (id, name, age, email) VALUES (?1, ?2, ?3, ?4)",
                rusqlite::params![
                    id,
                    format!("User {id}"),
                    20 + (id % 30),
                    format!("user{id}@example.com")
                ],
            )
            .expect("seed user row");
        }
    }

    fn seed_grid_rows(app: &App, count: usize) {
        let conn = app.pool.get().expect("test conn");
        for index in 0..count {
            let id = index as i64;
            conn.execute(
                "INSERT INTO users (id, name, age, email) VALUES (?1, ?2, ?3, ?4)",
                rusqlite::params![
                    id,
                    format!("user-{id}"),
                    20 + (id % 40),
                    format!("user{id}@example.com")
                ],
            )
            .expect("seed grid row");
        }
    }

    fn make_constrained_insert_app() -> (App, mpsc::UnboundedReceiver<Message>) {
        let manager = SqliteConnectionManager::memory();
        let pool = Arc::new(
            r2d2::Pool::builder()
                .max_size(1)
                .build(manager)
                .expect("test pool"),
        );
        let conn = pool.get().expect("test conn");
        conn.execute_batch(
            "CREATE TABLE users (
                id INTEGER PRIMARY KEY,
                name TEXT NOT NULL,
                age INTEGER DEFAULT 18
            );",
        )
        .expect("seed schema");
        let schema = db::load_schema(&conn).expect("load schema");
        let columns = schema.tables[0].columns.clone();
        drop(conn);

        let (tx, rx) = mpsc::unbounded_channel();
        let mut app = App::new(
            schema,
            Config::default(),
            pool,
            tx,
            false,
            ":memory:".to_string(),
        );
        app.grid = Some(crate::grid::GridState::new(crate::grid::GridInit {
            table_name: "users".to_string(),
            columns,
            fk_cols: vec![false; 3],
            enumerated_values: vec![Vec::new(); 3],
            rows: Vec::new(),
            width_sample_rows: vec![Vec::new(); 3],
            total_rows: 0,
            area_width: 80,
        }));
        (app, rx)
    }

    #[test]
    fn normalize_enumerated_values_skips_unique_columns() {
        let values = vec!["a".to_string(), "b".to_string(), "c".to_string()];

        assert!(normalize_enumerated_values(values, 3).is_empty());
    }

    #[test]
    fn normalize_enumerated_values_keeps_repeated_short_values() {
        let values = vec!["pending".to_string(), "done".to_string()];

        assert_eq!(normalize_enumerated_values(values.clone(), 5), values);
    }

    fn try_recv_variant(rx: &mut mpsc::UnboundedReceiver<Message>) -> String {
        let msg = rx.try_recv();
        match msg {
            Ok(m) => format!("{:?}", m),
            Err(_) => "no message".to_string(),
        }
    }

    /// Drain channel and process messages through app.update().
    fn drain_messages(app: &mut App, rx: &mut mpsc::UnboundedReceiver<Message>) {
        while let Ok(message) = rx.try_recv() {
            app.update(message);
        }
    }

    fn render_test_app(app: &mut App) {
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).expect("test terminal");
        terminal
            .draw(|frame| crate::ui::render(frame, app))
            .expect("render app");
    }

    fn text_editor_scroll_y(app: &App) -> u16 {
        match app.popup.as_ref() {
            Some(PopupKind::TextEditor(state)) => state.scroll_y,
            _ => panic!("expected text editor popup"),
        }
    }

    // ---------- global shortcuts ----------

    #[test]
    fn ctrl_q_sets_should_quit() {
        let (mut app, _rx) = make_test_app();
        app.update(Message::Key(crossterm::event::KeyEvent::new(
            KeyCode::Char('q'),
            KeyModifiers::CONTROL,
        )));
        assert!(app.should_quit);
    }

    #[test]
    fn ctrl_b_toggles_sidebar() {
        let (mut app, _rx) = make_test_app();
        assert!(app.sidebar_visible);
        app.update(Message::Key(crossterm::event::KeyEvent::new(
            KeyCode::Char('b'),
            KeyModifiers::CONTROL,
        )));
        assert!(!app.sidebar_visible);
        app.update(Message::Key(crossterm::event::KeyEvent::new(
            KeyCode::Char('b'),
            KeyModifiers::CONTROL,
        )));
        assert!(app.sidebar_visible);
    }

    #[test]
    fn ctrl_w_closes_current_tab() {
        let (mut app, mut rx) = make_test_app();
        app.open_tabs = vec![TableTab {
            table_name: "users".to_string(),
        }];
        app.active_tab = Some(0);

        app.update(Message::Key(crossterm::event::KeyEvent::new(
            KeyCode::Char('w'),
            KeyModifiers::CONTROL,
        )));
        drain_messages(&mut app, &mut rx);

        assert!(app.open_tabs.is_empty());
        assert_eq!(app.active_tab, None);
    }

    #[test]
    fn tab_toggles_focus_sidebar_to_grid() {
        let (mut app, _rx) = make_test_app();
        app.sidebar_visible = true;
        app.focus = FocusPane::Sidebar;
        app.update(Message::Key(crossterm::event::KeyEvent::new(
            KeyCode::Tab,
            KeyModifiers::NONE,
        )));
        assert!(matches!(app.focus, FocusPane::Grid));
        app.update(Message::Key(crossterm::event::KeyEvent::new(
            KeyCode::Tab,
            KeyModifiers::NONE,
        )));
        assert!(matches!(app.focus, FocusPane::Sidebar));
    }

    #[test]
    fn backtab_toggles_focus() {
        let (mut app, _rx) = make_test_app();
        app.sidebar_visible = true;
        app.focus = FocusPane::Sidebar;
        app.update(Message::Key(crossterm::event::KeyEvent::new(
            KeyCode::BackTab,
            KeyModifiers::NONE,
        )));
        assert!(matches!(app.focus, FocusPane::Grid));
    }

    #[test]
    fn question_mark_opens_and_closes_help() {
        let (mut app, mut rx) = make_test_app();
        assert!(app.popup.is_none());
        app.update(Message::Key(crossterm::event::KeyEvent::new(
            KeyCode::Char('?'),
            KeyModifiers::SHIFT,
        )));
        drain_messages(&mut app, &mut rx);
        assert!(
            matches!(app.popup, Some(PopupKind::Help(_))),
            "popup should be Help"
        );
        app.update(Message::Key(crossterm::event::KeyEvent::new(
            KeyCode::Char('?'),
            KeyModifiers::SHIFT,
        )));
        drain_messages(&mut app, &mut rx);
        assert!(app.popup.is_none(), "popup should be closed after second ?");
    }

    #[test]
    fn ctrl_p_opens_command_palette() {
        let (mut app, mut rx) = make_test_app();
        app.update(Message::Key(crossterm::event::KeyEvent::new(
            KeyCode::Char('p'),
            KeyModifiers::CONTROL,
        )));
        drain_messages(&mut app, &mut rx);
        assert!(
            matches!(app.popup, Some(PopupKind::CommandPalette(_))),
            "popup should be CommandPalette"
        );
    }

    #[test]
    fn ctrl_h_opens_help() {
        let (mut app, mut rx) = make_test_app();
        app.update(Message::Key(crossterm::event::KeyEvent::new(
            KeyCode::Char('h'),
            KeyModifiers::CONTROL,
        )));
        drain_messages(&mut app, &mut rx);
        assert!(matches!(app.popup, Some(PopupKind::Help(_))));
    }

    #[test]
    fn esc_in_help_closes_popup() {
        let (mut app, mut rx) = make_test_app();
        app.popup = Some(PopupKind::Help(HelpState::new()));
        app.mode = AppMode::Edit;
        app.update(Message::Key(crossterm::event::KeyEvent::new(
            KeyCode::Esc,
            KeyModifiers::NONE,
        )));
        drain_messages(&mut app, &mut rx);
        assert!(app.popup.is_none(), "help popup should be closed");
    }

    #[test]
    fn ctrl_enter_in_help_is_noop() {
        let (mut app, mut rx) = make_test_app();
        app.popup = Some(PopupKind::Help(HelpState::new()));
        app.mode = AppMode::Edit;
        app.update(Message::Key(crossterm::event::KeyEvent::new(
            KeyCode::Enter,
            KeyModifiers::CONTROL,
        )));
        // no CommitEdit sent for Help popup
        assert!(rx.try_recv().is_err());
    }

    #[test]
    fn ctrl_enter_in_text_editor_is_noop() {
        let (mut app, mut rx) = make_test_app();
        app.popup = Some(PopupKind::TextEditor(TextEditorState::new(
            "users".to_string(),
            1,
            "name".to_string(),
            "TEXT".to_string(),
            SqlValue::Text("Alice".to_string()),
            false,
        )));
        app.mode = AppMode::Edit;

        app.update(Message::Key(crossterm::event::KeyEvent::new(
            KeyCode::Enter,
            KeyModifiers::CONTROL,
        )));

        assert!(rx.try_recv().is_err());
    }

    #[test]
    fn enter_in_text_editor_sends_commit_edit() {
        let (mut app, mut rx) = make_test_app();
        app.popup = Some(PopupKind::TextEditor(TextEditorState::new(
            "users".to_string(),
            1,
            "name".to_string(),
            "TEXT".to_string(),
            SqlValue::Text("Alice".to_string()),
            false,
        )));
        app.mode = AppMode::Edit;

        app.update(Message::Key(crossterm::event::KeyEvent::new(
            KeyCode::Enter,
            KeyModifiers::NONE,
        )));

        assert_eq!(try_recv_variant(&mut rx), "CommitEdit");
    }

    #[test]
    fn alt_enter_in_text_editor_inserts_newline() {
        let (mut app, mut rx) = make_test_app();
        app.popup = Some(PopupKind::TextEditor(TextEditorState::new(
            "users".to_string(),
            1,
            "name".to_string(),
            "TEXT".to_string(),
            SqlValue::Text("Alice".to_string()),
            false,
        )));
        app.mode = AppMode::Edit;

        app.update(Message::Key(crossterm::event::KeyEvent::new(
            KeyCode::Enter,
            KeyModifiers::ALT,
        )));

        match app.popup {
            Some(PopupKind::TextEditor(ref state)) => assert_eq!(state.current, "Alice\n"),
            _ => panic!("expected text editor popup"),
        }
        assert!(rx.try_recv().is_err());
    }

    #[test]
    fn alt_enter_in_insert_row_sends_commit_insert_row() {
        let (mut app, mut rx) = make_constrained_insert_app();
        app.update(Message::InsertRow);
        drain_messages(&mut app, &mut rx);

        app.update(Message::Key(crossterm::event::KeyEvent::new(
            KeyCode::Enter,
            KeyModifiers::ALT,
        )));

        assert_eq!(try_recv_variant(&mut rx), "CommitInsertRow");
    }

    // ---------- grid navigation shortcuts ----------

    #[test]
    fn arrow_down_sends_move_down() {
        let (mut app, mut rx) = make_test_app();
        app.grid = Some(make_grid());
        app.focus = FocusPane::Grid;
        app.update(Message::Key(crossterm::event::KeyEvent::new(
            KeyCode::Down,
            KeyModifiers::NONE,
        )));
        assert_eq!(try_recv_variant(&mut rx), "MoveDown");
    }

    #[test]
    fn arrow_up_sends_move_up() {
        let (mut app, mut rx) = make_test_app();
        app.grid = Some(make_grid());
        app.focus = FocusPane::Grid;
        app.update(Message::Key(crossterm::event::KeyEvent::new(
            KeyCode::Up,
            KeyModifiers::NONE,
        )));
        assert_eq!(try_recv_variant(&mut rx), "MoveUp");
    }

    #[test]
    fn shift_down_selects_rows_from_focused_row() {
        let (mut app, _rx) = make_test_app();
        app.grid = Some(make_grid());
        app.focus = FocusPane::Grid;

        app.update(Message::Key(crossterm::event::KeyEvent::new(
            KeyCode::Down,
            KeyModifiers::SHIFT,
        )));

        let grid = app.grid.as_ref().expect("grid");
        assert_eq!(grid.focused_row, 1);
        assert_eq!(
            grid.row_selection,
            crate::grid::RowSelection::Rows(BTreeSet::from([0]))
        );
    }

    #[test]
    fn shift_up_extends_selection_toward_previous_rows() {
        let (mut app, _rx) = make_test_app();
        let mut grid = make_grid();
        grid.focused_row = 3;
        app.grid = Some(grid);
        app.focus = FocusPane::Grid;

        app.update(Message::Key(crossterm::event::KeyEvent::new(
            KeyCode::Up,
            KeyModifiers::SHIFT,
        )));

        let grid = app.grid.as_ref().expect("grid");
        assert_eq!(grid.focused_row, 2);
        assert_eq!(
            grid.row_selection,
            crate::grid::RowSelection::Rows(BTreeSet::from([3]))
        );
    }

    #[test]
    fn move_down_preserves_existing_selection() {
        let (mut app, _rx) = make_test_app();
        let mut grid = make_grid();
        grid.row_selection = crate::grid::RowSelection::Rows(BTreeSet::from([1, 3]));
        grid.window.fetch_in_flight = true;
        app.grid = Some(grid);

        app.update(Message::MoveDown);

        let grid = app.grid.as_ref().expect("grid");
        assert_eq!(grid.focused_row, 1);
        assert_eq!(
            grid.row_selection,
            crate::grid::RowSelection::Rows(BTreeSet::from([1, 3]))
        );
    }

    #[test]
    fn ctrl_a_selects_all_rows_in_grid() {
        let (mut app, _rx) = make_test_app();
        app.grid = Some(make_grid());
        app.focus = FocusPane::Grid;

        app.update(Message::Key(crossterm::event::KeyEvent::new(
            KeyCode::Char('a'),
            KeyModifiers::CONTROL,
        )));

        let grid = app.grid.as_ref().expect("grid");
        assert_eq!(grid.row_selection, crate::grid::RowSelection::All);
    }

    #[test]
    fn arrow_left_sends_move_left() {
        let (mut app, mut rx) = make_test_app();
        app.grid = Some(make_grid());
        app.focus = FocusPane::Grid;
        app.update(Message::Key(crossterm::event::KeyEvent::new(
            KeyCode::Left,
            KeyModifiers::NONE,
        )));
        assert_eq!(try_recv_variant(&mut rx), "MoveLeft");
    }

    #[test]
    fn arrow_right_sends_move_right() {
        let (mut app, mut rx) = make_test_app();
        app.grid = Some(make_grid());
        app.focus = FocusPane::Grid;
        app.update(Message::Key(crossterm::event::KeyEvent::new(
            KeyCode::Right,
            KeyModifiers::NONE,
        )));
        assert_eq!(try_recv_variant(&mut rx), "MoveRight");
    }

    #[test]
    fn vim_hjkl_navigation() {
        let (mut app, mut rx) = make_test_app();
        app.grid = Some(make_grid());
        app.focus = FocusPane::Grid;

        // h = left
        let _ = rx.try_recv(); // drain
        app.update(Message::Key(crossterm::event::KeyEvent::new(
            KeyCode::Char('h'),
            KeyModifiers::NONE,
        )));
        assert_eq!(try_recv_variant(&mut rx), "MoveLeft");

        // l = right
        app.update(Message::Key(crossterm::event::KeyEvent::new(
            KeyCode::Char('l'),
            KeyModifiers::NONE,
        )));
        assert_eq!(try_recv_variant(&mut rx), "MoveRight");

        // k = up
        app.update(Message::Key(crossterm::event::KeyEvent::new(
            KeyCode::Char('k'),
            KeyModifiers::NONE,
        )));
        assert_eq!(try_recv_variant(&mut rx), "MoveUp");

        // j (non-FK) = down
        app.update(Message::Key(crossterm::event::KeyEvent::new(
            KeyCode::Char('j'),
            KeyModifiers::NONE,
        )));
        assert_eq!(try_recv_variant(&mut rx), "MoveDown");
    }

    #[test]
    fn home_sends_move_col_first() {
        let (mut app, mut rx) = make_test_app();
        app.grid = Some(make_grid());
        app.focus = FocusPane::Grid;
        app.update(Message::Key(crossterm::event::KeyEvent::new(
            KeyCode::Home,
            KeyModifiers::NONE,
        )));
        assert_eq!(try_recv_variant(&mut rx), "MoveColFirst");
    }

    #[test]
    fn end_sends_move_col_last() {
        let (mut app, mut rx) = make_test_app();
        app.grid = Some(make_grid());
        app.focus = FocusPane::Grid;
        app.update(Message::Key(crossterm::event::KeyEvent::new(
            KeyCode::End,
            KeyModifiers::NONE,
        )));
        assert_eq!(try_recv_variant(&mut rx), "MoveColLast");
    }

    #[test]
    fn ctrl_home_sends_move_first_cell() {
        let (mut app, mut rx) = make_test_app();
        app.grid = Some(make_grid());
        app.focus = FocusPane::Grid;
        app.update(Message::Key(crossterm::event::KeyEvent::new(
            KeyCode::Home,
            KeyModifiers::CONTROL,
        )));
        assert_eq!(try_recv_variant(&mut rx), "MoveFirstCell");
    }

    #[test]
    fn ctrl_end_sends_move_last_cell() {
        let (mut app, mut rx) = make_test_app();
        app.grid = Some(make_grid());
        app.focus = FocusPane::Grid;
        app.update(Message::Key(crossterm::event::KeyEvent::new(
            KeyCode::End,
            KeyModifiers::CONTROL,
        )));
        assert_eq!(try_recv_variant(&mut rx), "MoveLastCell");
    }

    #[test]
    fn page_down_scrolls_viewport() {
        let (mut app, mut rx) = make_test_app();
        app.grid = Some(make_grid());
        let vp = make_viewport_rows(&app).saturating_sub(1);
        app.focus = FocusPane::Grid;
        app.update(Message::Key(crossterm::event::KeyEvent::new(
            KeyCode::PageDown,
            KeyModifiers::NONE,
        )));
        let msg = try_recv_variant(&mut rx);
        assert_eq!(
            msg,
            format!("ScrollDown({})", vp),
            "expected ScrollDown({}), got {}",
            vp,
            msg
        );
    }

    #[test]
    fn page_up_scrolls_viewport() {
        let (mut app, mut rx) = make_test_app();
        app.grid = Some(make_grid());
        let vp = make_viewport_rows(&app).saturating_sub(1);
        app.focus = FocusPane::Grid;
        app.update(Message::Key(crossterm::event::KeyEvent::new(
            KeyCode::PageUp,
            KeyModifiers::NONE,
        )));
        let msg = try_recv_variant(&mut rx);
        assert_eq!(
            msg,
            format!("ScrollUp({})", vp),
            "expected ScrollUp({}), got {}",
            vp,
            msg
        );
    }

    #[test]
    fn ctrl_up_scrolls_viewport() {
        let (mut app, mut rx) = make_test_app();
        app.grid = Some(make_grid());
        let vp = make_viewport_rows(&app).saturating_sub(1);
        app.focus = FocusPane::Grid;
        app.update(Message::Key(crossterm::event::KeyEvent::new(
            KeyCode::Up,
            KeyModifiers::CONTROL,
        )));
        let msg = try_recv_variant(&mut rx);
        assert_eq!(
            msg,
            format!("ScrollUp({})", vp),
            "expected ScrollUp({}), got {}",
            vp,
            msg
        );
    }

    #[test]
    fn ctrl_down_scrolls_viewport() {
        let (mut app, mut rx) = make_test_app();
        app.grid = Some(make_grid());
        let vp = make_viewport_rows(&app).saturating_sub(1);
        app.focus = FocusPane::Grid;
        app.update(Message::Key(crossterm::event::KeyEvent::new(
            KeyCode::Down,
            KeyModifiers::CONTROL,
        )));
        let msg = try_recv_variant(&mut rx);
        assert_eq!(
            msg,
            format!("ScrollDown({})", vp),
            "expected ScrollDown({}), got {}",
            vp,
            msg
        );
    }

    // ---------- sort and filter shortcuts ----------

    #[test]
    fn s_key_sends_cycle_sort() {
        let (mut app, mut rx) = make_test_app();
        app.grid = Some(make_grid());
        app.focus = FocusPane::Grid;
        app.update(Message::Key(crossterm::event::KeyEvent::new(
            KeyCode::Char('s'),
            KeyModifiers::NONE,
        )));
        assert_eq!(try_recv_variant(&mut rx), "CycleSort");
    }

    #[test]
    fn f_key_sends_open_filter_popup() {
        let (mut app, mut rx) = make_test_app();
        app.grid = Some(make_grid());
        app.focus = FocusPane::Grid;
        app.update(Message::Key(crossterm::event::KeyEvent::new(
            KeyCode::Char('f'),
            KeyModifiers::NONE,
        )));
        assert_eq!(try_recv_variant(&mut rx), "OpenFilterPopup");
    }

    #[test]
    fn shift_f_sends_clear_filters() {
        let (mut app, mut rx) = make_test_app();
        app.grid = Some(make_grid());
        app.focus = FocusPane::Grid;
        app.update(Message::Key(crossterm::event::KeyEvent::new(
            KeyCode::Char('F'),
            KeyModifiers::SHIFT,
        )));
        assert_eq!(try_recv_variant(&mut rx), "ClearFilters");
    }

    #[test]
    fn j_on_fk_col_sends_jump_to_fk() {
        let (mut app, mut rx) = make_test_app();
        let mut grid = make_grid();
        grid.fk_cols[1] = true; // name col is FK
        grid.focused_col = 1; // focus the FK column
        app.grid = Some(grid);
        app.focus = FocusPane::Grid;
        app.update(Message::Key(crossterm::event::KeyEvent::new(
            KeyCode::Char('j'),
            KeyModifiers::NONE,
        )));
        assert_eq!(try_recv_variant(&mut rx), "JumpToFk");
    }

    #[test]
    fn esc_in_grid_clears_selection() {
        let (mut app, _rx) = make_test_app();
        let mut grid = make_grid();
        grid.select_only_row(2);
        app.grid = Some(grid);
        app.focus = FocusPane::Grid;

        app.update(Message::Key(crossterm::event::KeyEvent::new(
            KeyCode::Esc,
            KeyModifiers::NONE,
        )));

        let grid = app.grid.as_ref().expect("grid");
        assert_eq!(grid.row_selection, crate::grid::RowSelection::None);
        assert!(matches!(app.focus, FocusPane::Grid));
    }

    #[test]
    fn backspace_with_jump_stack_sends_jump_back() {
        let (mut app, mut rx) = make_test_app();
        app.grid = Some(make_grid());
        app.focus = FocusPane::Grid;
        app.jump_stack.push(JumpFrame {
            table: "users".to_string(),
            rowid: 1,
            col: 0,
        });
        app.update(Message::Key(crossterm::event::KeyEvent::new(
            KeyCode::Backspace,
            KeyModifiers::NONE,
        )));
        assert_eq!(try_recv_variant(&mut rx), "JumpBack");
    }

    #[test]
    fn backspace_without_jump_stack_is_noop() {
        let (mut app, mut rx) = make_test_app();
        app.grid = Some(make_grid());
        app.focus = FocusPane::Grid;
        assert!(app.jump_stack.is_empty());
        let _ = rx.try_recv(); // drain
        app.update(Message::Key(crossterm::event::KeyEvent::new(
            KeyCode::Backspace,
            KeyModifiers::NONE,
        )));
        // no JumpBack should be sent
        let msg = try_recv_variant(&mut rx);
        assert!(!msg.contains("JumpBack"), "unexpected: {}", msg);
    }

    // ---------- editing shortcuts ----------

    #[test]
    fn i_key_in_grid_sends_insert_row() {
        let (mut app, mut rx) = make_test_app();
        app.grid = Some(make_grid());
        app.focus = FocusPane::Grid;
        app.update(Message::Key(crossterm::event::KeyEvent::new(
            KeyCode::Char('i'),
            KeyModifiers::NONE,
        )));
        assert_eq!(try_recv_variant(&mut rx), "InsertRow");
    }

    #[test]
    fn insert_key_in_grid_sends_insert_row() {
        let (mut app, mut rx) = make_test_app();
        app.grid = Some(make_grid());
        app.focus = FocusPane::Grid;
        app.update(Message::Key(crossterm::event::KeyEvent::new(
            KeyCode::Insert,
            KeyModifiers::NONE,
        )));
        assert_eq!(try_recv_variant(&mut rx), "InsertRow");
    }

    #[test]
    fn insert_row_opens_insert_popup_for_constrained_table() {
        let (mut app, _rx) = make_constrained_insert_app();
        app.focus = FocusPane::Grid;

        app.update(Message::InsertRow);

        assert!(matches!(
            app.popup,
            Some(PopupKind::InsertRow(ref state))
                if state.editing && state.insert_position == 0
        ));
        assert!(matches!(
            app.toast.toasts.back(),
            Some(toast) if toast.message == "Alt-Enter commits" && toast.kind == ToastKind::Info
        ));
    }

    #[test]
    fn invalid_insert_commit_shows_error_toast() {
        let (mut app, _rx) = make_constrained_insert_app();
        app.focus = FocusPane::Grid;
        app.update(Message::InsertRow);

        app.update(Message::CommitInsertRow);

        assert!(matches!(app.popup, Some(PopupKind::InsertRow(_))));
        assert!(matches!(
            app.toast.toasts.back(),
            Some(toast) if toast.message == "name is required" && toast.kind == ToastKind::Error
        ));
    }

    #[test]
    fn d_key_in_grid_shows_confirm_dialog() {
        let (mut app, mut rx) = make_test_app();
        app.grid = Some(make_grid());
        app.focus = FocusPane::Grid;
        app.update(Message::Key(crossterm::event::KeyEvent::new(
            KeyCode::Char('d'),
            KeyModifiers::NONE,
        )));
        // sends DeleteRow - confirm dialog appears as side-effect
        assert_eq!(try_recv_variant(&mut rx), "DeleteRow");
    }

    #[test]
    fn delete_key_in_grid_sends_delete_row() {
        let (mut app, mut rx) = make_test_app();
        app.grid = Some(make_grid());
        app.focus = FocusPane::Grid;
        app.update(Message::Key(crossterm::event::KeyEvent::new(
            KeyCode::Delete,
            KeyModifiers::NONE,
        )));

        assert_eq!(try_recv_variant(&mut rx), "DeleteRow");
    }

    #[test]
    fn delete_row_confirms_selected_row_range() {
        let (mut app, _rx) = make_test_app();
        seed_user_rows(&app, 6);
        let mut grid = make_grid();
        grid.row_selection = crate::grid::RowSelection::Rows(BTreeSet::from([1, 3, 5]));
        app.grid = Some(grid);

        app.update(Message::DeleteRow);

        assert!(matches!(
            app.pending_confirm.as_ref().map(|confirm| &confirm.kind),
            Some(ConfirmKind::DeleteSelectedRows {
                rowids,
                ..
            }) if rowids == &vec![2, 4, 6]
        ));
    }

    #[test]
    fn delete_row_confirms_table_clear_for_select_all() {
        let (mut app, _rx) = make_test_app();
        let mut grid = make_grid();
        grid.row_selection = crate::grid::RowSelection::All;
        app.grid = Some(grid);
        seed_user_rows(&app, 5);

        app.update(Message::DeleteRow);

        assert!(matches!(
            app.pending_confirm.as_ref().map(|confirm| &confirm.kind),
            Some(ConfirmKind::ClearTable { table }) if table == "users"
        ));
        let message = app
            .pending_confirm
            .as_ref()
            .map(|confirm| confirm.message.clone())
            .expect("confirm message");
        assert!(message.contains("Delete all 5 rows from users?"));
    }

    #[test]
    fn y_in_grid_sends_copy_cell() {
        let (mut app, mut rx) = make_test_app();
        app.grid = Some(make_grid());
        app.focus = FocusPane::Grid;
        app.update(Message::Key(crossterm::event::KeyEvent::new(
            KeyCode::Char('y'),
            KeyModifiers::NONE,
        )));
        assert_eq!(try_recv_variant(&mut rx), "CopyCell");
    }

    #[test]
    fn ctrl_c_in_grid_sends_copy_cell() {
        let (mut app, mut rx) = make_test_app();
        app.grid = Some(make_grid());
        app.focus = FocusPane::Grid;
        app.update(Message::Key(crossterm::event::KeyEvent::new(
            KeyCode::Char('c'),
            KeyModifiers::CONTROL,
        )));
        assert_eq!(try_recv_variant(&mut rx), "CopyCell");
    }

    #[test]
    fn shift_y_in_grid_sends_copy_row_json() {
        let (mut app, mut rx) = make_test_app();
        app.grid = Some(make_grid());
        app.focus = FocusPane::Grid;
        app.update(Message::Key(crossterm::event::KeyEvent::new(
            KeyCode::Char('Y'),
            KeyModifiers::SHIFT,
        )));
        assert_eq!(try_recv_variant(&mut rx), "CopyRowJson");
    }

    #[test]
    fn row_json_text_returns_single_object_without_selection() {
        let (mut app, _rx) = make_test_app();
        app.grid = Some(make_grid());
        seed_grid_rows(&app, 50);

        let (json_text, copied_selected_rows) =
            app.row_json_text().expect("row json").expect("json text");
        let json: serde_json::Value = serde_json::from_str(&json_text).expect("valid json");

        assert!(!copied_selected_rows);
        assert_eq!(json["id"], serde_json::json!(0));
        assert_eq!(json["name"], serde_json::json!("user-0"));
    }

    #[test]
    fn row_json_text_returns_selected_rows_as_array() {
        let (mut app, _rx) = make_test_app();
        let mut grid = make_grid();
        grid.row_selection = crate::grid::RowSelection::Rows(BTreeSet::from([1, 3]));
        app.grid = Some(grid);
        seed_grid_rows(&app, 50);

        let (json_text, copied_selected_rows) =
            app.row_json_text().expect("row json").expect("json text");
        let json: serde_json::Value = serde_json::from_str(&json_text).expect("valid json");

        assert!(copied_selected_rows);
        assert_eq!(
            json,
            serde_json::json!([
                {
                    "id": 1,
                    "name": "user-1",
                    "age": 21,
                    "email": "user1@example.com"
                },
                {
                    "id": 3,
                    "name": "user-3",
                    "age": 23,
                    "email": "user3@example.com"
                }
            ])
        );
    }

    #[test]
    fn ctrl_z_sends_undo_action() {
        let (mut app, mut rx) = make_test_app();
        app.grid = Some(make_grid());
        app.focus = FocusPane::Grid;
        let _ = rx.try_recv(); // drain
        app.update(Message::Key(crossterm::event::KeyEvent::new(
            KeyCode::Char('z'),
            KeyModifiers::CONTROL,
        )));
        assert_eq!(try_recv_variant(&mut rx), "UndoAction");
    }

    #[test]
    fn enter_in_grid_sends_open_popup() {
        let (mut app, mut rx) = make_test_app();
        app.grid = Some(make_grid());
        app.focus = FocusPane::Grid;
        let _ = rx.try_recv(); // drain
        app.update(Message::Key(crossterm::event::KeyEvent::new(
            KeyCode::Enter,
            KeyModifiers::NONE,
        )));
        assert_eq!(try_recv_variant(&mut rx), "OpenPopup");
    }

    #[test]
    fn e_in_grid_sends_open_direct_edit() {
        let (mut app, mut rx) = make_test_app();
        app.grid = Some(make_grid());
        app.focus = FocusPane::Grid;
        let _ = rx.try_recv(); // drain
        app.update(Message::Key(crossterm::event::KeyEvent::new(
            KeyCode::Char('e'),
            KeyModifiers::NONE,
        )));
        assert_eq!(try_recv_variant(&mut rx), "OpenDirectEdit");
    }

    #[test]
    fn n_in_grid_sends_set_focused_cell_null_when_allowed() {
        let (mut app, _rx) = make_test_app();
        let mut grid = make_grid();
        grid.focused_col = 1;
        app.grid = Some(grid);
        app.focus = FocusPane::Grid;

        let (tx, mut rx) = mpsc::unbounded_channel();
        app.tx = tx;
        app.update(Message::Key(crossterm::event::KeyEvent::new(
            KeyCode::Char('n'),
            KeyModifiers::NONE,
        )));

        assert_eq!(try_recv_variant(&mut rx), "SetFocusedCellNull");
    }

    #[test]
    fn n_in_grid_does_nothing_when_cell_cannot_be_null() {
        let (mut app, _rx) = make_test_app();
        let mut grid = make_grid();
        grid.focused_col = 1;
        grid.columns[1].not_null = true;
        app.grid = Some(grid);
        app.focus = FocusPane::Grid;

        let (tx, mut rx) = mpsc::unbounded_channel();
        app.tx = tx;
        app.update(Message::Key(crossterm::event::KeyEvent::new(
            KeyCode::Char('n'),
            KeyModifiers::NONE,
        )));

        assert_eq!(try_recv_variant(&mut rx), "no message");
    }

    #[test]
    fn open_direct_edit_opens_text_editor() {
        let (mut app, _rx) = make_test_app();
        let mut grid = make_grid();
        grid.focused_col = 1;
        app.grid = Some(grid);
        seed_user_row(&app);

        app.update(Message::OpenDirectEdit);

        assert!(matches!(app.popup, Some(PopupKind::TextEditor(_))));
    }

    // ---------- sidebar shortcuts ----------

    #[test]
    fn enter_in_sidebar_opens_table() {
        let (mut app, mut rx) = make_test_app();
        app.focus = FocusPane::Sidebar;
        // navigate to first table entry
        app.sidebar.move_down(&app.schema);
        app.update(Message::Key(crossterm::event::KeyEvent::new(
            KeyCode::Enter,
            KeyModifiers::NONE,
        )));
        let msg = try_recv_variant(&mut rx);
        assert!(msg.contains("OpenTable"), "expected OpenTable, got {}", msg);
    }

    #[test]
    fn up_down_arrows_in_sidebar_navigate() {
        let (mut app, _rx) = make_test_app();
        let initial = app.sidebar.selected;
        app.focus = FocusPane::Sidebar;
        app.update(Message::Key(crossterm::event::KeyEvent::new(
            KeyCode::Down,
            KeyModifiers::NONE,
        )));
        assert_ne!(app.sidebar.selected, initial);
    }

    #[test]
    fn left_right_arrows_in_sidebar_collapse_and_expand_selected_section() {
        let (mut app, _rx) = make_test_app();
        app.focus = FocusPane::Sidebar;

        app.update(Message::Key(crossterm::event::KeyEvent::new(
            KeyCode::Left,
            KeyModifiers::NONE,
        )));
        assert!(!app.sidebar.tables_expanded);

        app.update(Message::Key(crossterm::event::KeyEvent::new(
            KeyCode::Right,
            KeyModifiers::NONE,
        )));
        assert!(app.sidebar.tables_expanded);
    }

    #[test]
    fn left_right_arrows_in_sidebar_apply_to_views_and_indexes_headers() {
        let (mut app, _rx) = make_test_app();
        app.focus = FocusPane::Sidebar;

        // tables header, first table, views header
        app.sidebar.move_down(&app.schema);
        app.sidebar.move_down(&app.schema);
        app.update(Message::Key(crossterm::event::KeyEvent::new(
            KeyCode::Left,
            KeyModifiers::NONE,
        )));
        assert!(!app.sidebar.views_expanded);
        app.update(Message::Key(crossterm::event::KeyEvent::new(
            KeyCode::Right,
            KeyModifiers::NONE,
        )));
        assert!(app.sidebar.views_expanded);

        app.sidebar.move_down(&app.schema);
        app.update(Message::Key(crossterm::event::KeyEvent::new(
            KeyCode::Left,
            KeyModifiers::NONE,
        )));
        assert!(!app.sidebar.indexes_expanded);
        app.update(Message::Key(crossterm::event::KeyEvent::new(
            KeyCode::Right,
            KeyModifiers::NONE,
        )));
        assert!(app.sidebar.indexes_expanded);
    }

    // ---------- letter jumps ----------

    #[test]
    fn letter_key_on_text_sorted_column_sends_jump_to_letter() {
        let (mut app, mut rx) = make_test_app();
        let mut grid = make_grid();
        grid.sort = Some(SortSpec {
            col_idx: 1,
            direction: SortDir::Asc,
        }); // name is TEXT -> text sort
        app.grid = Some(grid);
        app.focus = FocusPane::Grid;
        let _ = rx.try_recv(); // drain
        app.update(Message::Key(crossterm::event::KeyEvent::new(
            KeyCode::Char('a'),
            KeyModifiers::NONE,
        )));
        assert_eq!(try_recv_variant(&mut rx), "JumpToLetter('a')");
    }

    #[test]
    fn hash_key_on_text_sorted_column_sends_jump_to_letter() {
        let (mut app, mut rx) = make_test_app();
        let mut grid = make_grid();
        grid.sort = Some(SortSpec {
            col_idx: 1,
            direction: SortDir::Asc,
        });
        app.grid = Some(grid);
        app.focus = FocusPane::Grid;
        let _ = rx.try_recv(); // drain
        app.update(Message::Key(crossterm::event::KeyEvent::new(
            KeyCode::Char('#'),
            KeyModifiers::NONE,
        )));
        assert_eq!(try_recv_variant(&mut rx), "JumpToLetter('#')");
    }

    #[test]
    fn letter_key_without_text_sort_does_nothing() {
        let (mut app, mut rx) = make_test_app();
        let mut grid = make_grid();
        grid.sort = None; // no sort
        app.grid = Some(grid);
        app.focus = FocusPane::Grid;
        let _ = rx.try_recv(); // drain
        app.update(Message::Key(crossterm::event::KeyEvent::new(
            KeyCode::Char('a'),
            KeyModifiers::NONE,
        )));
        let msg = try_recv_variant(&mut rx);
        assert!(!msg.contains("JumpToLetter"), "unexpected: {}", msg);
    }

    #[tokio::test]
    async fn commit_find_repositions_and_fetches_target_window() {
        let (mut app, _rx) = make_test_app();
        let mut grid = make_grid();
        let find_rows = grid.window.rows.clone();
        grid.window.rows.truncate(10);
        grid.window.offset = 0;
        app.grid = Some(grid);

        let columns = app.grid.as_ref().expect("grid").columns.clone();
        let mut find = FindState::new("users".to_string(), columns);
        find.loading = false;
        find.rows = find_rows;
        find.query = "user-40".to_string();
        app.popup = Some(PopupKind::Find(find));
        app.mode = AppMode::Edit;

        app.update(Message::CommitFind);

        assert!(app.popup.is_none(), "find popup should close after commit");
        assert_eq!(app.mode, AppMode::Browse);

        let grid = app.grid.as_ref().expect("grid");
        assert_eq!(grid.focused_row, 40);
        assert_eq!(grid.focused_col, 1);
        assert!(
            grid.viewport_start > 0,
            "viewport should move to target row"
        );
        assert!(
            grid.window.fetch_in_flight,
            "jump should start loading the target window immediately"
        );
    }

    // ---------- help popup navigation ----------

    #[test]
    fn help_scroll_up_and_down() {
        let mut state = HelpState::new();
        // simulate large viewport so scroll is visible
        state.max_scroll = 10;
        state.scroll_down(3);
        assert_eq!(state.scroll, 3);
        state.scroll_up(2);
        assert_eq!(state.scroll, 1);
        state.scroll_up(5);
        assert_eq!(state.scroll, 0); // clamps at 0
        state.scroll_down(100);
        assert_eq!(state.scroll, 10); // clamps at max_scroll
    }

    #[test]
    fn help_up_down_keys_scroll_in_edit_mode() {
        let (mut app, _rx) = make_test_app();
        let mut state = HelpState::new();
        state.max_scroll = 10;
        app.popup = Some(PopupKind::Help(state));
        app.mode = AppMode::Edit;

        app.update(Message::Key(crossterm::event::KeyEvent::new(
            KeyCode::Down,
            KeyModifiers::NONE,
        )));
        assert!(matches!(&app.popup, Some(PopupKind::Help(s)) if s.scroll == 3));

        app.update(Message::Key(crossterm::event::KeyEvent::new(
            KeyCode::Up,
            KeyModifiers::NONE,
        )));
        assert!(matches!(&app.popup, Some(PopupKind::Help(s)) if s.scroll == 0));
    }

    #[test]
    fn help_page_up_down_scrolls_faster() {
        let (mut app, _rx) = make_test_app();
        let mut state = HelpState::new();
        state.max_scroll = 30;
        app.popup = Some(PopupKind::Help(state));
        app.mode = AppMode::Edit;

        app.update(Message::Key(crossterm::event::KeyEvent::new(
            KeyCode::PageDown,
            KeyModifiers::NONE,
        )));
        assert!(matches!(&app.popup, Some(PopupKind::Help(s)) if s.scroll == 10));
    }

    // ---------- confirm dialog ----------

    #[test]
    fn y_confirm_and_n_cancel_in_confirm_dialog() {
        let (mut app, mut rx) = make_test_app();
        app.pending_confirm = Some(PendingConfirm {
            message: "Delete?".to_string(),
            kind: ConfirmKind::DeleteRow {
                table: "users".to_string(),
                rowid: 42,
            },
            created: std::time::Instant::now(),
            timeout_secs: 5,
        });

        // n cancels
        app.update(Message::Key(crossterm::event::KeyEvent::new(
            KeyCode::Char('n'),
            KeyModifiers::NONE,
        )));
        drain_messages(&mut app, &mut rx);
        assert!(app.pending_confirm.is_none());
    }

    #[test]
    fn esc_cancels_confirm_dialog() {
        let (mut app, mut rx) = make_test_app();
        app.pending_confirm = Some(PendingConfirm {
            message: "Delete?".to_string(),
            kind: ConfirmKind::DeleteRow {
                table: "users".to_string(),
                rowid: 42,
            },
            created: std::time::Instant::now(),
            timeout_secs: 5,
        });

        app.update(Message::Key(crossterm::event::KeyEvent::new(
            KeyCode::Esc,
            KeyModifiers::NONE,
        )));
        drain_messages(&mut app, &mut rx);
        assert!(app.pending_confirm.is_none());
    }

    // ---------- mouse handling ----------

    // mouse scroll triggers async fetch which needs tokio runtime;
    // tested instead via scroll shortcuts which verify scroll messages directly

    #[test]
    fn mouse_click_on_grid_focuses_grid() {
        let (mut app, mut rx) = make_test_app();
        app.grid = Some(make_grid());
        app.grid_inner_area = Some(ratatui::layout::Rect {
            x: 12,
            y: 8,
            width: 56,
            height: 16,
        });
        app.focus = FocusPane::Sidebar;
        let _ = rx.try_recv(); // drain
        app.update(Message::Mouse(crossterm::event::MouseEvent {
            kind: crossterm::event::MouseEventKind::Down(crossterm::event::MouseButton::Left),
            column: 20,
            row: 10,
            modifiers: KeyModifiers::NONE,
        }));
        assert!(matches!(app.focus, FocusPane::Grid));
    }

    #[test]
    fn mouse_drag_on_grid_scrollbar_scrolls_rows() {
        let (mut app, mut rx) = make_test_app();
        app.grid = Some(make_grid());
        app.grid.as_mut().expect("grid").window.fetch_in_flight = true;
        app.grid_inner_area = Some(ratatui::layout::Rect {
            x: 12,
            y: 8,
            width: 20,
            height: 13,
        });
        app.focus = FocusPane::Sidebar;
        let _ = rx.try_recv(); // drain

        app.update(Message::Mouse(crossterm::event::MouseEvent {
            kind: crossterm::event::MouseEventKind::Down(crossterm::event::MouseButton::Left),
            column: 31,
            row: 11,
            modifiers: KeyModifiers::NONE,
        }));
        let start_row = app.grid.as_ref().expect("grid").focused_row;
        assert!(app.grid_scrollbar_drag.is_some());

        app.update(Message::Mouse(crossterm::event::MouseEvent {
            kind: crossterm::event::MouseEventKind::Drag(crossterm::event::MouseButton::Left),
            column: 31,
            row: 20,
            modifiers: KeyModifiers::NONE,
        }));
        let dragged_row = app.grid.as_ref().expect("grid").focused_row;

        assert!(matches!(app.focus, FocusPane::Grid));
        assert!(dragged_row > start_row);

        app.update(Message::Mouse(crossterm::event::MouseEvent {
            kind: crossterm::event::MouseEventKind::Up(crossterm::event::MouseButton::Left),
            column: 31,
            row: 20,
            modifiers: KeyModifiers::NONE,
        }));
        assert!(app.grid_scrollbar_drag.is_none());
    }

    #[test]
    fn mouse_wheel_scrolls_text_editor_only_under_the_pointer() {
        let (mut app, _rx) = make_test_app();
        app.popup = Some(PopupKind::TextEditor(TextEditorState::new(
            "users".to_string(),
            1,
            "name".to_string(),
            "TEXT".to_string(),
            SqlValue::Text("x".repeat(1_000)),
            false,
        )));
        app.mode = AppMode::Edit;
        render_test_app(&mut app);
        let initial_scroll = text_editor_scroll_y(&app);
        assert!(initial_scroll > 0);

        app.update(Message::Mouse(crossterm::event::MouseEvent {
            kind: MouseEventKind::ScrollUp,
            column: 40,
            row: 12,
            modifiers: KeyModifiers::NONE,
        }));
        let scrolled = text_editor_scroll_y(&app);
        assert!(scrolled < initial_scroll);

        app.update(Message::Mouse(crossterm::event::MouseEvent {
            kind: MouseEventKind::ScrollDown,
            column: 40,
            row: 12,
            modifiers: KeyModifiers::NONE,
        }));
        assert_eq!(text_editor_scroll_y(&app), initial_scroll);

        app.update(Message::Mouse(crossterm::event::MouseEvent {
            kind: MouseEventKind::ScrollUp,
            column: 1,
            row: 1,
            modifiers: KeyModifiers::NONE,
        }));
        assert_eq!(text_editor_scroll_y(&app), initial_scroll);
    }

    #[test]
    fn mouse_drag_on_text_editor_scrollbar_scrolls_to_end() {
        let (mut app, _rx) = make_test_app();
        app.popup = Some(PopupKind::TextEditor(TextEditorState::new(
            "users".to_string(),
            1,
            "name".to_string(),
            "TEXT".to_string(),
            SqlValue::Text("x".repeat(1_000)),
            false,
        )));
        app.mode = AppMode::Edit;
        render_test_app(&mut app);
        let max_scroll = text_editor_scroll_y(&app);
        if let Some(PopupKind::TextEditor(state)) = app.popup.as_mut() {
            state.scroll_up(u16::MAX);
        }
        render_test_app(&mut app);
        assert_eq!(text_editor_scroll_y(&app), 0);

        app.update(Message::Mouse(crossterm::event::MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: 62,
            row: 10,
            modifiers: KeyModifiers::NONE,
        }));
        app.update(Message::Mouse(crossterm::event::MouseEvent {
            kind: MouseEventKind::Drag(MouseButton::Left),
            column: 62,
            row: 19,
            modifiers: KeyModifiers::NONE,
        }));
        assert_eq!(text_editor_scroll_y(&app), max_scroll);

        app.update(Message::Mouse(crossterm::event::MouseEvent {
            kind: MouseEventKind::Up(MouseButton::Left),
            column: 62,
            row: 19,
            modifiers: KeyModifiers::NONE,
        }));
        app.update(Message::Mouse(crossterm::event::MouseEvent {
            kind: MouseEventKind::Drag(MouseButton::Left),
            column: 62,
            row: 10,
            modifiers: KeyModifiers::NONE,
        }));
        assert_eq!(text_editor_scroll_y(&app), max_scroll);
    }

    #[test]
    fn ctrl_click_on_row_gutter_toggles_rows_without_clearing_selection() {
        let (mut app, _rx) = make_test_app();
        app.grid = Some(make_grid());
        app.grid_inner_area = Some(ratatui::layout::Rect {
            x: 12,
            y: 8,
            width: 56,
            height: 16,
        });

        app.update(Message::Mouse(crossterm::event::MouseEvent {
            kind: crossterm::event::MouseEventKind::Down(crossterm::event::MouseButton::Left),
            column: 12,
            row: 11,
            modifiers: KeyModifiers::CONTROL,
        }));
        app.update(Message::Mouse(crossterm::event::MouseEvent {
            kind: crossterm::event::MouseEventKind::Down(crossterm::event::MouseButton::Left),
            column: 12,
            row: 13,
            modifiers: KeyModifiers::CONTROL,
        }));

        let grid = app.grid.as_ref().expect("grid");
        assert_eq!(grid.focused_row, 2);
        assert_eq!(
            grid.row_selection,
            crate::grid::RowSelection::Rows(BTreeSet::from([0, 2]))
        );
    }

    #[test]
    fn tabbar_hit_test_targets_visible_close_glyph() {
        let (mut app, _rx) = make_test_app();
        app.open_tabs = vec![TableTab {
            table_name: "ghost".to_string(),
        }];
        app.active_tab = Some(0);

        assert!(matches!(
            crate::ui::tabbar::hit_test(
                ratatui::layout::Rect {
                    x: 0,
                    y: 0,
                    width: 20,
                    height: 3,
                },
                &app,
                9,
                1,
                false,
            ),
            Some(TabMouseAction::Close(0))
        ));
    }

    #[test]
    fn mouse_click_on_tab_close_button_closes_tab() {
        let (mut app, mut rx) = make_test_app();
        app.open_tabs = vec![TableTab {
            table_name: "ghost".to_string(),
        }];
        app.active_tab = Some(0);
        app.tabbar_area = ratatui::layout::Rect {
            x: 0,
            y: 0,
            width: 20,
            height: 3,
        };
        app.focus = FocusPane::Sidebar;

        app.update(Message::Mouse(crossterm::event::MouseEvent {
            kind: crossterm::event::MouseEventKind::Down(crossterm::event::MouseButton::Left),
            column: 9,
            row: 1,
            modifiers: KeyModifiers::NONE,
        }));
        drain_messages(&mut app, &mut rx);

        assert!(app.open_tabs.is_empty());
        assert_eq!(app.active_tab, None);
        assert!(matches!(app.focus, FocusPane::Grid));
    }

    #[test]
    fn closing_inactive_tab_keeps_same_active_tab() {
        assert_eq!(next_active_tab_after_close(Some(2), 0, 3), Some(1));
        assert_eq!(next_active_tab_after_close(Some(1), 2, 3), Some(1));
    }

    #[test]
    fn closing_active_tab_activates_previous_tab() {
        assert_eq!(next_active_tab_after_close(Some(2), 2, 3), Some(1));
        assert_eq!(next_active_tab_after_close(Some(0), 0, 2), Some(0));
        assert_eq!(next_active_tab_after_close(Some(0), 0, 0), None);
    }

    // ---------- value picker tests (existing) ----------

    #[test]
    fn value_picker_allows_long_entries_when_distinct_set_is_small() {
        let values = vec![
            "Apple Inc.".to_string(),
            "Embraer - Empresa Brasileira de Aeronáutica S.A.".to_string(),
        ];

        assert!(should_use_value_picker(&values));
    }

    #[test]
    fn value_picker_rejects_empty_and_oversized_distinct_sets() {
        assert!(!should_use_value_picker(&[]));

        let values = (0..101).map(|i| format!("value-{i}")).collect::<Vec<_>>();
        assert!(!should_use_value_picker(&values));
    }

    // ---------- file-watch tests ----------

    fn make_schema_with(table: &str) -> crate::db::schema::Schema {
        let manager = SqliteConnectionManager::memory();
        let pool = r2d2::Pool::builder()
            .max_size(1)
            .build(manager)
            .expect("pool");
        let conn = pool.get().expect("conn");
        conn.execute_batch(&format!("CREATE TABLE {table} (id INTEGER PRIMARY KEY);"))
            .expect("create");
        crate::db::load_schema(&conn).expect("schema")
    }

    #[test]
    fn external_refresh_with_identical_schema_does_not_push_toast() {
        let (mut app, _rx) = make_test_app();
        // First: send a schema with a DIFFERENT table name → should push a toast.
        let different_schema = make_schema_with("other_table");
        let toast_before = app.toast.toasts.len();
        app.update(Message::ExternalRefresh(different_schema));
        assert_eq!(
            app.toast.toasts.len(),
            toast_before + 1,
            "schema change should push a toast"
        );
        // Second: send the same schema again (fingerprint unchanged) → no new toast.
        let same_again = make_schema_with("other_table");
        let toast_mid = app.toast.toasts.len();
        app.update(Message::ExternalRefresh(same_again));
        assert_eq!(
            app.toast.toasts.len(),
            toast_mid,
            "identical schema must not push another toast"
        );
    }

    #[tokio::test]
    async fn file_changed_during_edit_mode_sets_pending_flag_not_needs_fetch() {
        let (mut app, _rx) = make_test_app();
        // Simulate an open grid.
        app.grid = Some(make_grid());
        // Put app into Edit mode (simulates an open popup).
        app.mode = AppMode::Edit;
        app.update(Message::FileChanged);
        assert!(
            app.pending_external_refresh,
            "flag should be set in Edit mode"
        );
        if let Some(ref grid) = app.grid {
            assert!(
                !grid.needs_fetch,
                "needs_fetch must not be set while editing"
            );
        }
    }

    #[tokio::test]
    async fn file_changed_after_own_write_still_refreshes() {
        let (mut app, _rx) = make_test_app();
        app.update(Message::FileChanged);
        assert!(app.file_check_in_flight);
    }

    #[tokio::test]
    async fn close_popup_clears_pending_refresh_and_triggers_fetch() {
        let (mut app, _rx) = make_test_app();
        app.grid = Some(make_grid());
        app.mode = AppMode::Edit;
        app.pending_external_refresh = true;
        app.popup = Some(PopupKind::Help(HelpState::new()));
        app.update(Message::ClosePopup);
        assert!(!app.pending_external_refresh, "flag must be cleared");
        assert_eq!(app.mode, AppMode::Browse);
        if let Some(ref grid) = app.grid {
            assert!(
                grid.window.fetch_in_flight,
                "a refreshed grid fetch must start after close"
            );
        }
    }

    #[test]
    fn stale_window_response_is_ignored() {
        let (mut app, _rx) = make_test_app();
        app.grid = Some(make_grid());
        app.grid_request_serial.store(2, Ordering::Release);
        let original = app.grid.as_ref().expect("grid").window.rows[0].clone();

        app.update(Message::WindowReady {
            request_id: 1,
            table: "users".to_string(),
            offset: 0,
            rows: vec![vec![SqlValue::Text("stale".to_string())]],
            rowids: vec![Some(99)],
            total_rows: 1,
        });

        assert_eq!(app.grid.as_ref().expect("grid").window.rows[0], original);
    }

    #[test]
    fn in_flight_window_completion_preserves_a_queued_scroll_fetch() {
        let (mut app, _rx) = make_test_app();
        let mut grid = make_grid();
        grid.window.fetch_in_flight = true;
        app.grid = Some(grid);

        app.update(Message::ScrollToRow(40));
        assert!(app.grid.as_ref().expect("grid").needs_fetch);
        app.update(Message::WindowReady {
            request_id: 0,
            table: "users".to_string(),
            offset: 0,
            rows: (0..20)
                .map(|index| vec![SqlValue::Integer(index)])
                .collect(),
            rowids: (1..=20).map(Some).collect(),
            total_rows: 100,
        });

        let grid = app.grid.as_ref().expect("grid");
        assert!(grid.needs_fetch);
        assert!(!grid.window.fetch_in_flight);
    }

    #[test]
    fn stale_alphabet_navigation_is_ignored() {
        let (mut app, _rx) = make_test_app();
        app.grid = Some(make_grid());
        app.navigation_request_serial = 2;

        app.update(Message::JumpToSortedOffset {
            request_id: 1,
            table: "users".to_string(),
            offset: 30,
        });

        assert_eq!(app.grid.as_ref().expect("grid").focused_row, 0);
    }

    #[test]
    fn export_failure_does_not_release_the_write_gate() {
        let (mut app, _rx) = make_test_app();
        app.write_in_flight = true;

        app.update(Message::ExportFailed("disk full".to_string()));

        assert!(app.write_in_flight);
    }

    #[test]
    fn opening_direct_editor_invalidates_pending_distinct_lookup() {
        let (mut app, _rx) = make_test_app();
        app.grid = Some(make_grid());
        app.popup_request_serial = 1;
        let column = app.grid.as_ref().expect("grid").columns[0].clone();

        app.update(Message::OpenDirectEdit);
        app.update(Message::DistinctValuesReady {
            request_id: 1,
            table: "users".to_string(),
            rowid: 1,
            col: column,
            original: SqlValue::Integer(0),
            values: vec!["stale".to_string()],
        });

        assert!(matches!(app.popup, Some(PopupKind::TextEditor(_))));
    }

    #[test]
    fn alphabet_navigation_counts_only_rows_in_the_active_filter() {
        let conn = rusqlite::Connection::open_in_memory().expect("database");
        conn.execute_batch(
            "CREATE TABLE items (name TEXT, category TEXT);
             INSERT INTO items VALUES
                ('Alpha', 'kept'),
                ('Bravo', 'hidden'),
                ('Charlie', 'kept'),
                ('Delta', 'kept');",
        )
        .expect("seed rows");
        let mut filter = crate::filter::FilterSet::default();
        filter.columns.insert(
            "category".to_string(),
            crate::filter::ColumnFilter {
                rules: vec![crate::filter::rule::FilterRule {
                    op: crate::filter::FilterOp::Eq,
                    value: crate::filter::FilterValue::Literal(SqlValue::Text("kept".to_string())),
                    enabled: true,
                    label: None,
                }],
            },
        );

        let offset = count_rows_before_letter(&conn, "items", "name", true, 'C', 'C', &filter)
            .expect("count offset");

        assert_eq!(offset, 1);
    }

    #[tokio::test]
    async fn external_refresh_detects_column_changes() {
        let (mut app, _rx) = make_test_app();
        app.grid = Some(make_grid());
        let conn = app.pool.get().expect("connection");
        conn.execute("ALTER TABLE users ADD COLUMN nickname TEXT", [])
            .expect("alter table");
        let changed = db::load_schema(&conn).expect("schema");
        drop(conn);
        app.update(Message::ExternalRefresh(changed));
        assert!(app.schema.tables[0]
            .columns
            .iter()
            .any(|column| column.name == "nickname"));
        assert!(app
            .grid
            .as_ref()
            .expect("active grid")
            .columns
            .iter()
            .any(|column| column.name == "nickname"));
    }

    #[tokio::test]
    async fn schema_refresh_restarts_a_pending_initial_grid_load() {
        let (mut app, _rx) = make_test_app();
        app.grid = None;
        app.open_tabs = vec![TableTab {
            table_name: "users".to_string(),
        }];
        app.active_tab = Some(0);
        app.grid_request_serial.store(1, Ordering::Release);
        let conn = app.pool.get().expect("connection");
        conn.execute("ALTER TABLE users ADD COLUMN nickname TEXT", [])
            .expect("alter table");
        let changed = db::load_schema(&conn).expect("schema");
        drop(conn);

        app.update(Message::ExternalRefresh(changed));
        let current_request = app.grid_request_serial.load(Ordering::Acquire);
        assert!(current_request > 1);
        app.update(Message::GridDataReady {
            request_id: 1,
            table: "users".to_string(),
            columns: make_grid().columns,
            fk_cols: vec![false; 4],
            enumerated_values: vec![Vec::new(); 4],
            width_sample_rows: Vec::new(),
            rows: Vec::new(),
            rowids: Vec::new(),
            total_rows: 0,
        });
        assert!(app.grid.is_none(), "stale initial response must be ignored");
    }

    #[tokio::test]
    async fn dropping_a_table_invalidates_its_pending_initial_grid_load() {
        let (mut app, _rx) = make_test_app();
        app.grid = None;
        app.open_tabs = vec![TableTab {
            table_name: "users".to_string(),
        }];
        app.active_tab = Some(0);
        app.grid_request_serial.store(1, Ordering::Release);
        let conn = app.pool.get().expect("connection");
        conn.execute("DROP TABLE users", []).expect("drop table");
        let changed = db::load_schema(&conn).expect("schema");
        drop(conn);

        app.update(Message::ExternalRefresh(changed));
        assert!(app.grid_request_serial.load(Ordering::Acquire) > 1);
        app.update(Message::GridDataReady {
            request_id: 1,
            table: "users".to_string(),
            columns: make_grid().columns,
            fk_cols: vec![false; 4],
            enumerated_values: vec![Vec::new(); 4],
            width_sample_rows: Vec::new(),
            rows: Vec::new(),
            rowids: Vec::new(),
            total_rows: 0,
        });
        assert!(app.grid.is_none(), "dropped table must not be restored");
    }

    #[tokio::test]
    async fn write_completion_applies_a_pending_schema_refresh() {
        let (mut app, _rx) = make_test_app();
        app.grid = Some(make_grid());
        app.popup = Some(PopupKind::Help(HelpState::new()));
        app.mode = AppMode::Edit;
        let conn = app.pool.get().expect("connection");
        conn.execute("ALTER TABLE users ADD COLUMN nickname TEXT", [])
            .expect("alter table");
        app.schema = db::load_schema(&conn).expect("schema");
        drop(conn);
        app.pending_external_refresh = true;

        app.update(Message::RowInserted {
            table: "users".to_string(),
            rowid: 99,
        });

        assert!(!app.pending_external_refresh);
        assert!(app
            .grid
            .as_ref()
            .expect("grid")
            .columns
            .iter()
            .any(|column| column.name == "nickname"));
    }
}
