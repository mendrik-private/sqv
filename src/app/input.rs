//! Translation of terminal key and mouse events into application messages and
//! direct UI state changes. The bindings are exactly those in
//! [`crate::keymap`], which generates the help popup and README.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use ratatui::layout::{Position, Rect};

use super::{App, FocusPane, GridScrollbarDrag, Message};
use crate::{
    grid::{GridHit, GridState},
    ui::{
        popup::{command_palette::CopyFormat, PopupAction, PopupKind},
        sidebar::SidebarAction,
        tabbar::TabMouseAction,
        toast::ToastKind,
    },
};

/// Popups that take typed text, where `?` is a character rather than help.
fn takes_text(popup: &PopupKind) -> bool {
    matches!(
        popup,
        PopupKind::TextEditor(_)
            | PopupKind::ValuePicker(_)
            | PopupKind::InsertRow(_)
            | PopupKind::FkPicker(_)
            | PopupKind::FilterPopup(_)
            | PopupKind::CommandPalette(_)
            | PopupKind::Find(_)
            | PopupKind::GoToRow(_)
            | PopupKind::SqlConsole(_)
            | PopupKind::GlobalSearch(_)
            | PopupKind::Export(_)
    )
}

impl App {
    /// Queues a message for the update loop.
    fn send(&self, message: Message) {
        let _ = self.tx.send(message);
    }

    pub(super) fn handle_key(&mut self, key: KeyEvent) {
        if self.pending_confirm.is_some() {
            match key.code {
                KeyCode::Char('y' | 'Y') => self.send(Message::ConfirmDelete),
                KeyCode::Char('n' | 'N') | KeyCode::Esc => self.send(Message::CancelConfirm),
                _ => {}
            }
            return;
        }
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        if ctrl && matches!(key.code, KeyCode::Char('q' | 'Q')) {
            self.should_quit = true;
            return;
        }
        if key.code == KeyCode::Char('?') && !self.popup.as_ref().is_some_and(takes_text) {
            self.send(Message::OpenHelp);
            return;
        }
        if self.popup.is_some() {
            self.handle_popup_key(key);
            return;
        }
        let letter_jump = std::mem::take(&mut self.letter_jump_armed);

        match (key.code, key.modifiers) {
            (KeyCode::Char('p' | 'P'), m) if m.contains(KeyModifiers::CONTROL) => {
                self.send(Message::OpenCommandPalette);
            }
            (KeyCode::Char('w'), KeyModifiers::CONTROL) => {
                if let Some(index) = self.active_tab {
                    self.close_tab(index);
                }
            }
            (KeyCode::Char('z'), KeyModifiers::CONTROL) => self.send(Message::UndoAction),
            (KeyCode::Char('b'), KeyModifiers::CONTROL) => self.toggle_sidebar(),
            (KeyCode::PageDown, KeyModifiers::CONTROL) => self.cycle_tab(true),
            (KeyCode::PageUp, KeyModifiers::CONTROL) => self.cycle_tab(false),
            (KeyCode::Char(c @ '1'..='9'), KeyModifiers::NONE) if !self.open_tabs.is_empty() => {
                self.activate_tab(c as usize - '1' as usize);
            }
            (KeyCode::Char('0'), KeyModifiers::NONE) if !self.open_tabs.is_empty() => {
                self.activate_tab(9);
            }
            (KeyCode::Char(']'), KeyModifiers::NONE) => self.cycle_tab(true),
            (KeyCode::Char('['), KeyModifiers::NONE) => self.cycle_tab(false),
            (KeyCode::Tab, KeyModifiers::NONE) | (KeyCode::BackTab, _) => {
                self.focus = match self.focus {
                    FocusPane::Grid if self.sidebar_visible => FocusPane::Sidebar,
                    _ => FocusPane::Grid,
                };
            }
            _ => match self.focus {
                FocusPane::Sidebar => self.handle_sidebar_key(key),
                FocusPane::Grid => self.handle_grid_key(key, letter_jump),
            },
        }
    }

    fn handle_popup_key(&mut self, key: KeyEvent) {
        let action = match self.popup.as_mut() {
            None => return,
            Some(popup) => match popup {
                PopupKind::TextEditor(state) => state.handle_key(&key),
                PopupKind::ValuePicker(state) => state.handle_key(&key),
                PopupKind::DatePicker(state) => state.handle_key(&key),
                PopupKind::InsertRow(state) => state.handle_key(&key),
                PopupKind::FkPicker(state) => state.handle_key(&key),
                PopupKind::FilterPopup(state) => state.handle_key(&key),
                PopupKind::CommandPalette(state) => state.handle_key(&key),
                PopupKind::Help(state) => state.handle_key(&key),
                PopupKind::Find(state) => state.handle_key(&key),
                PopupKind::GoToRow(state) => state.handle_key(&key),
                PopupKind::Record(state) => state.handle_key(&key),
                PopupKind::Schema(state) => state.handle_key(&key),
                PopupKind::SqlConsole(state) => state.handle_key(&key),
                PopupKind::GlobalSearch(state) => state.handle_key(&key),
                PopupKind::Export(state) => state.handle_key(&key),
                PopupKind::References(state) => state.handle_key(&key),
                PopupKind::Json(state) => state.handle_key(&key),
            },
        };
        self.apply_popup_action(action);
    }

    /// Carries out what a popup made of an input event.
    fn apply_popup_action(&mut self, action: PopupAction) {
        match action {
            PopupAction::Ignored => {}
            PopupAction::Handled => {
                if let Some(PopupKind::InsertRow(state)) = &self.popup {
                    let col = state.selected;
                    self.sync_inline_insert_column(col);
                }
            }
            PopupAction::Close => self.send(Message::ClosePopup),
            PopupAction::QueryChanged => self.run_popup_search(),
            PopupAction::Copy => self.copy_from_popup(),
            PopupAction::Follow => self.follow_record_field(),
            PopupAction::Submit => self.submit_popup(),
        }
        self.dirty = true;
    }

    fn submit_popup(&mut self) {
        match &self.popup {
            Some(
                PopupKind::TextEditor(_)
                | PopupKind::ValuePicker(_)
                | PopupKind::DatePicker(_)
                | PopupKind::FkPicker(_),
            ) => self.send(Message::CommitEdit),
            Some(PopupKind::InsertRow(_)) => self.send(Message::CommitInsertRow),
            Some(PopupKind::FilterPopup(state)) => {
                let (col_name, col_filter) = (state.col_name.clone(), state.col_filter.clone());
                self.apply_column_filter(col_name, col_filter);
            }
            Some(PopupKind::CommandPalette(state)) => {
                if let Some(command) = state.selected_command() {
                    self.finish_popup();
                    self.execute_palette_command(command);
                }
            }
            Some(PopupKind::Find(_)) => self.commit_find(),
            Some(PopupKind::GoToRow(_)) => self.commit_goto(),
            Some(PopupKind::Record(_)) => self.edit_record_field(),
            Some(PopupKind::SqlConsole(_)) => self.run_sql_statement(),
            Some(PopupKind::GlobalSearch(_)) => self.commit_global_search(),
            Some(PopupKind::Export(_)) => self.run_export(),
            Some(PopupKind::References(_)) => self.commit_reference(),
            Some(PopupKind::Help(_) | PopupKind::Schema(_) | PopupKind::Json(_)) | None => {}
        }
    }

    fn handle_grid_key(&mut self, key: KeyEvent, letter_jump: bool) {
        let Some(grid) = self.grid.as_ref() else {
            if key.code == KeyCode::Esc && self.sidebar_visible {
                self.focus = FocusPane::Sidebar;
            }
            return;
        };
        if letter_jump {
            if let KeyCode::Char(c) = key.code {
                if c == '#' || c.is_alphabetic() {
                    self.send(Message::JumpToLetter(c));
                    return;
                }
            }
        }
        let page = grid.window.viewport_rows.saturating_sub(1).max(1);
        let on_link = grid.fk_cols.get(grid.focused_col).copied().unwrap_or(false);
        let focused_col = grid.focused_col;
        match (key.code, key.modifiers) {
            (KeyCode::Char('a'), KeyModifiers::CONTROL) => {
                if let Some(grid) = self.grid.as_mut() {
                    grid.select_all_rows();
                    if grid.has_row_selection() {
                        let message = if grid.filter.is_empty() {
                            "All rows selected; d deletes the whole table"
                        } else {
                            "All filtered rows selected"
                        };
                        self.toast.push(message, ToastKind::Info);
                    }
                }
            }
            (KeyCode::Char('f'), KeyModifiers::CONTROL) => self.send(Message::OpenFind),
            (KeyCode::Char('g'), KeyModifiers::CONTROL) => self.open_goto(),
            (KeyCode::Char('c'), KeyModifiers::CONTROL)
            | (KeyCode::Char('y'), KeyModifiers::NONE) => {
                self.send(Message::CopyCell);
            }
            (KeyCode::Char('Y'), _) => self.send(Message::CopyRows(CopyFormat::Json)),
            (KeyCode::Down, KeyModifiers::SHIFT) => {
                self.update_grid(|grid| grid.extend_row_selection_down(1));
            }
            (KeyCode::Up, KeyModifiers::SHIFT) => {
                self.update_grid(|grid| grid.extend_row_selection_up(1))
            }
            (KeyCode::Down, KeyModifiers::CONTROL) | (KeyCode::PageDown, _) => {
                self.send(Message::ScrollDown(page));
            }
            (KeyCode::Up, KeyModifiers::CONTROL) | (KeyCode::PageUp, _) => {
                self.send(Message::ScrollUp(page))
            }
            (KeyCode::Char('j'), KeyModifiers::NONE) if on_link => self.send(Message::JumpToFk),
            (KeyCode::Down, _) | (KeyCode::Char('j'), KeyModifiers::NONE) => {
                self.send(Message::MoveDown)
            }
            (KeyCode::Up, _) | (KeyCode::Char('k'), KeyModifiers::NONE) => {
                self.send(Message::MoveUp)
            }
            (KeyCode::Right, _) | (KeyCode::Char('l'), KeyModifiers::NONE) => {
                self.send(Message::MoveRight)
            }
            (KeyCode::Left, _) | (KeyCode::Char('h'), KeyModifiers::NONE) => {
                self.send(Message::MoveLeft)
            }
            (KeyCode::Home, KeyModifiers::CONTROL) => self.send(Message::MoveFirstCell),
            (KeyCode::End, KeyModifiers::CONTROL) => self.send(Message::MoveLastCell),
            (KeyCode::Home, _) => self.send(Message::MoveColFirst),
            (KeyCode::End, _) => self.send(Message::MoveColLast),
            (KeyCode::Char(' '), KeyModifiers::NONE) => self.update_grid(|grid| {
                let row = grid.focused_row;
                grid.toggle_row_selected(row);
            }),
            (KeyCode::Enter, _) => self.send(Message::OpenPopup),
            (KeyCode::Char('e'), KeyModifiers::NONE) => self.send(Message::OpenDirectEdit),
            (KeyCode::Char('n'), KeyModifiers::NONE) => {
                if self.focused_cell_can_be_set_null() {
                    self.send(Message::SetFocusedCellNull);
                } else if !self.is_readonly_view() {
                    self.toast
                        .push("This cell cannot be set to NULL", ToastKind::Info);
                } else {
                    self.ensure_writable();
                }
            }
            (KeyCode::Esc, _) => {
                if self.grid.as_ref().is_some_and(GridState::has_row_selection) {
                    if let Some(grid) = self.grid.as_mut() {
                        grid.clear_row_selection();
                    }
                } else if self.sidebar_visible {
                    self.focus = FocusPane::Sidebar;
                }
            }
            (KeyCode::Backspace, _) => {
                if self.jump_stack.is_empty() {
                    self.toast.push("Nothing to go back to", ToastKind::Info);
                } else {
                    self.send(Message::JumpBack);
                }
            }
            (KeyCode::Char('v'), KeyModifiers::NONE) => self.open_record(),
            (KeyCode::Char('r'), KeyModifiers::NONE) => self.open_references(),
            (KeyCode::Char(':'), _) => self.open_sql_console(),
            (KeyCode::Char('s'), KeyModifiers::NONE) => self.send(Message::CycleSort),
            (KeyCode::Char('S'), _) => self.send(Message::AddSortKey),
            (KeyCode::Char('f'), KeyModifiers::NONE) => self.send(Message::OpenFilterPopup),
            (KeyCode::Char('F'), _) => self.send(Message::ClearFilters),
            (KeyCode::Char('<'), _) => {
                self.update_grid(|grid| grid.adjust_column_width(focused_col, -2));
                self.save_view_settings();
            }
            (KeyCode::Char('>'), _) => {
                self.update_grid(|grid| grid.adjust_column_width(focused_col, 2));
                self.save_view_settings();
            }
            (KeyCode::Char('-'), KeyModifiers::NONE) => {
                let hidden = self
                    .grid
                    .as_mut()
                    .is_some_and(|grid| grid.hide_column(focused_col));
                if hidden {
                    self.save_view_settings();
                    self.toast.push(
                        "Column hidden; Ctrl-P \"Show hidden columns\" brings it back",
                        ToastKind::Info,
                    );
                } else {
                    self.toast
                        .push("The last visible column cannot be hidden", ToastKind::Info);
                }
            }
            (KeyCode::Insert | KeyCode::Char('i'), KeyModifiers::NONE) => {
                self.send(Message::InsertRow)
            }
            (KeyCode::Delete | KeyCode::Char('d'), KeyModifiers::NONE) => {
                self.send(Message::DeleteRow)
            }
            (KeyCode::Char('\''), _) => {
                if self.grid.as_ref().is_some_and(GridState::is_text_sorted) {
                    self.letter_jump_armed = true;
                } else {
                    self.toast
                        .push("Sort a text column (s) to jump by letter", ToastKind::Info);
                }
            }
            _ => {}
        }
    }

    fn handle_sidebar_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Up | KeyCode::Char('k') => self.sidebar.move_up(&self.schema),
            KeyCode::Down | KeyCode::Char('j') => self.sidebar.move_down(&self.schema),
            KeyCode::Left | KeyCode::Char('h') => {
                self.sidebar.collapse_selected_section(&self.schema);
            }
            KeyCode::Right | KeyCode::Char('l') => {
                self.sidebar.expand_selected_section(&self.schema);
            }
            KeyCode::Enter => {
                let action = self.sidebar.enter(&self.schema);
                self.apply_sidebar_action(action);
            }
            KeyCode::Char('i') => match self.sidebar.selected_name(&self.schema) {
                Some(name) => self.open_schema(name),
                None => self
                    .toast
                    .push("Select a table, view or index", ToastKind::Info),
            },
            KeyCode::Esc if self.active_tab.is_some() => self.focus = FocusPane::Grid,
            _ => {}
        }
    }

    fn apply_sidebar_action(&mut self, action: Option<SidebarAction>) {
        match action {
            Some(SidebarAction::OpenTable(name)) => self.open_table(name),
            Some(SidebarAction::ShowSchema(name)) => self.open_schema(name),
            Some(SidebarAction::Toggle) | None => {}
        }
    }

    pub(super) fn handle_mouse(&mut self, mouse: MouseEvent) {
        // The deletion prompt is modal.
        if self.pending_confirm.is_some() {
            return;
        }
        let (x, y, kind, modifiers) = (mouse.column, mouse.row, mouse.kind, mouse.modifiers);
        if self.popup.is_some() {
            self.handle_popup_mouse(kind, modifiers, x, y);
            return;
        }
        let shift = modifiers.contains(KeyModifiers::SHIFT);
        match kind {
            MouseEventKind::ScrollLeft | MouseEventKind::ScrollRight => {
                self.scroll_grid_columns(x, y, kind == MouseEventKind::ScrollRight);
                return;
            }
            MouseEventKind::ScrollDown | MouseEventKind::ScrollUp if shift => {
                self.scroll_grid_columns(x, y, kind == MouseEventKind::ScrollDown);
                return;
            }
            MouseEventKind::ScrollDown | MouseEventKind::ScrollUp => {
                self.mouse_scroll_panel(x, y, 3, kind == MouseEventKind::ScrollDown);
                return;
            }
            MouseEventKind::Drag(MouseButton::Left) => {
                self.drag_grid_scrollbar(y);
                return;
            }
            MouseEventKind::Up(MouseButton::Left) => {
                self.grid_scrollbar_drag = None;
                return;
            }
            _ => {}
        }

        let middle_click = matches!(kind, MouseEventKind::Down(MouseButton::Middle));
        let left_click = matches!(kind, MouseEventKind::Down(MouseButton::Left));
        if !left_click && !middle_click {
            return;
        }

        if let Some(action) =
            crate::ui::tabbar::hit_test(self.tabbar_area, self, x, y, middle_click)
        {
            match action {
                TabMouseAction::Activate(idx) => self.activate_tab(idx),
                TabMouseAction::Close(idx) => self.close_tab(idx),
            }
            return;
        }

        let position = Position { x, y };
        if let Some(area) = self
            .sidebar_area
            .filter(|area| self.sidebar_visible && area.contains(position))
        {
            self.focus = FocusPane::Sidebar;
            if left_click {
                let action = self.sidebar.click_at(area, &self.schema, x, y);
                self.apply_sidebar_action(action);
            }
            return;
        }

        let Some(area) = self.grid_inner_area.filter(|area| area.contains(position)) else {
            return;
        };
        self.focus = FocusPane::Grid;
        if !left_click {
            return;
        }
        let ctrl = modifiers.contains(KeyModifiers::CONTROL);
        let Some(hit) = self
            .grid
            .as_ref()
            .and_then(|grid| crate::grid::hit_test(area, grid, x, y))
        else {
            return;
        };
        match hit {
            GridHit::Header(col) => {
                self.update_grid(|grid| grid.focus_cell(grid.focused_row, col));
                self.send(Message::CycleSort);
            }
            GridHit::RowGutter(row) => self.update_grid(|grid| {
                let focused_col = grid.focused_col;
                if ctrl {
                    grid.toggle_row_selected(row);
                } else {
                    grid.select_only_row(row);
                }
                grid.focus_cell_preserve_selection(row, focused_col);
            }),
            GridHit::Cell { row, col } => self.update_grid(|grid| grid.focus_cell(row, col)),
            GridHit::AlphabetRail(letter) => self.send(Message::JumpToLetter(letter)),
            GridHit::Scrollbar => self.begin_grid_scrollbar_drag(area, y),
        }
    }

    fn handle_popup_mouse(
        &mut self,
        kind: MouseEventKind,
        modifiers: KeyModifiers,
        x: u16,
        y: u16,
    ) {
        let action = match self.popup.as_mut() {
            None => return,
            Some(popup) => match popup {
                PopupKind::TextEditor(state) => state.handle_mouse(kind, x, y),
                PopupKind::ValuePicker(state) => state.handle_mouse(kind, x, y),
                PopupKind::CommandPalette(state) => state.handle_mouse(kind, x, y),
                PopupKind::FilterPopup(state) => state.handle_mouse(kind, x, y),
                PopupKind::Help(state) => state.handle_mouse(kind),
                PopupKind::Schema(state) => state.handle_mouse(kind),
                PopupKind::Find(state) => state.handle_mouse(kind, modifiers, x, y),
                PopupKind::FkPicker(state) => state.handle_mouse(kind, modifiers, x, y),
                PopupKind::GlobalSearch(state) => state.handle_mouse(kind, modifiers, x, y),
                PopupKind::SqlConsole(state) => state.handle_mouse(kind, modifiers, x, y),
                PopupKind::Record(state) => state.handle_mouse(kind, x, y),
                PopupKind::References(state) => state.handle_mouse(kind, x, y),
                PopupKind::Json(state) => state.handle_mouse(kind, x, y),
                // Inline inserts and the small forms have nothing to click.
                PopupKind::InsertRow(_)
                | PopupKind::DatePicker(_)
                | PopupKind::GoToRow(_)
                | PopupKind::Export(_) => PopupAction::Ignored,
            },
        };
        self.apply_popup_action(action);
    }

    fn scroll_grid_columns(&mut self, x: u16, y: u16, right: bool) {
        let position = Position { x, y };
        if self
            .grid_outer_area
            .is_some_and(|area| area.contains(position))
        {
            self.focus = FocusPane::Grid;
            self.update_grid(|grid| grid.scroll_columns(right));
        }
    }

    fn mouse_scroll_panel(&mut self, x: u16, y: u16, amount: usize, down: bool) {
        let position = Position { x, y };
        if let Some(area) = self
            .sidebar_area
            .filter(|area| self.sidebar_visible && area.contains(position))
        {
            self.focus = FocusPane::Sidebar;
            let viewport_rows = area.height.saturating_sub(2) as usize;
            if down {
                self.sidebar
                    .scroll_down(&self.schema, viewport_rows, amount);
            } else {
                self.sidebar.scroll_up(&self.schema, viewport_rows, amount);
            }
            return;
        }
        if self
            .grid_outer_area
            .is_some_and(|area| area.contains(position))
        {
            self.focus = FocusPane::Grid;
            if down {
                self.scroll_grid_down(amount);
            } else {
                self.scroll_grid_up(amount);
            }
        }
    }

    fn sync_inline_insert_column(&mut self, selected_col: usize) {
        if let Some(grid) = self.grid.as_mut() {
            let focused_row = grid.focused_row;
            grid.focus_cell(focused_row, selected_col);
        }
    }

    fn begin_grid_scrollbar_drag(&mut self, area: Rect, y: u16) {
        let Some((grab_offset, offset)) = self
            .grid
            .as_ref()
            .and_then(|grid| crate::grid::scrollbar_drag_start(area, grid, y))
        else {
            return;
        };
        self.grid_scrollbar_drag = Some(GridScrollbarDrag { grab_offset });
        self.update_grid(|grid| grid.scroll_viewport_to(offset));
    }

    fn drag_grid_scrollbar(&mut self, y: u16) {
        let (Some(area), Some(grab)) = (
            self.grid_inner_area,
            self.grid_scrollbar_drag
                .as_ref()
                .map(|drag| drag.grab_offset),
        ) else {
            return;
        };
        if let Some(offset) = self
            .grid
            .as_ref()
            .and_then(|grid| crate::grid::scrollbar_drag_offset(area, grid, y, grab))
        {
            self.update_grid(|grid| grid.scroll_viewport_to(offset));
        }
    }
}
