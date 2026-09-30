use crossterm::event::{KeyCode, KeyEvent, MouseButton, MouseEventKind};
use ratatui::{
    layout::{Position, Rect},
    style::{Modifier, Style},
    widgets::{Block, Borders},
    Frame,
};

use super::PopupAction;
use crate::{
    db::types::{parse_input, SqlValue},
    filter::{ColumnFilter, Condition, FilterOp, FilterRule},
    symbols::Symbols,
    theme::Theme,
    ui::widgets::{
        frame::PopupFrame,
        hints::{hint, render_hints, Hint},
        input::TextInput,
        list::{paint_row, ListCursor},
        scrollbar::Scrollbar,
        text::{put, truncate_with_ellipsis},
    },
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FilterPopupFocus {
    RuleList,
    Operator,
    Value,
}

/// Edits the rules of one column. Rules of a column are alternatives (OR);
/// different columns must all match (AND). The list ends with a "new rule" row.
pub struct FilterPopupState {
    pub col_name: String,
    pub col_type: String,
    pub col_filter: ColumnFilter,
    pub rules: ListCursor,
    pub draft_op: FilterOp,
    pub draft: TextInput,
    pub focus: FilterPopupFocus,
    /// Why the last attempt to save the draft failed.
    pub error: Option<String>,
    layout: Layout,
}

#[derive(Debug, Clone, Copy, Default)]
struct Layout {
    rule_rows: Rect,
    operator: Rect,
    value: Rect,
}

impl FilterPopupState {
    pub fn new(col_name: String, col_type: String, col_filter: ColumnFilter) -> Self {
        let has_rules = !col_filter.rules.is_empty();
        let draft_op = col_filter
            .rules
            .last()
            .map(|rule| rule.condition.op())
            .unwrap_or(FilterOp::Contains);
        let mut state = Self {
            col_name,
            col_type,
            col_filter,
            rules: ListCursor::default(),
            draft_op,
            draft: TextInput::default(),
            focus: if has_rules {
                FilterPopupFocus::RuleList
            } else {
                FilterPopupFocus::Value
            },
            error: None,
            layout: Layout::default(),
        };
        state.sync_editor_from_selection();
        state
    }

    /// Rules plus the trailing "new rule" row.
    fn row_count(&self) -> usize {
        self.col_filter.rules.len() + 1
    }

    pub fn is_new_rule_selected(&self) -> bool {
        self.rules.selected >= self.col_filter.rules.len()
    }

    fn shift_op(&mut self, steps: usize) {
        let ops = FilterOp::ALL;
        let idx = ops.iter().position(|op| *op == self.draft_op).unwrap_or(0);
        self.draft_op = ops[(idx + steps) % ops.len()];
    }

    pub fn next_op(&mut self) {
        self.shift_op(1);
    }

    pub fn prev_op(&mut self) {
        self.shift_op(FilterOp::ALL.len() - 1);
    }

    fn select_rule(&mut self, index: usize) {
        self.rules.select(index, self.row_count());
        self.sync_editor_from_selection();
    }

    pub fn delete_selected_rule(&mut self) -> bool {
        if self.is_new_rule_selected() {
            return false;
        }
        self.col_filter.rules.remove(self.rules.selected);
        let selected = self
            .rules
            .selected
            .min(self.col_filter.rules.len().saturating_sub(1));
        self.select_rule(selected);
        true
    }

    pub fn toggle_selected_rule_enabled(&mut self) -> bool {
        match self.col_filter.rules.get_mut(self.rules.selected) {
            Some(rule) => {
                rule.enabled = !rule.enabled;
                true
            }
            None => false,
        }
    }

    /// Saves the draft as a new rule or over the selected one.
    pub fn save_draft(&mut self) -> Result<(), String> {
        let rule = self.build_rule()?;
        match self.col_filter.rules.get_mut(self.rules.selected) {
            Some(existing) => {
                existing.condition = rule.condition;
            }
            None => {
                self.col_filter.rules.push(rule);
                let last = self.col_filter.rules.len() - 1;
                self.rules.select(last, self.row_count());
            }
        }
        self.sync_editor_from_selection();
        Ok(())
    }

    fn build_rule(&self) -> Result<FilterRule, String> {
        let needle = self.draft.value().trim();
        if needle.is_empty() {
            return Err("Type a value to filter by".to_string());
        }
        let literal =
            || parse_input(&self.col_type, needle).map_err(|error| format!("The value {error}"));
        Ok(FilterRule::new(match self.draft_op {
            FilterOp::Lt => Condition::Lt(literal()?),
            FilterOp::Gt => Condition::Gt(literal()?),
            FilterOp::Eq => Condition::Eq(literal()?),
            FilterOp::Contains => Condition::Contains(needle.to_string()),
            FilterOp::Regex => {
                regex::Regex::new(needle).map_err(|error| format!("Invalid pattern: {error}"))?;
                Condition::Regex(needle.to_string())
            }
        }))
    }

    fn sync_editor_from_selection(&mut self) {
        self.error = None;
        match self.col_filter.rules.get(self.rules.selected) {
            Some(rule) => {
                self.draft_op = rule.condition.op();
                self.draft.set(rule.condition.operand_text());
            }
            None => self.draft.clear(),
        }
    }

    fn cycle_focus(&mut self, forward: bool) {
        use FilterPopupFocus::*;
        self.focus = match (self.focus, forward) {
            (RuleList, true) | (Value, false) => Operator,
            (Operator, true) | (RuleList, false) => Value,
            (Value, true) | (Operator, false) => RuleList,
        };
    }

    /// `Submit` means the column's rules changed and should be applied.
    pub fn handle_key(&mut self, key: &KeyEvent) -> PopupAction {
        use FilterPopupFocus::*;
        match (self.focus, key.code) {
            (_, KeyCode::Esc) => PopupAction::Close,
            (_, KeyCode::Tab) => {
                self.cycle_focus(true);
                PopupAction::Handled
            }
            (_, KeyCode::BackTab) => {
                self.cycle_focus(false);
                PopupAction::Handled
            }
            (RuleList | Operator, KeyCode::Enter) => {
                self.focus = Value;
                PopupAction::Handled
            }
            (Value, KeyCode::Enter) => match self.save_draft() {
                Ok(()) => PopupAction::Submit,
                Err(error) => {
                    self.error = Some(error);
                    PopupAction::Handled
                }
            },
            (RuleList, KeyCode::Char(' ')) => {
                if self.toggle_selected_rule_enabled() {
                    PopupAction::Submit
                } else {
                    PopupAction::Handled
                }
            }
            (RuleList, KeyCode::Delete | KeyCode::Backspace) => {
                if self.delete_selected_rule() {
                    PopupAction::Submit
                } else {
                    PopupAction::Handled
                }
            }
            (RuleList, _) => {
                let len = self.row_count();
                if self.rules.handle_key(key, len) {
                    self.sync_editor_from_selection();
                    PopupAction::Handled
                } else {
                    PopupAction::Ignored
                }
            }
            (Operator, KeyCode::Down | KeyCode::Right | KeyCode::Char(' ')) => {
                self.next_op();
                PopupAction::Handled
            }
            (Operator, KeyCode::Up | KeyCode::Left) => {
                self.prev_op();
                PopupAction::Handled
            }
            (Operator, _) => PopupAction::Ignored,
            (Value, _) => {
                if self.draft.handle_key(key).consumed() {
                    self.error = None;
                    PopupAction::Handled
                } else {
                    PopupAction::Ignored
                }
            }
        }
    }

    /// Clicks focus the list, operator or value; clicking a rule's checkbox
    /// toggles it and its `x` deletes it (both `Submit`).
    pub fn handle_mouse(&mut self, kind: MouseEventKind, x: u16, y: u16) -> PopupAction {
        let position = Position { x, y };
        match kind {
            MouseEventKind::ScrollDown | MouseEventKind::ScrollUp
                if self.layout.rule_rows.contains(position) =>
            {
                let len = self.row_count();
                self.rules.scroll(kind == MouseEventKind::ScrollDown, len);
                self.sync_editor_from_selection();
                PopupAction::Handled
            }
            MouseEventKind::Down(MouseButton::Left) => {
                let rows = self.layout.rule_rows;
                if rows.contains(position) {
                    let Some(index) = self.rules.hit((y - rows.y) as usize, self.row_count())
                    else {
                        return PopupAction::Handled;
                    };
                    self.focus = FilterPopupFocus::RuleList;
                    self.select_rule(index);
                    if index < self.col_filter.rules.len() {
                        if x < rows.x + 4 {
                            self.toggle_selected_rule_enabled();
                            return PopupAction::Submit;
                        }
                        if x >= rows.right().saturating_sub(4) {
                            self.delete_selected_rule();
                            return PopupAction::Submit;
                        }
                    }
                    PopupAction::Handled
                } else if self.layout.operator.contains(position) {
                    if self.focus == FilterPopupFocus::Operator {
                        self.next_op();
                    }
                    self.focus = FilterPopupFocus::Operator;
                    PopupAction::Handled
                } else if self.layout.value.contains(position) {
                    self.focus = FilterPopupFocus::Value;
                    PopupAction::Handled
                } else {
                    PopupAction::Ignored
                }
            }
            _ => PopupAction::Ignored,
        }
    }

    fn hints(&self) -> Vec<Hint> {
        let mut hints = match self.focus {
            FilterPopupFocus::RuleList => vec![
                hint("Enter", "edit"),
                hint("Space", "on/off"),
                hint("Del", "remove"),
            ],
            FilterPopupFocus::Operator => vec![hint("↑↓", "operator"), hint("Enter", "to value")],
            FilterPopupFocus::Value if self.is_new_rule_selected() => {
                vec![hint("Enter", "add rule")]
            }
            FilterPopupFocus::Value => vec![hint("Enter", "update rule")],
        };
        hints.extend([hint("Tab", "next field"), hint("Esc", "close")]);
        hints
    }
}

fn format_rule(rule: &FilterRule) -> String {
    let operand = match &rule.condition {
        Condition::Lt(SqlValue::Text(text))
        | Condition::Gt(SqlValue::Text(text))
        | Condition::Eq(SqlValue::Text(text))
        | Condition::Contains(text)
        | Condition::Regex(text) => format!("\"{text}\""),
        condition => condition.operand_text().into_owned(),
    };
    format!("{} {}", rule.condition.op().symbol(), operand)
}

pub fn render(
    frame: &mut Frame,
    area: Rect,
    state: &mut FilterPopupState,
    theme: &Theme,
    symbols: &Symbols,
) {
    let width = (area.width * 3 / 5).max(60);
    let height = (area.height / 2).max(14);
    let inner =
        PopupFrame::new("Filter", Some(&state.col_name), width, height).render(frame, area, theme);
    if inner.height < 8 || inner.width < 30 {
        return;
    }
    let bg = theme.bg_raised;
    let editor_width = (inner.width * 2 / 5).clamp(22, 40);
    let list = Rect::new(
        inner.x,
        inner.y,
        inner.width - editor_width - 1,
        inner.height - 3,
    );
    let editor = Rect::new(list.right() + 1, inner.y, editor_width, inner.height - 3);
    let buf = frame.buffer_mut();
    for row in 0..list.height {
        put(
            buf,
            list.right(),
            list.y + row,
            list.right() + 1,
            &symbols.box_vertical.to_string(),
            Style::default().fg(theme.line).bg(bg),
        );
    }

    // Rule list.
    let heading = Style::default()
        .fg(theme.fg_mute)
        .bg(bg)
        .add_modifier(Modifier::BOLD);
    put(
        buf,
        list.x + 1,
        list.y,
        list.right(),
        "Rules (any may match)",
        heading,
    );
    let rows = Rect::new(
        list.x,
        list.y + 2,
        list.width.saturating_sub(1),
        list.height.saturating_sub(2),
    );
    state.layout.rule_rows = rows;
    let count = state.row_count();
    let list_focused = state.focus == FilterPopupFocus::RuleList;
    for (row, index) in state.rules.visible(count, rows.height as usize).enumerate() {
        let y = rows.y + row as u16;
        let selected = index == state.rules.selected;
        paint_row(
            buf,
            Rect::new(rows.x, y, rows.width, 1),
            selected && list_focused,
            theme,
            symbols,
        );
        let row_bg = if selected && list_focused {
            theme.bg_soft
        } else {
            bg
        };
        match state.col_filter.rules.get(index) {
            Some(rule) => {
                let check = if rule.enabled {
                    format!("[{}]", symbols.valid)
                } else {
                    "[ ]".to_string()
                };
                let fg = if rule.enabled {
                    theme.fg
                } else {
                    theme.fg_mute
                };
                let text = format!("{check} {}", format_rule(rule));
                let text = truncate_with_ellipsis(
                    &text,
                    rows.width.saturating_sub(6) as usize,
                    symbols.ellipsis,
                );
                put(
                    buf,
                    rows.x + 1,
                    y,
                    rows.right(),
                    &text,
                    Style::default().fg(fg).bg(row_bg),
                );
                put(
                    buf,
                    rows.right().saturating_sub(3),
                    y,
                    rows.right(),
                    "[x]",
                    Style::default().fg(theme.red).bg(row_bg),
                );
            }
            None => {
                put(
                    buf,
                    rows.x + 1,
                    y,
                    rows.right(),
                    "+ New rule",
                    Style::default().fg(theme.accent).bg(row_bg),
                );
            }
        }
    }
    Scrollbar {
        offset: state.rules.offset(),
        total: count,
        viewport: rows.height as usize,
    }
    .render(
        buf,
        Rect::new(rows.right(), rows.y, 1, rows.height),
        bg,
        theme,
        symbols,
    );

    // Editor.
    let title = if state.is_new_rule_selected() {
        "New rule"
    } else {
        "Edit rule"
    };
    put(buf, editor.x + 1, editor.y, editor.right(), title, heading);
    let boxed = |buf: &mut ratatui::buffer::Buffer, rect: Rect, label: &str, focused: bool| {
        let border = if focused { theme.accent } else { theme.line };
        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(border).bg(theme.bg_soft))
            .title(format!(" {label} "))
            .style(Style::default().bg(theme.bg_soft));
        let inner = block.inner(rect);
        ratatui::widgets::Widget::render(block, rect, buf);
        inner
    };
    let operator_box = Rect::new(editor.x, editor.y + 1, editor.width, 3);
    let value_box = Rect::new(editor.x, editor.y + 4, editor.width, 3);
    state.layout.operator = operator_box;
    state.layout.value = value_box;
    let operator_inner = boxed(
        buf,
        operator_box,
        "operator",
        state.focus == FilterPopupFocus::Operator,
    );
    put(
        buf,
        operator_inner.x,
        operator_inner.y,
        operator_inner.right(),
        &format!("{} {}", state.draft_op.symbol(), symbols.dropdown),
        Style::default().fg(theme.fg).bg(theme.bg_soft),
    );
    let value_focused = state.focus == FilterPopupFocus::Value;
    let value_inner = boxed(buf, value_box, "value", value_focused);
    state.draft.render(
        buf,
        value_inner,
        Style::default().fg(theme.fg).bg(theme.bg_soft),
        value_focused.then_some((
            symbols.cursor,
            Style::default().fg(theme.accent).bg(theme.bg_soft),
        )),
        symbols.ellipsis,
    );
    let status_y = value_box.bottom();
    if let Some(error) = &state.error {
        let text = truncate_with_ellipsis(error, editor.width as usize, symbols.ellipsis);
        put(
            buf,
            editor.x + 1,
            status_y,
            editor.right(),
            &text,
            Style::default().fg(theme.red).bg(bg),
        );
    } else if let Some(rule) = state.col_filter.rules.get(state.rules.selected) {
        let (label, color) = if rule.enabled {
            ("Rule is on", theme.green)
        } else {
            ("Rule is off", theme.fg_mute)
        };
        put(
            buf,
            editor.x + 1,
            status_y,
            editor.right(),
            label,
            Style::default().fg(color).bg(bg),
        );
    }

    // Footer.
    let rule_y = inner.bottom() - 3;
    let rule: String = symbols
        .box_horizontal
        .to_string()
        .repeat(inner.width as usize);
    put(
        buf,
        inner.x,
        rule_y,
        inner.right(),
        &rule,
        Style::default().fg(theme.line).bg(bg),
    );
    render_hints(
        buf,
        Rect::new(inner.x + 1, rule_y + 1, inner.width - 1, 1),
        &state.hints(),
        theme,
        bg,
    );
    put(
        buf,
        inner.x + 1,
        rule_y + 2,
        inner.right(),
        "Rules on one column: any may match · rules on different columns: all must match",
        Style::default().fg(theme.fg_faint).bg(bg),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    fn popup(rules: Vec<Condition>) -> FilterPopupState {
        FilterPopupState::new(
            "name".to_string(),
            "TEXT".to_string(),
            ColumnFilter {
                rules: rules.into_iter().map(FilterRule::new).collect(),
            },
        )
    }

    #[test]
    fn numeric_needles_are_parsed_by_column_type() {
        let mut state =
            FilterPopupState::new("amount".into(), "INTEGER".into(), ColumnFilter::default());
        state.draft_op = FilterOp::Gt;
        state.draft.set("42");
        assert_eq!(
            state.handle_key(&KeyEvent::from(KeyCode::Enter)),
            PopupAction::Submit
        );
        assert_eq!(
            state.col_filter.rules[0].condition,
            Condition::Gt(SqlValue::Integer(42))
        );

        state.rules.select(1, 2);
        state.draft.set("abc");
        assert_eq!(
            state.handle_key(&KeyEvent::from(KeyCode::Enter)),
            PopupAction::Handled
        );
        assert!(state
            .error
            .as_deref()
            .is_some_and(|e| e.contains("integer")));
    }

    #[test]
    fn rule_list_edits_toggle_and_delete() {
        let mut state = popup(vec![
            Condition::Contains("a".into()),
            Condition::Contains("b".into()),
        ]);
        assert_eq!(state.focus, FilterPopupFocus::RuleList);
        state.handle_key(&KeyEvent::from(KeyCode::Down));
        assert_eq!(state.draft.value(), "b");
        assert_eq!(
            state.handle_key(&KeyEvent::from(KeyCode::Char(' '))),
            PopupAction::Submit
        );
        assert!(!state.col_filter.rules[1].enabled);
        assert_eq!(
            state.handle_key(&KeyEvent::from(KeyCode::Delete)),
            PopupAction::Submit
        );
        assert_eq!(state.col_filter.rules.len(), 1);
        assert_eq!(state.rules.selected, 0);
    }

    #[test]
    fn editing_an_existing_rule_replaces_its_condition() {
        let mut state = popup(vec![Condition::Contains("gon".into())]);
        state.focus = FilterPopupFocus::Value;
        state.draft_op = FilterOp::Regex;
        state.draft.set("^g.*");
        assert_eq!(
            state.handle_key(&KeyEvent::from(KeyCode::Enter)),
            PopupAction::Submit
        );
        assert_eq!(
            state.col_filter.rules,
            vec![FilterRule::new(Condition::Regex("^g.*".into()))]
        );
    }

    #[test]
    fn long_rule_lists_scroll_with_the_selection() {
        let conditions = (0..30)
            .map(|i| Condition::Contains(format!("v{i}")))
            .collect();
        let mut state = popup(conditions);
        state.handle_key(&KeyEvent::from(KeyCode::End));
        assert!(state.is_new_rule_selected());
        let range = state.rules.visible(31, 5);
        assert_eq!(range, 26..31);
    }
}
