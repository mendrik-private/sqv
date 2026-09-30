//! Translation of terminal key and mouse events into application messages and
//! direct UI state changes.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use ratatui::layout::{Position, Rect};

use super::{App, AppMode, FocusPane, GridScrollbarDrag, Message};
use crate::{
    grid::{GridHit, GridState},
    ui::{
        popup::{
            filter::{FilterPopupFocus, FilterPopupHit},
            DateFocus, PopupKind,
        },
        sidebar::SidebarAction,
        tabbar::TabMouseAction,
        toast::ToastKind,
    },
};

impl App {
    pub(super) fn handle_key(&mut self, key: KeyEvent) {
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
            (KeyCode::Tab, KeyModifiers::NONE) | (KeyCode::BackTab, _) => {
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

    fn handle_edit_key(&mut self, key: KeyEvent) {
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
                        state.move_day(-1);
                    } else {
                        state.focus_prev();
                    }
                    self.dirty = true;
                }
                KeyCode::Right => {
                    if state.focus == DateFocus::Calendar {
                        state.move_day(1);
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
                                FilterPopupFocus::RuleList => {
                                    state.focus_value();
                                }
                                FilterPopupFocus::Operator => {
                                    state.focus_value();
                                }
                                FilterPopupFocus::Value => match state.add_rule() {
                                    Ok(()) => {
                                        apply = Some((
                                            state.col_name.clone(),
                                            state.col_filter.clone(),
                                        ));
                                    }
                                    Err(message) => {
                                        toast = Some(message);
                                    }
                                },
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
                        FilterPopupFocus::RuleList => {
                            state.select_prev_rule();
                        }
                        FilterPopupFocus::Operator => {
                            state.prev_op();
                        }
                        FilterPopupFocus::Value => {
                            state.prev_op();
                        }
                    }
                    self.dirty = true;
                }
                KeyCode::Down => {
                    match state.focus {
                        FilterPopupFocus::RuleList => {
                            state.select_next_rule();
                        }
                        FilterPopupFocus::Operator => {
                            state.next_op();
                        }
                        FilterPopupFocus::Value => {
                            state.next_op();
                        }
                    }
                    self.dirty = true;
                }
                KeyCode::Left => {
                    if state.focus == FilterPopupFocus::Value {
                        state.move_cursor_left();
                    }
                    self.dirty = true;
                }
                KeyCode::Right => {
                    if state.focus == FilterPopupFocus::Value {
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
                KeyCode::Char(' ') if state.focus == FilterPopupFocus::RuleList => {
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
                KeyCode::Char(' ') if state.focus == FilterPopupFocus::Operator => {
                    state.next_op();
                    self.dirty = true;
                }
                KeyCode::Delete | KeyCode::Backspace
                    if state.focus == FilterPopupFocus::RuleList =>
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
                    if state.focus == FilterPopupFocus::Value
                        && (key.modifiers == KeyModifiers::NONE
                            || key.modifiers == KeyModifiers::SHIFT) =>
                {
                    state.push_char(c);
                    self.dirty = true;
                }
                KeyCode::Backspace if state.focus == FilterPopupFocus::Value => {
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

    fn handle_grid_key(&mut self, key: KeyEvent) {
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
            (KeyCode::Insert | KeyCode::Char('i'), KeyModifiers::NONE) => {
                let _ = self.tx.send(Message::InsertRow);
            }
            (KeyCode::Delete | KeyCode::Char('d'), KeyModifiers::NONE) => {
                let _ = self.tx.send(Message::DeleteRow);
            }
            (KeyCode::Char('c'), KeyModifiers::CONTROL)
            | (KeyCode::Char('y'), KeyModifiers::NONE) => {
                let _ = self.tx.send(Message::CopyCell);
            }
            (KeyCode::Char('Y'), KeyModifiers::SHIFT) => {
                let _ = self.tx.send(Message::CopyRowJson);
            }
            // Letters bound above never reach this arm; `n` only when its guard failed.
            (KeyCode::Char(c), KeyModifiers::NONE)
                if (c == '#' || (c.is_alphabetic() && c != 'n'))
                    && self.grid.as_ref().is_some_and(GridState::is_text_sorted) =>
            {
                let _ = self.tx.send(Message::JumpToLetter(c));
            }
            _ => {}
        }
    }

    fn handle_sidebar_key(&mut self, key: KeyEvent) {
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

    pub(super) fn handle_mouse(&mut self, mouse: MouseEvent) {
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

        let position = Position { x, y };
        if let Some(area) = self.sidebar_area {
            if self.sidebar_visible && area.contains(position) {
                self.focus = FocusPane::Sidebar;
                if left_click {
                    if let Some(action) = self.sidebar.click_at(area, &self.schema, x, y) {
                        match action {
                            SidebarAction::OpenTable(name) => self.open_table(name),
                            SidebarAction::Toggle => {}
                        }
                    }
                }
                return;
            }
        }

        if let Some(area) = self.grid_inner_area {
            if area.contains(position) {
                self.focus = FocusPane::Grid;
                if !left_click {
                    return;
                }
                let mut cycle_sort = false;
                if let Some(grid) = self.grid.as_mut() {
                    if let Some(hit) = crate::grid::hit_test(area, grid, x, y) {
                        match hit {
                            GridHit::Header(col) => {
                                grid.focus_cell(grid.focused_row, col);
                                cycle_sort = true;
                            }
                            GridHit::RowGutter(row) => {
                                let focused_col = grid.focused_col;
                                if ctrl_click {
                                    grid.toggle_row_selected(row);
                                } else {
                                    grid.select_only_row(row);
                                }
                                grid.focus_cell_preserve_selection(row, focused_col);
                                self.dirty = true;
                            }
                            GridHit::Cell { row, col } => {
                                grid.focus_cell(row, col);
                            }
                            GridHit::AlphabetRail(letter) => {
                                let _ = self.tx.send(Message::JumpToLetter(letter));
                            }
                            GridHit::Scrollbar => {
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

    fn handle_text_editor_mouse(&mut self, mouse: MouseEvent) {
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

    fn handle_filter_popup_mouse(&mut self, mouse: MouseEvent) {
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
        let position = Position { x, y };
        if let Some(area) = self.sidebar_area {
            if self.sidebar_visible && area.contains(position) {
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
            if area.contains(position) {
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
}
