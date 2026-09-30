//! Inspection, search, console, export and clipboard features built on the
//! grid: each opens a popup or starts background work and handles its result.

use std::path::PathBuf;

use super::{App, FocusedCellContext, JumpFrame, Message};
use crate::{
    db::{
        self,
        query::ViewQuery,
        schema::Column,
        types::{ColumnKind, SqlValue},
        SearchHit, SqlOutcome,
    },
    export::{self, ExportFormat, ExportRows},
    filter::{ColumnFilter, Condition, FilterRule},
    grid::RowSelection,
    ui::{
        popup::{
            command_palette::CopyFormat,
            find::FIND_LIMIT,
            fk_picker::FK_PICKER_LIMIT,
            global_search::{GlobalHit, GLOBAL_SEARCH_MIN_CHARS, GLOBAL_SEARCH_PER_TABLE},
            record::RecordInit,
            references::Reference,
            schema_view::SchemaLine,
            sql_console::SQL_ROW_LIMIT,
            ExportState, FindState, FkPickerState, GlobalSearchState, GoToRowState, JsonViewState,
            PopupKind, RecordState, ReferencesState, SchemaViewState, SqlConsoleState,
        },
        toast::ToastKind,
    },
};

/// Matches shown for one search across all tables.
const GLOBAL_SEARCH_LIMIT: usize = 500;

/// Which rows of the view an action works on.
enum RowScope {
    Focused(i64),
    Offsets(Vec<i64>),
    AllExcept(Vec<i64>),
}

impl App {
    // ── find & pickers ───────────────────────────────────────────────────

    pub(super) fn open_find(&mut self) {
        let Some(grid) = self.grid.as_ref() else {
            return;
        };
        let state = FindState::new(grid.table_name.clone(), grid.columns.clone());
        self.open_popup(PopupKind::Find(state));
        self.run_popup_search();
    }

    /// Opens the foreign-key picker for `cell`; false when the column's
    /// reference cannot be resolved in the schema.
    pub(super) fn open_fk_picker(&mut self, cell: &FocusedCellContext) -> bool {
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
        // The referenced key first, then the descriptive columns.
        let columns = std::iter::once(key_column.clone())
            .chain(
                target_columns
                    .iter()
                    .filter(|c| c.name != fk.to_col)
                    .cloned(),
            )
            .collect();
        self.open_popup(PopupKind::FkPicker(FkPickerState::new(
            fk.to_table,
            columns,
            cell.table_name.clone(),
            cell.col.name.clone(),
            cell.rowid,
            cell.cell_value.clone(),
        )));
        self.run_popup_search();
        true
    }

    /// Runs the search of the open Find, foreign-key picker or global search
    /// popup for its current text; older results become stale.
    pub(super) fn run_popup_search(&mut self) {
        let search = match &self.popup {
            Some(PopupKind::Find(state)) => {
                self.grid
                    .as_ref()
                    .map(|grid| grid.view_query())
                    .map(|view| {
                        (
                            view,
                            state.columns.clone(),
                            state.query().to_string(),
                            FIND_LIMIT,
                        )
                    })
            }
            Some(PopupKind::FkPicker(state)) => Some((
                Ok(ViewQuery::table(&state.target_table)),
                state.columns.clone(),
                state.query().to_string(),
                FK_PICKER_LIMIT,
            )),
            Some(PopupKind::GlobalSearch(state)) => {
                let needle = state.query().to_string();
                self.run_global_search(needle);
                return;
            }
            _ => None,
        };
        let Some((view, columns, needle, limit)) = search else {
            return;
        };
        let view = match view {
            Ok(view) => view,
            Err(error) => {
                self.on_search_failed(error.to_string());
                return;
            }
        };
        let request_id = self.next_popup_request();
        self.spawn_db(
            move |conn| db::search_view(conn, &view, &columns, &needle, limit),
            move |result| match result {
                Ok(hits) => Message::SearchReady { request_id, hits },
                Err(error) => Message::SearchFailed { request_id, error },
            },
        );
    }

    pub(super) fn on_search_ready(&mut self, hits: Vec<SearchHit>) {
        let symbols = &self.symbols;
        match &mut self.popup {
            Some(PopupKind::Find(state)) => state.set_hits(hits, symbols),
            Some(PopupKind::FkPicker(state)) => state.set_hits(hits, symbols),
            _ => {}
        }
        self.dirty = true;
    }

    pub(super) fn on_search_failed(&mut self, error: String) {
        match &mut self.popup {
            Some(PopupKind::Find(state)) => state.panel.set_error(error),
            Some(PopupKind::FkPicker(state)) => state.panel.set_error(error),
            Some(PopupKind::GlobalSearch(state)) => state.panel.set_error(error),
            _ => self.toast.push(error, ToastKind::Error),
        }
        self.dirty = true;
    }

    /// Focuses the selected Find match, on the first column that matches.
    pub(super) fn commit_find(&mut self) {
        let target = match &self.popup {
            Some(PopupKind::Find(state)) => state.selected_offset().map(|offset| {
                let needle = state.query().to_lowercase();
                let col = state.panel.table.selected_row().and_then(|values| {
                    values.iter().position(|value| {
                        !needle.is_empty() && value.to_text().to_lowercase().contains(&needle)
                    })
                });
                (offset, col)
            }),
            _ => None,
        };
        self.finish_popup();
        if let Some((offset, col)) = target {
            self.update_grid(|grid| {
                let col = col.unwrap_or(grid.focused_col);
                grid.focus_cell(offset as usize, col);
            });
        }
    }

    // ── global search ────────────────────────────────────────────────────

    pub(super) fn open_global_search(&mut self) {
        self.open_popup(PopupKind::GlobalSearch(GlobalSearchState::new()));
    }

    fn run_global_search(&mut self, needle: String) {
        let request_id = self.next_popup_request();
        if needle.chars().count() < GLOBAL_SEARCH_MIN_CHARS {
            let symbols = &self.symbols;
            if let Some(PopupKind::GlobalSearch(state)) = &mut self.popup {
                state.set_hits(Vec::new(), GLOBAL_SEARCH_LIMIT, symbols);
            }
            return;
        }
        let tables: Vec<(String, Vec<Column>)> = self
            .schema
            .tables
            .iter()
            .chain(&self.schema.views)
            .map(|table| (table.name.clone(), table.columns.clone()))
            .collect();
        if let Some(PopupKind::GlobalSearch(state)) = &mut self.popup {
            state.panel.loading = true;
        }
        self.spawn_db(
            move |conn| global_search(conn, &tables, &needle),
            move |result| match result {
                Ok(hits) => Message::GlobalSearchReady { request_id, hits },
                Err(error) => Message::SearchFailed { request_id, error },
            },
        );
    }

    pub(super) fn on_global_search_ready(&mut self, hits: Vec<GlobalHit>) {
        let symbols = &self.symbols;
        if let Some(PopupKind::GlobalSearch(state)) = &mut self.popup {
            state.set_hits(hits, GLOBAL_SEARCH_LIMIT, symbols);
        }
        self.dirty = true;
    }

    pub(super) fn commit_global_search(&mut self) {
        let Some(PopupKind::GlobalSearch(state)) = &self.popup else {
            return;
        };
        let Some(hit) = state.selected().cloned() else {
            return;
        };
        self.finish_popup();
        let col = self
            .schema
            .relation(&hit.table)
            .and_then(|meta| meta.columns.iter().position(|c| c.name == hit.column));
        match hit.rowid {
            Some(rowid) => self.open_table_at(hit.table, rowid, col),
            None => {
                self.open_table(hit.table);
                self.toast.push(
                    "This row has no rowid to jump to; use Find (Ctrl-F)",
                    ToastKind::Info,
                );
            }
        }
    }

    // ── go to row ────────────────────────────────────────────────────────

    pub(super) fn open_goto(&mut self) {
        let Some(total) = self.grid.as_ref().map(|grid| grid.window.total_rows) else {
            return;
        };
        if total <= 0 {
            self.toast
                .push("There are no rows to go to", ToastKind::Info);
            return;
        }
        self.open_popup(PopupKind::GoToRow(GoToRowState::new(total)));
    }

    pub(super) fn commit_goto(&mut self) {
        let target = match &self.popup {
            Some(PopupKind::GoToRow(state)) => state.target().ok(),
            _ => None,
        };
        self.finish_popup();
        if let Some(row) = target {
            self.scroll_grid_to_row(row);
        }
    }

    // ── record view ──────────────────────────────────────────────────────

    pub(super) fn open_record(&mut self) {
        let Some(grid) = self.grid.as_ref() else {
            return;
        };
        let Some(values) = grid.window.get_row(grid.focused_row as i64).cloned() else {
            self.toast.push("The row is still loading", ToastKind::Info);
            return;
        };
        let state = RecordState::new(RecordInit {
            table: grid.table_name.clone(),
            row_number: grid.focused_row as i64 + 1,
            names: grid.columns.iter().map(|c| c.name.clone()).collect(),
            kinds: grid.kinds.clone(),
            values,
            links: grid.fk_cols.clone(),
            editable: !self.is_readonly_view(),
            focused: grid.focused_col,
        });
        self.open_popup(PopupKind::Record(state));
    }

    /// Enter on a record field: edit that cell in the grid's editor.
    pub(super) fn edit_record_field(&mut self) {
        let Some(PopupKind::Record(state)) = &self.popup else {
            return;
        };
        let field = state.selected();
        self.finish_popup();
        if let Some(grid) = self.grid.as_mut() {
            grid.focused_col = field;
        }
        self.update(Message::OpenPopup);
    }

    /// `j` on a link follows it; `o` on a JSON value opens the JSON viewer.
    pub(super) fn follow_record_field(&mut self) {
        let Some(PopupKind::Record(state)) = &self.popup else {
            return;
        };
        let field = state.selected();
        let is_link = state.links.get(field).copied().unwrap_or(false);
        if !is_link {
            let json = match state.selected_value() {
                Some(SqlValue::Text(text)) => serde_json::from_str(text).ok(),
                _ => None,
            };
            if let Some(json) = json {
                let title = format!("{}.{}", state.table, state.names[field]);
                self.push_popup(PopupKind::Json(JsonViewState::new(title, json)));
            }
            return;
        }
        self.finish_popup();
        if let Some(grid) = self.grid.as_mut() {
            grid.focused_col = field;
        }
        self.update(Message::JumpToFk);
    }

    // ── schema view ──────────────────────────────────────────────────────

    /// Loads and shows the definition of a table, view or index.
    pub(super) fn open_schema(&mut self, name: String) {
        let is_index = self.schema.indexes.contains(&name);
        let job_name = name.clone();
        self.spawn_db(
            move |conn| {
                let ddl = db::load_ddl(conn, &job_name)?.unwrap_or_default();
                let indexes = if is_index {
                    Vec::new()
                } else {
                    db::load_indexes(conn, &job_name)?
                };
                let index_table = if is_index {
                    db::index_table(conn, &job_name)?
                } else {
                    None
                };
                Ok((ddl, indexes, index_table))
            },
            move |result| match result {
                Ok((ddl, indexes, index_table)) => {
                    let mut lines = Vec::new();
                    if let Some(table) = index_table {
                        lines.push(SchemaLine::Heading("Table".into()));
                        lines.push(SchemaLine::Text(table));
                    }
                    if !indexes.is_empty() {
                        lines.push(SchemaLine::Heading("Indexes".into()));
                        lines.extend(indexes.into_iter().map(|index| {
                            let unique = if index.unique { "UNIQUE " } else { "" };
                            SchemaLine::Text(format!(
                                "{unique}{} ({})",
                                index.name,
                                index.columns.join(", ")
                            ))
                        }));
                    }
                    Message::SchemaViewReady { name, ddl, lines }
                }
                Err(error) => Message::Notify(
                    format!("Could not load the schema: {error}"),
                    ToastKind::Error,
                ),
            },
        );
    }

    /// Adds the columns and keys known from the loaded schema and shows the view.
    pub(super) fn show_schema_view(&mut self, name: String, ddl: String, extra: Vec<SchemaLine>) {
        let mut lines = Vec::new();
        if let Some(meta) = self.schema.relation(&name) {
            lines.push(SchemaLine::Heading(
                if meta.is_view {
                    "View columns"
                } else {
                    "Columns"
                }
                .into(),
            ));
            for column in &meta.columns {
                let mut text = format!(
                    "{}  {}",
                    column.name,
                    if column.col_type.is_empty() {
                        "(untyped)"
                    } else {
                        &column.col_type
                    }
                );
                if column.is_pk {
                    text.push_str("  PRIMARY KEY");
                }
                if column.not_null {
                    text.push_str("  NOT NULL");
                }
                if let Some(default) = &column.default_value {
                    text.push_str(&format!("  DEFAULT {default}"));
                }
                if let Some(fk) = meta.foreign_key(&column.name) {
                    text.push_str(&format!(
                        "  {} {}.{}",
                        self.symbols.foreign_key_arrow, fk.to_table, fk.to_col
                    ));
                }
                lines.push(SchemaLine::Text(text));
            }
        }
        lines.extend(extra);
        let referencing: Vec<String> = self
            .schema
            .references_to(&name)
            .map(|(source, fk)| {
                format!(
                    "{}.{} {} {}",
                    source.name, fk.from_col, self.symbols.foreign_key_arrow, fk.to_col
                )
            })
            .collect();
        if !referencing.is_empty() {
            lines.push(SchemaLine::Heading("Referenced by".into()));
            lines.extend(referencing.into_iter().map(SchemaLine::Text));
        }
        if !ddl.is_empty() {
            lines.push(SchemaLine::Heading("Definition".into()));
            lines.extend(ddl.lines().map(|line| SchemaLine::Text(line.to_string())));
        }
        self.push_popup(PopupKind::Schema(SchemaViewState::new(name, ddl, lines)));
    }

    // ── SQL console ──────────────────────────────────────────────────────

    pub(super) fn open_sql_console(&mut self) {
        let history = crate::app_dirs::history_file()
            .and_then(|path| std::fs::read_to_string(path).ok())
            .map(|text| decode_history(&text))
            .unwrap_or_default();
        self.open_popup(PopupKind::SqlConsole(SqlConsoleState::new(history)));
    }

    pub(super) fn run_sql_statement(&mut self) {
        let Some(PopupKind::SqlConsole(state)) = &mut self.popup else {
            return;
        };
        let statement = state.take_statement();
        let history = encode_history(&state.history);
        if statement.is_empty() {
            state.set_error("Type a statement to run".into());
            return;
        }
        if let Some(path) = crate::app_dirs::history_file() {
            let saved = path
                .parent()
                .map_or(Ok(()), std::fs::create_dir_all)
                .and_then(|()| std::fs::write(&path, history));
            if let Err(error) = saved {
                self.toast.push(
                    format!("Could not save the SQL history: {error}"),
                    ToastKind::Error,
                );
            }
        }
        let allow_writes = !self.readonly;
        let request_id = self.next_popup_request();
        self.spawn_db(
            move |conn| db::run_sql(conn, &statement, SQL_ROW_LIMIT, allow_writes),
            move |result| Message::SqlDone { request_id, result },
        );
    }

    pub(super) fn on_sql_done(&mut self, result: Result<SqlOutcome, String>) {
        let symbols = &self.symbols;
        let Some(PopupKind::SqlConsole(state)) = &mut self.popup else {
            return;
        };
        match result {
            Ok(SqlOutcome::Rows {
                columns,
                rows,
                truncated,
            }) => state.set_rows(columns, rows, truncated, symbols),
            Ok(SqlOutcome::Changed(changed)) => {
                state.set_changed(changed);
                // The statement may have changed rows or the schema.
                let _ = self.tx.send(Message::FileChanged);
            }
            Err(error) => state.set_error(error),
        }
    }

    // ── referencing rows ─────────────────────────────────────────────────

    pub(super) fn open_references(&mut self) {
        let Some(grid) = self.grid.as_ref() else {
            return;
        };
        let Some(row) = grid.window.get_row(grid.focused_row as i64) else {
            self.toast.push("The row is still loading", ToastKind::Info);
            return;
        };
        let references: Vec<Reference> = self
            .schema
            .references_to(&grid.table_name)
            .filter_map(|(source, fk)| {
                let key = grid
                    .columns
                    .iter()
                    .position(|c| c.name == fk.to_col)
                    .or_else(|| grid.columns.iter().position(|c| c.is_pk))?;
                Some(Reference {
                    table: source.name.clone(),
                    column: fk.from_col.clone(),
                    value: row.get(key)?.clone(),
                })
            })
            .collect();
        let table = grid.table_name.clone();
        self.open_popup(PopupKind::References(ReferencesState::new(
            table, references,
        )));
    }

    /// Opens the referencing table filtered to the rows that point at the
    /// focused row; Backspace comes back.
    pub(super) fn commit_reference(&mut self) {
        let Some(PopupKind::References(state)) = &self.popup else {
            return;
        };
        let Some(reference) = state.selected().cloned() else {
            return;
        };
        self.finish_popup();
        if let Some(grid) = self.grid.as_ref() {
            if let Some(rowid) = grid.window.get_rowid(grid.focused_row as i64) {
                self.jump_stack.push(JumpFrame {
                    table: grid.table_name.clone(),
                    rowid,
                    col: grid.focused_col,
                });
            }
        }
        self.open_table(reference.table.clone());
        if let Some(grid) = self.grid_for(&reference.table) {
            grid.filter.columns.insert(
                reference.column.clone(),
                ColumnFilter {
                    rules: vec![FilterRule::new(Condition::Eq(reference.value.clone()))],
                },
            );
            grid.reset_to_top();
            self.save_view_settings();
            self.fetch_window_around_focus();
            self.toast.push(
                format!(
                    "Filtered {} by {}; F clears it",
                    reference.table, reference.column
                ),
                ToastKind::Info,
            );
        }
    }

    // ── export & clipboard ───────────────────────────────────────────────

    pub(super) fn open_export(&mut self, format: ExportFormat) {
        let Some(grid) = self.grid.as_ref() else {
            return;
        };
        let safe_table: String = grid
            .table_name
            .chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                    c
                } else {
                    '_'
                }
            })
            .collect();
        let stamp = chrono::Local::now().format("%Y%m%d-%H%M%S");
        let path = format!("~/sqview_{safe_table}_{stamp}.{}", format.extension());
        let selected = grid.selected_row_count();
        self.open_popup(PopupKind::Export(ExportState::new(format, path, selected)));
    }

    pub(super) fn run_export(&mut self) {
        let Some(PopupKind::Export(state)) = &self.popup else {
            return;
        };
        let (format, raw_path, only_selection) = (
            state.format,
            state.path.value().trim().to_string(),
            state.only_selection,
        );
        self.finish_popup();
        let Some(grid) = self.grid.as_ref() else {
            return;
        };
        let view = match grid.view_query() {
            Ok(view) => view,
            Err(error) => {
                self.toast
                    .push(format!("Export failed: {error}"), ToastKind::Error);
                return;
            }
        };
        let scope = if only_selection {
            selection_scope(grid)
        } else {
            None
        };
        let columns = grid.columns.clone();
        let path = expand_home(&raw_path);
        let shown = path.display().to_string();
        self.toast
            .push(format!("Exporting to {shown}"), ToastKind::Info);
        self.spawn_db(
            move |conn| match scope {
                Some(scope) => {
                    let rows = rows_in_scope(conn, &view, &columns, scope)?;
                    export::export(
                        conn,
                        format,
                        &view.table,
                        &columns,
                        ExportRows::Given(&rows),
                        &path,
                    )
                }
                None => export::export(
                    conn,
                    format,
                    &view.table,
                    &columns,
                    ExportRows::View(&view),
                    &path,
                ),
            },
            move |result| match result {
                Ok(count) => Message::ExportDone { path: shown, count },
                Err(error) => Message::ExportFailed(error),
            },
        );
    }

    pub(super) fn copy_cell(&mut self) {
        let text = self.grid.as_ref().and_then(|g| {
            g.window
                .get_row(g.focused_row as i64)?
                .get(g.focused_col)
                .map(|value| value.to_text().into_owned())
        });
        match text {
            Some(text) => self.copy_to_clipboard(&text, "Copied the cell"),
            None => self.toast.push("The row is still loading", ToastKind::Info),
        }
    }

    /// Copies the selected rows, or the focused row, as JSON, CSV or SQL.
    pub(super) fn copy_rows(&mut self, format: CopyFormat) {
        let Some(grid) = self.grid.as_ref() else {
            return;
        };
        let view = match grid.view_query() {
            Ok(view) => view,
            Err(error) => {
                self.toast.push(error.to_string(), ToastKind::Error);
                return;
            }
        };
        let scope = selection_scope(grid).unwrap_or(RowScope::Focused(grid.focused_row as i64));
        let many = !matches!(scope, RowScope::Focused(_));
        let columns = grid.columns.clone();
        self.spawn_db(
            move |conn| {
                let rows = rows_in_scope(conn, &view, &columns, scope)?;
                if rows.is_empty() {
                    anyhow::bail!("No rows to copy");
                }
                let text = match format {
                    CopyFormat::Json if !many => {
                        serde_json::to_string(&export::row_to_json(&columns, &rows[0]))?
                    }
                    CopyFormat::Json => {
                        export::rows_to_text(ExportFormat::Json, &view.table, &columns, &rows)?
                            .trim_end()
                            .to_string()
                    }
                    CopyFormat::Csv => {
                        export::rows_to_text(ExportFormat::Csv, &view.table, &columns, &rows)?
                    }
                    CopyFormat::Sql => {
                        export::rows_to_text(ExportFormat::Sql, &view.table, &columns, &rows)?
                    }
                };
                let noun = if rows.len() == 1 { "row" } else { "rows" };
                let label = match format {
                    CopyFormat::Json => "JSON",
                    CopyFormat::Csv => "CSV",
                    CopyFormat::Sql => "SQL",
                };
                Ok((text, format!("Copied {} {noun} as {label}", rows.len())))
            },
            |result| match result {
                Ok((text, message)) => Message::CopyReady { text, message },
                Err(error) => Message::Notify(error, ToastKind::Error),
            },
        );
    }

    /// Copies the focused column's values of the selected rows, or of the
    /// whole view, one per line.
    pub(super) fn copy_column(&mut self) {
        let Some(grid) = self.grid.as_ref() else {
            return;
        };
        let view = match grid.view_query() {
            Ok(view) => view,
            Err(error) => {
                self.toast.push(error.to_string(), ToastKind::Error);
                return;
            }
        };
        let Some(column) = grid.columns.get(grid.focused_col).cloned() else {
            return;
        };
        let scope = selection_scope(grid).unwrap_or(RowScope::AllExcept(Vec::new()));
        self.spawn_db(
            move |conn| {
                let columns = [column];
                let rows = rows_in_scope(conn, &view, &columns, scope)?;
                let text = rows
                    .iter()
                    .map(|row| {
                        row.first()
                            .map_or_else(String::new, |value| value.to_text().into_owned())
                    })
                    .collect::<Vec<_>>()
                    .join("\n");
                Ok((
                    text,
                    format!("Copied {} values of {}", rows.len(), columns[0].name),
                ))
            },
            |result| match result {
                Ok((text, message)) => Message::CopyReady { text, message },
                Err(error) => Message::Notify(error, ToastKind::Error),
            },
        );
    }

    /// Copies what the open popup offers: a record value, a JSON node or DDL.
    pub(super) fn copy_from_popup(&mut self) {
        let text = match &self.popup {
            Some(PopupKind::Record(state)) => {
                state.selected_value().map(|v| v.to_text().into_owned())
            }
            Some(PopupKind::Json(state)) => state.selected_text(),
            Some(PopupKind::Schema(state)) => Some(state.ddl.clone()),
            _ => None,
        };
        if let Some(text) = text {
            self.copy_to_clipboard(&text, "Copied to clipboard");
        }
    }
}

/// The selection as a scope, or `None` without a selection.
fn selection_scope(grid: &crate::grid::GridState) -> Option<RowScope> {
    match &grid.row_selection {
        RowSelection::None => None,
        RowSelection::All { except } => Some(RowScope::AllExcept(
            except.iter().map(|&r| r as i64).collect(),
        )),
        RowSelection::Rows(_) => {
            let offsets: Vec<i64> = grid.selected_rows().into_iter().map(|r| r as i64).collect();
            (!offsets.is_empty()).then_some(RowScope::Offsets(offsets))
        }
    }
}

fn rows_in_scope(
    conn: &rusqlite::Connection,
    view: &ViewQuery,
    columns: &[Column],
    scope: RowScope,
) -> anyhow::Result<Vec<Vec<SqlValue>>> {
    Ok(match scope {
        RowScope::Focused(offset) => db::fetch_rows(conn, view, columns, offset, 1)?.rows,
        RowScope::Offsets(offsets) => db::fetch_rows_at_offsets(conn, view, columns, &offsets)?,
        RowScope::AllExcept(except) => {
            let mut rows = Vec::new();
            let mut offset = 0;
            db::visit_rows(conn, view, columns, |row| {
                if !except.contains(&offset) {
                    rows.push(row);
                }
                offset += 1;
                Ok(())
            })?;
            rows
        }
    })
}

/// Searches every table and view for `needle`, reporting the first matching
/// column of each matching row.
fn global_search(
    conn: &rusqlite::Connection,
    tables: &[(String, Vec<Column>)],
    needle: &str,
) -> anyhow::Result<Vec<GlobalHit>> {
    let lowered = needle.to_lowercase();
    let mut hits = Vec::new();
    for (table, columns) in tables {
        let searchable: Vec<Column> = columns
            .iter()
            .filter(|c| ColumnKind::of(&c.col_type, &c.name) != ColumnKind::Blob)
            .cloned()
            .collect();
        let found = db::search_view(
            conn,
            &ViewQuery::table(table),
            &searchable,
            needle,
            GLOBAL_SEARCH_PER_TABLE,
        )?;
        for hit in found {
            let Some(index) = hit
                .values
                .iter()
                .position(|value| value.to_text().to_lowercase().contains(&lowered))
            else {
                continue;
            };
            hits.push(GlobalHit {
                table: table.clone(),
                column: searchable[index].name.clone(),
                rowid: hit.rowid,
                value: hit.values[index].clone(),
            });
            if hits.len() >= GLOBAL_SEARCH_LIMIT {
                return Ok(hits);
            }
        }
    }
    Ok(hits)
}

fn expand_home(path: &str) -> PathBuf {
    match path.strip_prefix("~/") {
        Some(rest) => std::env::var_os("HOME").map_or_else(
            || PathBuf::from(path),
            |home| PathBuf::from(home).join(rest),
        ),
        None => PathBuf::from(path),
    }
}

/// History entries are stored one per line with newlines escaped.
fn encode_history(history: &[String]) -> String {
    history
        .iter()
        .map(|entry| entry.replace('\\', "\\\\").replace('\n', "\\n"))
        .collect::<Vec<_>>()
        .join("\n")
}

fn decode_history(text: &str) -> Vec<String> {
    text.lines()
        .filter(|line| !line.is_empty())
        .map(|line| {
            let mut out = String::new();
            let mut chars = line.chars();
            while let Some(ch) = chars.next() {
                match (ch, chars.clone().next()) {
                    ('\\', Some('n')) => {
                        out.push('\n');
                        chars.next();
                    }
                    ('\\', Some('\\')) => {
                        out.push('\\');
                        chars.next();
                    }
                    _ => out.push(ch),
                }
            }
            out
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn history_round_trips_multiline_statements() {
        let history = vec!["SELECT 1".to_string(), "SELECT\n  'a\\b'".to_string()];
        assert_eq!(decode_history(&encode_history(&history)), history);
    }

    #[test]
    fn home_is_expanded_in_export_paths() {
        let expanded = expand_home("~/x.csv");
        assert!(expanded.ends_with("x.csv"));
        assert!(!expanded.starts_with("~"));
        assert_eq!(expand_home("/tmp/x.csv"), PathBuf::from("/tmp/x.csv"));
    }
}
