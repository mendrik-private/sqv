use std::sync::Arc;

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
    grid::{SortDir, SortSpec},
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
    pub last_own_write_at: Option<std::time::Instant>,
    pub file_check_in_flight: bool,
    pub pending_external_refresh: bool,
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
        table: String,
        columns: Vec<Column>,
        fk_cols: Vec<bool>,
        enumerated_values: Vec<Vec<String>>,
        width_sample_rows: Vec<Vec<SqlValue>>,
        rows: Vec<Vec<SqlValue>>,
        total_rows: i64,
    },
    WindowReady {
        table: String,
        offset: i64,
        rows: Vec<Vec<SqlValue>>,
        total_rows: i64,
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
    DistinctCountReady {
        col: String,
        count: i64,
        values: Vec<String>,
    },
    JumpToFk,
    FkRowsReady {
        target_table: String,
        rows: Vec<Vec<SqlValue>>,
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
        table: String,
        offset: i64,
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
    },
    OpenCommandPalette,
    OpenHelp,
    ExecuteCommand(PaletteCommand),
    ExportDone {
        format: String,
        path: String,
        count: u64,
    },
    ReloadSchema,
    SchemaReady(Schema),
    CopyCell,
    FileChanged,
    ExternalRefresh(Schema),
    OpenFind,
    FindReady(Vec<Vec<SqlValue>>),
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
            last_own_write_at: None,
            file_check_in_flight: false,
            pending_external_refresh: false,
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
                table,
                columns,
                fk_cols,
                enumerated_values,
                width_sample_rows,
                rows,
                total_rows,
            } => {
                self.on_grid_data_ready(GridDataReadyPayload {
                    columns,
                    fk_cols,
                    enumerated_values,
                    width_sample_rows,
                    rows,
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
                table,
                offset,
                rows,
                total_rows,
            } => {
                if let Some(ref mut grid) = self.grid {
                    if grid.table_name == table {
                        grid.window.offset = offset;
                        grid.window.rows = rows;
                        grid.window.total_rows = total_rows;
                        grid.window.fetch_in_flight = false;
                        grid.needs_fetch = false;
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
                    }
                }
                self.dirty = true;
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
                let maybe_fetch = if let Some(ref mut grid) = self.grid {
                    grid.scroll_down(1);
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
            Message::MoveUp => {
                let maybe_fetch = if let Some(ref mut grid) = self.grid {
                    grid.scroll_up(1);
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
                                            .map(|c| format!("\"{}\"", c))
                                            .collect::<Vec<_>>()
                                            .join(", ");
                                        let sql = format!(
                                            "SELECT {} FROM \"{}\" LIMIT 200",
                                            col_list, to_table_c
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
                                if let Ok(Ok(rows)) = result {
                                    let _ = tx.send(Message::FkRowsReady {
                                        target_table: to_table,
                                        rows,
                                    });
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
                    } else {
                        let distinct_values = match self.pool.get() {
                            Ok(conn) => match db::load_distinct_values(
                                &conn,
                                &table_name,
                                &col.name,
                                VALUE_PICKER_DISTINCT_LIMIT + 1,
                            ) {
                                Ok(values) => Some(values),
                                Err(err) => {
                                    self.toast.push(
                                        format!("Distinct lookup failed: {}", err),
                                        ToastKind::Error,
                                    );
                                    None
                                }
                            },
                            Err(err) => {
                                self.toast.push(
                                    format!("DB connection failed: {}", err),
                                    ToastKind::Error,
                                );
                                None
                            }
                        };

                        if let Some(values) =
                            distinct_values.filter(|values| should_use_value_picker(values))
                        {
                            self.popup = Some(PopupKind::ValuePicker(
                                crate::ui::popup::ValuePickerState::new(
                                    table_name,
                                    actual_rowid,
                                    col.name.clone(),
                                    col.col_type.clone(),
                                    values,
                                    original,
                                ),
                            ));
                        } else {
                            self.open_text_editor(
                                table_name,
                                actual_rowid,
                                col.name,
                                col.col_type,
                                original,
                            );
                        }
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
                self.popup = None;
                self.mode = AppMode::Browse;
                if self.pending_external_refresh {
                    self.pending_external_refresh = false;
                    if let Some(ref mut grid) = self.grid {
                        if !grid.window.fetch_in_flight {
                            grid.needs_fetch = true;
                        }
                    }
                }
                self.dirty = true;
            }
            Message::CommitEdit => {
                if self.readonly {
                    self.toast.push("Read-only database", ToastKind::Error);
                    self.popup = None;
                    self.mode = AppMode::Browse;
                    self.dirty = true;
                    return;
                }
                let write_info = self.popup.as_ref().and_then(|p| match p {
                    PopupKind::TextEditor(s) => Some((
                        s.table.clone(),
                        s.col_name.clone(),
                        s.rowid,
                        s.as_sql_value(),
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
                let maybe_fetch = if let Some(ref mut grid) = self.grid {
                    grid.window.rows.clear();
                    if !grid.window.fetch_in_flight {
                        grid.window.fetch_in_flight = true;
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
                        grid.needs_fetch = true;
                        None
                    }
                } else {
                    None
                };
                self.popup = None;
                self.mode = AppMode::Browse;
                self.undo_stack.push(UndoFrame {
                    op: UndoOp::Update,
                    table,
                    rowid,
                    cols: vec![(col, original)],
                });
                if self.undo_stack.len() > 100 {
                    self.undo_stack.remove(0);
                }
                self.last_own_write_at = Some(std::time::Instant::now());
                if let Some((table, cols, sort, off, lim)) = maybe_fetch {
                    self.spawn_window_fetch(&table, &cols, sort, off, lim);
                }
                self.toast.push("Cell updated", ToastKind::Success);
                self.dirty = true;
            }
            Message::EditFailed(err) => {
                self.toast.push(format!("Error: {}", err), ToastKind::Error);
                self.dirty = true;
            }
            Message::DistinctCountReady { .. } => {}
            Message::FkRowsReady { target_table, rows } => {
                if let Some(PopupKind::FkPicker(ref mut state)) = self.popup {
                    if state.target_table == target_table {
                        state.rows = rows;
                        state.loading = false;
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
                    let abs_row = g.focused_row as i64;
                    let cell_val = g.window.get_row(abs_row)?.get(col_idx)?.clone();
                    let sort = g.sort.as_ref().and_then(|s| {
                        g.columns
                            .get(s.col_idx)
                            .map(|c| (c.name.clone(), s.direction == SortDir::Asc))
                    });
                    Some((
                        g.table_name.clone(),
                        abs_row,
                        col_idx,
                        sort,
                        g.filter.clone(),
                        fk.to_table.clone(),
                        fk.to_col.clone(),
                        cell_val,
                    ))
                });

                if let Some((
                    from_table,
                    abs_row,
                    from_col,
                    from_sort,
                    from_filter,
                    to_table,
                    to_col,
                    cell_val,
                )) = jump_info
                {
                    let Some(source_rowid) =
                        self.resolve_rowid_at_offset(&from_table, abs_row, from_sort, from_filter)
                    else {
                        self.dirty = true;
                        return;
                    };
                    let frame = JumpFrame {
                        table: from_table,
                        rowid: source_rowid,
                        col: from_col,
                    };
                    self.jump_stack.push(frame);
                    let _ = self.tx.send(Message::OpenTable(to_table.clone()));
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
                                    _ => rusqlite::types::Value::Null,
                                };
                                let rowid: Option<i64> = conn
                                    .query_row(
                                        &format!(
                                            "SELECT rowid FROM \"{}\" WHERE \"{}\" = ?1 LIMIT 1",
                                            to_table_c, to_col_c
                                        ),
                                        rusqlite::params![val],
                                        |row| row.get(0),
                                    )
                                    .optional()?;
                                Ok(rowid)
                            })
                            .await;
                        if let Ok(Ok(Some(rowid))) = result {
                            let _ = tx.send(Message::JumpToTargetRow {
                                table: to_table,
                                rowid,
                                col: None,
                            });
                        }
                    });
                }
                self.dirty = true;
            }
            Message::JumpToTargetRow { table, rowid, col } => self.jump_to_rowid(table, rowid, col),
            Message::CycleSort => {
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
                    if !grid.window.fetch_in_flight {
                        grid.window.fetch_in_flight = true;
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
                        grid.needs_fetch = true;
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
            Message::JumpToSortedOffset { table, offset } => {
                let maybe_fetch = if let Some(ref mut grid) = self.grid {
                    if grid.table_name == table {
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
                            let letter_uc = letter.to_uppercase().next().unwrap_or(letter);
                            let table_inner = table.clone();
                            tokio::task::spawn(async move {
                                let result = tokio::task::spawn_blocking(move || -> anyhow::Result<i64> {
                                    let conn = pool.get()?;
                                    let offset: i64 = if dir_asc {
                                        if letter == '#' {
                                            conn.query_row(
                                                &format!(
                                                    "SELECT COUNT(*) FROM \"{}\" WHERE \"{}\" IS NULL",
                                                    table_inner, col_name
                                                ),
                                                [],
                                                |row| row.get(0),
                                            )?
                                        } else {
                                            conn.query_row(
                                                &format!(
                                                    "SELECT COUNT(*) FROM \"{}\" WHERE \"{}\" IS NULL OR \"{}\" < ?1",
                                                    table_inner, col_name, col_name
                                                ),
                                                rusqlite::params![letter_uc.to_string()],
                                                |row| row.get(0),
                                            )?
                                        }
                                    } else if letter == '#' {
                                        conn.query_row(
                                            &format!(
                                                "SELECT COUNT(*) FROM \"{}\" WHERE \"{}\" IS NOT NULL AND \"{}\" NOT GLOB '[0-9]*'",
                                                table_inner, col_name, col_name
                                            ),
                                            [],
                                            |row| row.get(0),
                                        )?
                                    } else {
                                        let pattern = format!("{}%", letter_uc);
                                        conn.query_row(
                                            &format!(
                                                "SELECT COUNT(*) FROM \"{}\" WHERE \"{}\" > ?1 AND \"{}\" NOT LIKE ?2",
                                                table_inner, col_name, col_name
                                            ),
                                            rusqlite::params![letter_uc.to_string(), pattern],
                                            |row| row.get(0),
                                        )?
                                    };
                                    Ok(offset)
                                })
                                .await;
                                if let Ok(Ok(offset)) = result {
                                    let _ = tx.send(Message::JumpToSortedOffset { table, offset });
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
                        let _ = crate::filter::save_filter(&filter, &db_path, &table);
                        self.spawn_window_fetch_with_filter(&table, &cols, sort, off, lim, filter);
                    }
                    self.mode = AppMode::Browse;
                }
                self.dirty = true;
            }
            Message::ClearFilters => {
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
                    let _ = crate::filter::save_filter(&empty_filter, &db_path, &table);
                    self.spawn_window_fetch_with_filter(
                        &table,
                        &cols,
                        sort,
                        off,
                        lim,
                        empty_filter,
                    );
                }
                self.popup = None;
                self.mode = AppMode::Browse;
                self.dirty = true;
            }
            Message::InsertRow => {
                if self.readonly {
                    self.toast.push("Read-only database", ToastKind::Error);
                    return;
                }
                if let Some(ref grid) = self.grid {
                    self.popup = Some(PopupKind::InsertRow(InsertRowState::new(
                        grid.table_name.clone(),
                        grid.columns.clone(),
                    )));
                    self.mode = AppMode::Edit;
                }
                self.dirty = true;
            }
            Message::CommitInsertRow => {
                if self.readonly {
                    self.toast.push("Read-only database", ToastKind::Error);
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
                self.popup = None;
                self.mode = AppMode::Browse;
                self.last_own_write_at = Some(std::time::Instant::now());
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
                        grid.window.total_rows += 1;
                        grid.needs_fetch = true;
                    }
                }
                self.toast.push("Row inserted", ToastKind::Success);
                self.dirty = true;
            }
            Message::DeleteRow => {
                if self.readonly {
                    self.toast.push("Read-only database", ToastKind::Error);
                    return;
                }
                let delete_context = self.grid.as_ref().map(|grid| {
                    let row_num = grid.focused_row + 1;
                    let table = grid.table_name.clone();
                    let abs_row = grid.focused_row as i64;
                    let sort = grid.sort.as_ref().and_then(|s| {
                        grid.columns
                            .get(s.col_idx)
                            .map(|c| (c.name.clone(), s.direction == SortDir::Asc))
                    });
                    (row_num, table, abs_row, sort, grid.filter.clone())
                });
                if let Some((row_num, table, abs_row, sort, filter)) = delete_context {
                    let Some(rowid) = self.resolve_rowid_at_offset(&table, abs_row, sort, filter)
                    else {
                        self.dirty = true;
                        return;
                    };
                    let msg = format!("Delete row #{}? [y/n]", row_num);
                    self.pending_confirm = Some(PendingConfirm {
                        message: msg,
                        kind: ConfirmKind::DeleteRow { table, rowid },
                        created: std::time::Instant::now(),
                        timeout_secs: 5,
                    });
                }
                self.dirty = true;
            }
            Message::ConfirmDelete => {
                if let Some(confirm) = self.pending_confirm.take() {
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
                                drop(columns);
                                let tx_err = tx_ch.clone();
                                let result =
                                    tokio::task::spawn_blocking(move || -> anyhow::Result<()> {
                                        let conn = pool.get()?;
                                        crate::db::write::delete_row(&conn, &table_c, rowid)?;
                                        let _ = tx_ch.send(Message::RowDeleted {
                                            table: table_c,
                                            rowid,
                                        });
                                        Ok(())
                                    })
                                    .await;
                                if let Ok(Err(e)) = result {
                                    let _ = tx_err.send(Message::EditFailed(e.to_string()));
                                }
                            });
                        }
                    }
                }
                self.dirty = true;
            }
            Message::RowDeleted { table, rowid } => {
                self.last_own_write_at = Some(std::time::Instant::now());
                self.undo_stack.push(UndoFrame {
                    op: UndoOp::Delete,
                    table: table.clone(),
                    rowid,
                    cols: Vec::new(),
                });
                if self.undo_stack.len() > 100 {
                    self.undo_stack.remove(0);
                }
                if let Some(ref mut grid) = self.grid {
                    if grid.table_name == table {
                        grid.window.rows.clear();
                        grid.window.total_rows = grid.window.total_rows.saturating_sub(1);
                        grid.needs_fetch = true;
                    }
                }
                self.toast.push("Row deleted", ToastKind::Success);
                self.dirty = true;
            }
            Message::CancelConfirm => {
                self.pending_confirm = None;
                self.dirty = true;
            }
            Message::UndoAction => {
                if let Some(frame) = self.undo_stack.pop() {
                    match frame.op {
                        UndoOp::Update => {
                            if self.readonly {
                                self.toast.push("Read-only: cannot undo", ToastKind::Error);
                                return;
                            }
                            let pool = Arc::clone(&self.pool);
                            let tx = self.tx.clone();
                            let table = frame.table.clone();
                            let rowid = frame.rowid;
                            let cols = frame.cols.clone();
                            tokio::task::spawn(async move {
                                let result =
                                    tokio::task::spawn_blocking(move || -> anyhow::Result<()> {
                                        let conn = pool.get()?;
                                        for (col_name, value) in &cols {
                                            crate::db::write::commit_cell_edit(
                                                &conn, &table, col_name, rowid, value,
                                            )?;
                                        }
                                        Ok(())
                                    })
                                    .await;
                                if let Ok(Err(e)) = result {
                                    let _ = tx.send(Message::EditFailed(e.to_string()));
                                }
                            });
                            let toast_msg = if frame.cols.len() == 1 {
                                format!(
                                    "Undo: reverted col \"{}\" on row {}",
                                    frame.cols[0].0, frame.rowid
                                )
                            } else {
                                format!(
                                    "Undo: reverted {} cols on row {}",
                                    frame.cols.len(),
                                    frame.rowid
                                )
                            };
                            self.toast.push(toast_msg, ToastKind::Info);
                        }
                        UndoOp::Insert => {
                            let pool = Arc::clone(&self.pool);
                            let tx = self.tx.clone();
                            let table = frame.table.clone();
                            let rowid = frame.rowid;
                            tokio::task::spawn(async move {
                                let result = tokio::task::spawn_blocking(move || {
                                    let conn = pool.get()?;
                                    crate::db::write::delete_row(&conn, &table, rowid)?;
                                    Ok::<_, anyhow::Error>(())
                                })
                                .await;
                                if let Ok(Err(e)) = result {
                                    let _ = tx.send(Message::EditFailed(e.to_string()));
                                }
                            });
                            self.toast.push(
                                format!("Undo: deleted inserted row {}", frame.rowid),
                                ToastKind::Info,
                            );
                        }
                        UndoOp::Delete => {
                            if frame.cols.is_empty() {
                                self.toast.push(
                                    "Undo: cannot restore deleted row (no backup)",
                                    ToastKind::Error,
                                );
                                return;
                            }
                            let pool = Arc::clone(&self.pool);
                            let tx = self.tx.clone();
                            let table = frame.table.clone();
                            let rowid = frame.rowid;
                            let cols = frame.cols.clone();
                            tokio::task::spawn(async move {
                                let result =
                                    tokio::task::spawn_blocking(move || -> anyhow::Result<()> {
                                        let conn = pool.get()?;
                                        crate::db::write::reinsert_row(
                                            &conn, &table, rowid, &cols,
                                        )?;
                                        Ok(())
                                    })
                                    .await;
                                if let Ok(Err(e)) = result {
                                    let _ = tx.send(Message::EditFailed(e.to_string()));
                                }
                            });
                            self.toast.push(
                                format!("Undo: restored deleted row {}", rowid),
                                ToastKind::Info,
                            );
                        }
                    }
                    if let Some(ref mut grid) = self.grid {
                        grid.window.rows.clear();
                        grid.needs_fetch = true;
                    }
                    self.dirty = true;
                } else {
                    self.toast.push("Nothing to undo", ToastKind::Info);
                    self.dirty = true;
                }
            }
            Message::OpenCommandPalette => {
                let table_names = self.schema.tables.iter().map(|t| t.name.clone()).collect();
                let state = CommandPaletteState::new(table_names);
                self.popup = Some(PopupKind::CommandPalette(state));
                self.mode = AppMode::Edit;
                self.dirty = true;
            }
            Message::OpenHelp => {
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
            Message::ReloadSchema => {
                let pool = Arc::clone(&self.pool);
                let tx = self.tx.clone();
                tokio::task::spawn(async move {
                    let result = tokio::task::spawn_blocking(move || {
                        let conn = pool.get()?;
                        crate::db::load_schema(&conn)
                    })
                    .await;
                    if let Ok(Ok(schema)) = result {
                        let _ = tx.send(Message::SchemaReady(schema));
                    }
                });
                self.dirty = true;
            }
            Message::SchemaReady(schema) => {
                self.schema = schema;
                self.sidebar.tables_expanded = true;
                self.toast.push("Schema reloaded", ToastKind::Success);
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
                    use std::io::Write;
                    let encoded = base64_encode(text.as_bytes());
                    let osc52 = format!("\x1b]52;c;{}\x07", encoded);
                    let _ = std::io::stdout().write_all(osc52.as_bytes());
                    let _ = std::io::stdout().flush();
                    self.toast.push("Copied to clipboard", ToastKind::Success);
                }
                self.dirty = true;
            }
            Message::FileChanged => {
                // Ignore events caused by our own writes (500 ms debounce window).
                if self
                    .last_own_write_at
                    .is_some_and(|t| t.elapsed() < std::time::Duration::from_millis(500))
                {
                    return;
                }
                // Deduplicate rapid bursts (e.g. WAL flush + main file write).
                if self.file_check_in_flight {
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
                    if let Ok(Ok(schema)) = result {
                        let _ = tx.send(Message::ExternalRefresh(schema));
                    }
                });

                // Schedule data refresh, but not while a popup is open.
                if self.mode == AppMode::Edit {
                    self.pending_external_refresh = true;
                } else if let Some(ref mut grid) = self.grid {
                    if !grid.window.fetch_in_flight {
                        grid.needs_fetch = true;
                    }
                }
                self.dirty = true;
            }
            Message::ExternalRefresh(new_schema) => {
                self.file_check_in_flight = false;

                // Build sorted fingerprints: (name, "table"/"view"/"index").
                let mut old_fp: Vec<(String, &str)> = self
                    .schema
                    .tables
                    .iter()
                    .map(|t| (t.name.clone(), "table"))
                    .chain(self.schema.views.iter().map(|v| (v.name.clone(), "view")))
                    .chain(
                        self.schema
                            .indexes
                            .iter()
                            .map(|i| (i.name.clone(), "index")),
                    )
                    .collect();
                old_fp.sort_unstable();

                let mut new_fp: Vec<(String, &str)> = new_schema
                    .tables
                    .iter()
                    .map(|t| (t.name.clone(), "table"))
                    .chain(new_schema.views.iter().map(|v| (v.name.clone(), "view")))
                    .chain(new_schema.indexes.iter().map(|i| (i.name.clone(), "index")))
                    .collect();
                new_fp.sort_unstable();

                if old_fp != new_fp {
                    self.schema = new_schema;
                    let total = self.sidebar.visible_count(&self.schema);
                    if self.sidebar.selected >= total {
                        self.sidebar.selected = total.saturating_sub(1);
                    }
                    self.toast.push("Schema changed", ToastKind::Info);
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

                let pool = Arc::clone(&self.pool);
                let tx = self.tx.clone();
                tokio::task::spawn(async move {
                    let result = tokio::task::spawn_blocking(
                        move || -> anyhow::Result<Vec<Vec<SqlValue>>> {
                            let conn = pool.get()?;
                            let (where_clause, where_params) = filter_to_sql(&filter);
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
                    if let Ok(Ok(rows)) = result {
                        let _ = tx.send(Message::FindReady(rows));
                    }
                });
            }
            Message::FindReady(rows) => {
                if let Some(PopupKind::Find(ref mut state)) = self.popup {
                    state.rows = rows;
                    state.loading = false;
                    self.dirty = true;
                }
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

                self.popup = None;
                self.mode = AppMode::Browse;
                self.dirty = true;

                if let Some((table, cols, sort, off, lim)) = maybe_fetch {
                    self.spawn_window_fetch(&table, &cols, sort, off, lim);
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
                    let export_path = format!("{}/sqview_export.{}", home, format);
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
                        } else if let Ok(Err(e)) = result {
                            let _ = tx.send(Message::EditFailed(e.to_string()));
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
                self.copy_row_as_json();
            }
            PaletteCommand::Quit => {
                self.should_quit = true;
            }
        }
    }

    fn copy_row_as_json(&self) {
        let json = self.grid.as_ref().and_then(|g| {
            let abs_row = g.focused_row as i64;
            let row = g.window.get_row(abs_row)?;
            let fields: Vec<String> = g
                .columns
                .iter()
                .zip(row)
                .map(|(col, val)| {
                    let v_json = match val {
                        SqlValue::Null => "null".to_string(),
                        SqlValue::Integer(n) => n.to_string(),
                        SqlValue::Real(f) => f.to_string(),
                        SqlValue::Text(s) => format!("\"{}\"", s.replace('"', "\\\"")),
                        SqlValue::Blob(_) => "null".to_string(),
                    };
                    format!("\"{}\": {}", col.name.replace('"', "\\\""), v_json)
                })
                .collect();
            Some(format!("{{{}}}", fields.join(", ")))
        });
        if let Some(text) = json {
            use std::io::Write;
            let encoded = base64_encode(text.as_bytes());
            let osc52 = format!("\x1b]52;c;{}\x07", encoded);
            let _ = std::io::stdout().write_all(osc52.as_bytes());
            let _ = std::io::stdout().flush();
        }
    }

    fn handle_mouse(&mut self, mouse: crossterm::event::MouseEvent) {
        use crossterm::event::{MouseButton, MouseEventKind};

        if matches!(self.popup, Some(PopupKind::FilterPopup(_))) {
            self.handle_filter_popup_mouse(mouse);
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
                                grid.focus_cell(row, grid.focused_col);
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
                    return;
                }
                KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => {
                    let _ = self.tx.send(Message::CancelConfirm);
                    return;
                }
                _ => {}
            }
        }

        // ? toggles help (from any mode, unless confirming)
        if key.code == KeyCode::Char('?') {
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
        if key.code == KeyCode::Enter && key.modifiers.contains(KeyModifiers::CONTROL) {
            match self.popup {
                Some(PopupKind::ValuePicker(_))
                | Some(PopupKind::DatePicker(_))
                | Some(PopupKind::DatetimePicker(_))
                | Some(PopupKind::InsertRow(_))
                | Some(PopupKind::FkPicker(_)) => {
                    let _ = self.tx.send(match self.popup {
                        Some(PopupKind::InsertRow(_)) => Message::CommitInsertRow,
                        _ => Message::CommitEdit,
                    });
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
            Some(PopupKind::InsertRow(state)) => match key.code {
                KeyCode::Esc => {
                    let _ = self.tx.send(Message::ClosePopup);
                }
                KeyCode::Enter => {
                    if state.editing {
                        state.stop_editing();
                    } else {
                        state.start_editing();
                    }
                    self.dirty = true;
                }
                KeyCode::Up if !state.editing => {
                    state.move_up();
                    self.dirty = true;
                }
                KeyCode::Down if !state.editing => {
                    state.move_down();
                    self.dirty = true;
                }
                KeyCode::Left if state.editing => {
                    state.move_cursor_left();
                    self.dirty = true;
                }
                KeyCode::Right if state.editing => {
                    state.move_cursor_right();
                    self.dirty = true;
                }
                KeyCode::Backspace if state.editing => {
                    state.delete_backward();
                    self.dirty = true;
                }
                KeyCode::Delete => {
                    state.reset_selected();
                    self.dirty = true;
                }
                KeyCode::Char(c)
                    if state.editing
                        && (key.modifiers == KeyModifiers::NONE
                            || key.modifiers == KeyModifiers::SHIFT) =>
                {
                    state.insert_char(c);
                    self.dirty = true;
                }
                _ => {}
            },
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
                    self.popup = None;
                    self.mode = AppMode::Browse;
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
                self.focus = FocusPane::Sidebar;
                self.dirty = true;
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
            (KeyCode::Char('i'), KeyModifiers::NONE) => {
                let _ = self.tx.send(Message::InsertRow);
            }
            (KeyCode::Char('d'), KeyModifiers::NONE) => {
                let _ = self.tx.send(Message::DeleteRow);
            }
            (KeyCode::Char(c), KeyModifiers::NONE)
                if c.is_alphabetic()
                    && !matches!(c, 'j' | 'k' | 'h' | 'l' | 's' | 'f' | 'i' | 'd' | 'e' | 'n') =>
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
        let table_name = grid.table_name.clone();
        let abs_row = grid.focused_row as i64;
        let cell_value = grid
            .window
            .get_row(abs_row)
            .and_then(|row| row.get(col_idx))
            .cloned()
            .unwrap_or(SqlValue::Null);
        let is_fk = grid.fk_cols.get(col_idx).copied().unwrap_or(false);
        let sort = grid.sort.as_ref().and_then(|s| {
            grid.columns
                .get(s.col_idx)
                .map(|c| (c.name.clone(), s.direction == SortDir::Asc))
        });
        let filter = grid.filter.clone();
        let rowid = self.resolve_rowid_at_offset(&table_name, abs_row, sort, filter)?;

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

    fn open_text_editor(
        &mut self,
        table: String,
        rowid: i64,
        col_name: String,
        col_type: String,
        original: SqlValue,
    ) {
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

    fn spawn_grid_fetch(&self, table: String, columns: Vec<Column>, fk_cols: Vec<bool>) {
        let tx = self.tx.clone();
        let pool = Arc::clone(&self.pool);
        tokio::task::spawn(async move {
            let table_c = table.clone();
            let cols_c = columns.clone();
            let result = tokio::task::spawn_blocking(move || -> anyhow::Result<GridFetchResult> {
                let conn = pool.get()?;
                let total = db::count_rows(&conn, &table_c, "", &[])?;
                let rows = db::fetch_rows(
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
                let enumerated_values = cols_c
                    .iter()
                    .map(|col| {
                        db::load_distinct_values(
                            &conn,
                            &table_c,
                            &col.name,
                            ENUM_COLOR_DISTINCT_LIMIT,
                        )
                        .map(|values| normalize_enumerated_values(values, total))
                        .unwrap_or_default()
                    })
                    .collect();
                let width_sample_rows = db::fetch_random_rows(&conn, &table_c, &cols_c, 50)
                    .unwrap_or_else(|_| rows.iter().take(50).cloned().collect());
                Ok((rows, total, enumerated_values, width_sample_rows))
            })
            .await;
            if let Ok(Ok((rows, total_rows, enumerated_values, width_sample_rows))) = result {
                let _ = tx.send(Message::GridDataReady {
                    table,
                    columns,
                    fk_cols,
                    enumerated_values,
                    width_sample_rows,
                    rows,
                    total_rows,
                });
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
        let pool = Arc::clone(&self.pool);
        let tx = self.tx.clone();
        let table = table.to_string();
        let columns = columns.to_vec();
        tokio::task::spawn(async move {
            let table_c = table.clone();
            let result = tokio::task::spawn_blocking(
                move || -> anyhow::Result<(Vec<Vec<SqlValue>>, i64)> {
                    let conn = pool.get()?;
                    let (where_clause, where_params) = filter_to_sql(&filter);
                    let total = db::count_rows(&conn, &table_c, &where_clause, &where_params)?;
                    let order_by = sort.as_ref().map(|(s, b)| (s.as_str(), *b));
                    let rows = db::fetch_rows(
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
                },
            )
            .await;
            if let Ok(Ok((rows, total_rows))) = result {
                let _ = tx.send(Message::WindowReady {
                    table,
                    offset,
                    rows,
                    total_rows,
                });
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

    fn resolve_rowid_at_offset(
        &mut self,
        table: &str,
        offset: i64,
        sort: Option<(String, bool)>,
        filter: crate::filter::FilterSet,
    ) -> Option<i64> {
        let (where_clause, where_params) = filter_to_sql(&filter);
        let conn = match self.pool.get() {
            Ok(conn) => conn,
            Err(err) => {
                self.toast
                    .push(format!("DB connection failed: {}", err), ToastKind::Error);
                return None;
            }
        };
        match db::fetch_rowid_at_offset(
            &conn,
            table,
            offset,
            sort.as_ref().map(|(col, asc)| (col.as_str(), *asc)),
            &where_clause,
            &where_params,
        ) {
            Ok(Some(rowid)) => Some(rowid),
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

    fn resolve_offset_for_rowid(
        &mut self,
        table: &str,
        rowid: i64,
        sort: Option<(String, bool)>,
        filter: crate::filter::FilterSet,
    ) -> Option<i64> {
        let (where_clause, where_params) = filter_to_sql(&filter);
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
            let _ = crate::filter::save_filter(&filter, &db_path, &table);
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
    use std::sync::Arc;

    use crossterm::event::{KeyCode, KeyModifiers};
    use r2d2_sqlite::SqliteConnectionManager;
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
        crate::grid::GridState::new(crate::grid::GridInit {
            table_name: "users".to_string(),
            columns,
            fk_cols: vec![false; 4],
            enumerated_values: vec![Vec::new(); 4],
            rows,
            width_sample_rows: vec![Vec::new(); 4],
            total_rows: 50,
            area_width: 80,
        })
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
        loop {
            match rx.try_recv() {
                Ok(msg) => {
                    // avoid infinite recursion for actions that spawn more messages
                    app.update(msg);
                    if rx.try_recv().is_err() {
                        break;
                    }
                }
                Err(_) => break,
            }
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
    fn esc_in_grid_sets_focus_to_sidebar() {
        let (mut app, _rx) = make_test_app();
        app.grid = Some(make_grid());
        app.focus = FocusPane::Grid;
        app.update(Message::Key(crossterm::event::KeyEvent::new(
            KeyCode::Esc,
            KeyModifiers::NONE,
        )));
        assert!(matches!(app.focus, FocusPane::Sidebar));
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
    fn insert_row_opens_insert_popup_for_constrained_table() {
        let (mut app, _rx) = make_constrained_insert_app();
        app.focus = FocusPane::Grid;

        app.update(Message::InsertRow);

        assert!(matches!(app.popup, Some(PopupKind::InsertRow(_))));
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
        assert_eq!(state.scroll, 9); // clamps at max_scroll-1
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

    #[test]
    fn file_changed_within_own_write_debounce_is_ignored() {
        let (mut app, _rx) = make_test_app();
        // Mark a very recent own write.
        app.last_own_write_at = Some(std::time::Instant::now());
        app.update(Message::FileChanged);
        // file_check_in_flight should NOT be set (handler returned early).
        assert!(
            !app.file_check_in_flight,
            "in-flight must not be set when debounced"
        );
    }

    #[test]
    fn close_popup_clears_pending_refresh_and_triggers_fetch() {
        let (mut app, _rx) = make_test_app();
        app.grid = Some(make_grid());
        app.mode = AppMode::Edit;
        app.pending_external_refresh = true;
        app.popup = Some(PopupKind::Help(HelpState::new()));
        app.update(Message::ClosePopup);
        assert!(!app.pending_external_refresh, "flag must be cleared");
        assert_eq!(app.mode, AppMode::Browse);
        if let Some(ref grid) = app.grid {
            assert!(grid.needs_fetch, "needs_fetch must be set after close");
        }
    }
}
