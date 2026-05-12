use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{block::BorderType, Block, Borders, Paragraph},
    Frame,
};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use crate::{
    db::types::SqlValue,
    filter::{rule::FilterRule, FilterOp, FilterValue},
    symbols::Symbols,
    theme::Theme,
};

pub struct FilterPopupState {
    pub col_name: String,
    pub col_type: String,
    pub col_filter: crate::filter::ColumnFilter,
    pub selected_rule: usize,
    pub draft_op: crate::filter::FilterOp,
    pub draft_value: String,
    pub draft_cursor_pos: usize,
    pub focus: FilterPopupFocus,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum FilterPopupFocus {
    RuleList,
    Operator,
    Value,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FilterPopupHit {
    RuleRow(usize),
    RuleToggle(usize),
    RuleDelete(usize),
    Operator,
    OperatorChevron,
    Value(u16),
}

#[derive(Clone, Copy)]
struct FilterPopupLayout {
    popup_area: Rect,
    rule_list_area: Rect,
    rule_rows_area: Rect,
    divider_area: Rect,
    editor_area: Rect,
    operator_box: Rect,
    operator_inner: Rect,
    value_box: Rect,
    value_inner: Rect,
    footer_area: Rect,
}

impl FilterPopupState {
    pub fn new(
        col_name: String,
        col_type: String,
        col_filter: crate::filter::ColumnFilter,
    ) -> Self {
        let has_rules = !col_filter.rules.is_empty();
        let draft_op = col_filter
            .rules
            .last()
            .map(|rule| rule.op)
            .unwrap_or(crate::filter::FilterOp::Contains);
        let mut state = Self {
            col_name,
            col_type,
            col_filter,
            selected_rule: 0,
            draft_op,
            draft_value: String::new(),
            draft_cursor_pos: 0,
            focus: if has_rules {
                FilterPopupFocus::RuleList
            } else {
                FilterPopupFocus::Value
            },
        };
        state.sync_editor_from_selection();
        state
    }

    pub fn next_op(&mut self) {
        let ops = popup_ops();
        let idx = ops.iter().position(|op| *op == self.draft_op).unwrap_or(0);
        self.draft_op = ops[(idx + 1) % ops.len()];
    }

    pub fn prev_op(&mut self) {
        let ops = popup_ops();
        let idx = ops.iter().position(|op| *op == self.draft_op).unwrap_or(0);
        self.draft_op = ops[(idx + ops.len() - 1) % ops.len()];
    }

    pub fn select_prev_rule(&mut self) {
        self.selected_rule = self.selected_rule.saturating_sub(1);
        self.sync_editor_from_selection();
    }

    pub fn select_next_rule(&mut self) {
        if self.selected_rule < self.col_filter.rules.len() {
            self.selected_rule += 1;
        }
        self.sync_editor_from_selection();
    }

    pub fn move_cursor_left(&mut self) {
        self.draft_cursor_pos = self.draft_cursor_pos.saturating_sub(1);
    }

    pub fn move_cursor_right(&mut self) {
        let len = self.draft_value.chars().count();
        if self.draft_cursor_pos < len {
            self.draft_cursor_pos += 1;
        }
    }

    pub fn push_char(&mut self, ch: char) {
        let byte_pos = self
            .draft_value
            .char_indices()
            .nth(self.draft_cursor_pos)
            .map_or(self.draft_value.len(), |(i, _)| i);
        self.draft_value.insert(byte_pos, ch);
        self.draft_cursor_pos += 1;
    }

    pub fn pop_char(&mut self) {
        if self.draft_cursor_pos == 0 {
            return;
        }
        let byte_pos = self
            .draft_value
            .char_indices()
            .nth(self.draft_cursor_pos - 1)
            .map(|(i, _)| i)
            .unwrap_or(0);
        let end_pos = self
            .draft_value
            .char_indices()
            .nth(self.draft_cursor_pos)
            .map_or(self.draft_value.len(), |(i, _)| i);
        self.draft_value.replace_range(byte_pos..end_pos, "");
        self.draft_cursor_pos -= 1;
    }

    pub fn next_focus(&mut self) {
        self.focus = match self.focus {
            FilterPopupFocus::RuleList => FilterPopupFocus::Operator,
            FilterPopupFocus::Operator => FilterPopupFocus::Value,
            FilterPopupFocus::Value => FilterPopupFocus::RuleList,
        };
    }

    pub fn prev_focus(&mut self) {
        self.focus = match self.focus {
            FilterPopupFocus::RuleList => FilterPopupFocus::Value,
            FilterPopupFocus::Operator => FilterPopupFocus::RuleList,
            FilterPopupFocus::Value => FilterPopupFocus::Operator,
        };
    }

    pub fn focus_rule_list(&mut self) {
        self.focus = FilterPopupFocus::RuleList;
    }

    pub fn focus_operator(&mut self) {
        self.focus = FilterPopupFocus::Operator;
    }

    pub fn focus_value(&mut self) {
        self.focus = FilterPopupFocus::Value;
    }

    pub fn set_selected_rule(&mut self, selected_rule: usize) {
        self.selected_rule = selected_rule.min(self.col_filter.rules.len());
        self.sync_editor_from_selection();
    }

    pub fn set_cursor_from_display_x(&mut self, display_x: u16) {
        let mut width = 0u16;
        let mut cursor = 0usize;
        for ch in self.draft_value.chars() {
            let ch_width = UnicodeWidthChar::width(ch).unwrap_or(1) as u16;
            if width + ch_width > display_x {
                break;
            }
            width += ch_width;
            cursor += 1;
        }
        self.draft_cursor_pos = cursor;
    }

    pub fn delete_selected_rule(&mut self) -> bool {
        if self.selected_rule < self.col_filter.rules.len() {
            self.col_filter.rules.remove(self.selected_rule);
            if !self.col_filter.rules.is_empty()
                && self.selected_rule == self.col_filter.rules.len()
            {
                self.selected_rule = self.selected_rule.saturating_sub(1);
            }
            self.clamp_selection();
            self.sync_editor_from_selection();
            return true;
        }
        false
    }

    pub fn toggle_selected_rule_enabled(&mut self) -> bool {
        if let Some(rule) = self.col_filter.rules.get_mut(self.selected_rule) {
            rule.enabled = !rule.enabled;
            return true;
        }
        false
    }

    pub fn add_rule(&mut self) -> Result<(), String> {
        let mut rule = self.build_rule()?;
        if self.selected_rule < self.col_filter.rules.len() {
            let existing = &self.col_filter.rules[self.selected_rule];
            rule.enabled = existing.enabled;
            rule.label = existing.label.clone();
            self.col_filter.rules[self.selected_rule] = rule;
        } else {
            self.col_filter.rules.push(rule);
            self.selected_rule = self.col_filter.rules.len().saturating_sub(1);
        }
        self.sync_editor_from_selection();
        Ok(())
    }

    pub fn display_draft_value(&self, cursor: char) -> String {
        let before: String = self
            .draft_value
            .chars()
            .take(self.draft_cursor_pos)
            .collect();
        let after: String = self
            .draft_value
            .chars()
            .skip(self.draft_cursor_pos)
            .collect();
        format!("{before}{cursor}{after}")
    }

    fn build_rule(&self) -> Result<FilterRule, String> {
        let needle = self.draft_value.trim();
        if needle.is_empty() {
            return Err("Needle is required".to_string());
        }

        let value = match self.draft_op {
            FilterOp::Contains => FilterValue::Pattern(needle.to_string()),
            FilterOp::Regex => FilterValue::Regex(needle.to_string()),
            FilterOp::Eq | FilterOp::Lt | FilterOp::Gt => {
                FilterValue::Literal(self.parse_literal(needle)?)
            }
            _ => return Err("Unsupported filter operator".to_string()),
        };

        Ok(FilterRule {
            op: self.draft_op,
            value,
            enabled: true,
            label: None,
        })
    }

    fn parse_literal(&self, needle: &str) -> Result<SqlValue, String> {
        let upper = self.col_type.to_uppercase();
        if upper.contains("INT") {
            return needle
                .parse::<i64>()
                .map(SqlValue::Integer)
                .map_err(|_| "Needle must be a valid integer".to_string());
        }
        if upper.contains("REAL")
            || upper.contains("FLOAT")
            || upper.contains("DOUBLE")
            || upper.contains("NUM")
        {
            return needle
                .parse::<f64>()
                .map(SqlValue::Real)
                .map_err(|_| "Needle must be a valid number".to_string());
        }
        Ok(SqlValue::Text(needle.to_string()))
    }

    fn clamp_selection(&mut self) {
        let max_index = self.col_filter.rules.len();
        if self.selected_rule > max_index {
            self.selected_rule = max_index;
        }
    }

    pub fn is_new_rule_selected(&self) -> bool {
        self.selected_rule >= self.col_filter.rules.len()
    }

    fn sync_editor_from_selection(&mut self) {
        if let Some(rule) = self.col_filter.rules.get(self.selected_rule) {
            if popup_ops().contains(&rule.op) {
                self.draft_op = rule.op;
            }
            self.draft_value = draft_value(rule);
            self.draft_cursor_pos = self.draft_value.chars().count();
        } else {
            self.draft_value.clear();
            self.draft_cursor_pos = 0;
            if !popup_ops().contains(&self.draft_op) {
                self.draft_op = FilterOp::Contains;
            }
        }
    }
}

fn popup_layout(area: Rect) -> FilterPopupLayout {
    let popup_w = ((area.width * 3) / 4)
        .max(60)
        .min(area.width)
        .saturating_sub(25)
        .max(40)
        .min(area.width);
    let popup_h = ((area.height * 3) / 5)
        .saturating_sub(6)
        .max(12)
        .min(area.height);
    let popup_area = Rect {
        x: area.x + (area.width.saturating_sub(popup_w)) / 2,
        y: area.y + (area.height.saturating_sub(popup_h)) / 2,
        width: popup_w,
        height: popup_h,
    };
    let inner = inner_rect(popup_area);
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(7), Constraint::Length(3)])
        .split(inner);
    let body_width = chunks[0].width.saturating_sub(1);
    let editor_width = ((body_width * 38) / 100)
        .saturating_add(5)
        .min(body_width.saturating_sub(20));
    let rule_list_width = body_width.saturating_sub(editor_width);
    let body = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Length(rule_list_width),
            Constraint::Length(1),
            Constraint::Length(editor_width),
        ])
        .split(chunks[0]);
    let editor_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Length(3),
            Constraint::Length(3),
            Constraint::Min(2),
        ])
        .split(body[2]);

    FilterPopupLayout {
        popup_area,
        rule_list_area: body[0],
        rule_rows_area: Rect {
            x: body[0].x,
            y: body[0].y.saturating_add(2),
            width: body[0].width,
            height: body[0].height.saturating_sub(2),
        },
        divider_area: body[1],
        editor_area: body[2],
        operator_box: editor_chunks[1],
        operator_inner: inner_rect(editor_chunks[1]),
        value_box: editor_chunks[2],
        value_inner: inner_rect(editor_chunks[2]),
        footer_area: chunks[1],
    }
}

fn inner_rect(area: Rect) -> Rect {
    Rect {
        x: area.x.saturating_add(1),
        y: area.y.saturating_add(1),
        width: area.width.saturating_sub(2),
        height: area.height.saturating_sub(2),
    }
}

pub fn render(
    frame: &mut Frame,
    area: Rect,
    state: &FilterPopupState,
    theme: &Theme,
    symbols: &Symbols,
) {
    let layout = popup_layout(area);
    let popup_area = layout.popup_area;

    super::paint_popup_surface(frame, popup_area, theme);

    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(theme.accent))
        .title(format!(" Filter: {} ", state.col_name))
        .style(Style::default().bg(theme.bg_raised));
    frame.render_widget(block, popup_area);

    for row in 0..layout.divider_area.height {
        frame.buffer_mut().set_string(
            layout.divider_area.x,
            layout.divider_area.y + row,
            symbols.box_vertical.to_string(),
            Style::default().fg(theme.line).bg(theme.bg_raised),
        );
    }

    render_rule_list(frame, layout.rule_list_area, state, theme, symbols);
    render_editor(frame, &layout, state, theme, symbols);
    render_footer(frame, layout.footer_area, state, theme, symbols);
}

pub fn hit_test(area: Rect, state: &FilterPopupState, x: u16, y: u16) -> Option<FilterPopupHit> {
    let layout = popup_layout(area);
    if x < layout.popup_area.x
        || x >= layout.popup_area.x + layout.popup_area.width
        || y < layout.popup_area.y
        || y >= layout.popup_area.y + layout.popup_area.height
    {
        return None;
    }

    if x >= layout.rule_rows_area.x
        && x < layout.rule_rows_area.x + layout.rule_rows_area.width
        && y >= layout.rule_rows_area.y
        && y < layout.rule_rows_area.y + layout.rule_rows_area.height
    {
        let row_index = (y - layout.rule_rows_area.y) as usize;
        if row_index > state.col_filter.rules.len() {
            return None;
        }
        let is_new_row = row_index == state.col_filter.rules.len();
        if !is_new_row {
            if x < layout.rule_rows_area.x.saturating_add(3) {
                return Some(FilterPopupHit::RuleToggle(row_index));
            }
            let actions_width = UnicodeWidthStr::width(" [x]") as u16;
            let action_x = layout
                .rule_rows_area
                .x
                .saturating_add(layout.rule_rows_area.width.saturating_sub(actions_width));
            if x >= action_x && x < action_x.saturating_add(actions_width) {
                return Some(FilterPopupHit::RuleDelete(row_index));
            }
        }
        return Some(FilterPopupHit::RuleRow(row_index));
    }

    if x >= layout.operator_box.x
        && x < layout.operator_box.x + layout.operator_box.width
        && y >= layout.operator_box.y
        && y < layout.operator_box.y + layout.operator_box.height
    {
        let chevron_x = layout
            .operator_inner
            .x
            .saturating_add(layout.operator_inner.width.saturating_sub(1));
        return Some(if x == chevron_x {
            FilterPopupHit::OperatorChevron
        } else {
            FilterPopupHit::Operator
        });
    }

    if x >= layout.value_box.x
        && x < layout.value_box.x + layout.value_box.width
        && y >= layout.value_box.y
        && y < layout.value_box.y + layout.value_box.height
    {
        return Some(FilterPopupHit::Value(
            x.saturating_sub(layout.value_inner.x),
        ));
    }

    None
}

fn popup_ops() -> &'static [FilterOp] {
    &[
        FilterOp::Lt,
        FilterOp::Gt,
        FilterOp::Eq,
        FilterOp::Contains,
        FilterOp::Regex,
    ]
}

fn popup_op_label(op: FilterOp) -> &'static str {
    match op {
        FilterOp::Lt => "<",
        FilterOp::Gt => ">",
        FilterOp::Eq => "==",
        FilterOp::Contains => "contains",
        FilterOp::Regex => "regexp",
        _ => op.label(),
    }
}

fn draft_value(rule: &FilterRule) -> String {
    match &rule.value {
        FilterValue::Literal(SqlValue::Null) => "NULL".to_string(),
        FilterValue::Literal(SqlValue::Integer(n)) => n.to_string(),
        FilterValue::Literal(SqlValue::Real(f)) => f.to_string(),
        FilterValue::Literal(SqlValue::Text(s)) => s.clone(),
        FilterValue::Literal(SqlValue::Blob(b)) => format!("<blob {} bytes>", b.len()),
        FilterValue::Pattern(s) => s.clone(),
        FilterValue::Regex(s) => s.clone(),
        FilterValue::Range(_, _)
        | FilterValue::List(_)
        | FilterValue::Formula(_)
        | FilterValue::N(_) => format_rule(rule),
    }
}

fn format_rule(rule: &FilterRule) -> String {
    let value = match &rule.value {
        FilterValue::Literal(SqlValue::Null) => "NULL".to_string(),
        FilterValue::Literal(SqlValue::Integer(n)) => n.to_string(),
        FilterValue::Literal(SqlValue::Real(f)) => f.to_string(),
        FilterValue::Literal(SqlValue::Text(s)) => format!("\"{}\"", s),
        FilterValue::Literal(SqlValue::Blob(b)) => format!("<blob {} bytes>", b.len()),
        FilterValue::Pattern(s) => format!("\"{}\"", s),
        FilterValue::Regex(s) => format!("\"{}\"", s),
        FilterValue::Range(lo, hi) => format!("{lo:?}..{hi:?}"),
        FilterValue::List(values) => format!("{} values", values.len()),
        FilterValue::Formula(s) => s.clone(),
        FilterValue::N(n) => n.to_string(),
    };
    format!("{} {}", popup_op_label(rule.op), value)
}

fn format_rule_summary(col_name: &str, rule: &FilterRule) -> String {
    format!("{col_name} {}", format_rule(rule))
}

fn render_rule_list(
    frame: &mut Frame,
    area: Rect,
    state: &FilterPopupState,
    theme: &Theme,
    symbols: &Symbols,
) {
    let buf = frame.buffer_mut();
    let title = format!(" Rules in {} ", state.col_name);
    buf.set_string(
        area.x,
        area.y,
        truncate(&title, area.width as usize, symbols.ellipsis),
        Style::default()
            .fg(theme.fg_faint)
            .bg(theme.bg_raised)
            .add_modifier(Modifier::BOLD),
    );

    let rows_area = Rect {
        x: area.x,
        y: area.y.saturating_add(2),
        width: area.width,
        height: area.height.saturating_sub(2),
    };

    for row in 0..rows_area.height {
        let y = rows_area.y + row;
        let row_index = row as usize;
        let is_new_row = row_index == state.col_filter.rules.len();
        if row_index > state.col_filter.rules.len() {
            break;
        }

        let is_selected = row_index == state.selected_rule;
        let is_active = is_selected && state.focus == FilterPopupFocus::RuleList;
        let bg = if is_selected {
            theme.bg_soft
        } else {
            theme.bg_raised
        };
        let base_style = Style::default()
            .fg(if is_new_row {
                theme.accent
            } else if state
                .col_filter
                .rules
                .get(row_index)
                .is_some_and(|rule| rule.enabled)
            {
                if is_selected {
                    theme.fg
                } else {
                    theme.fg_dim
                }
            } else {
                theme.fg_mute
            })
            .bg(bg)
            .add_modifier(if is_selected {
                Modifier::BOLD
            } else {
                Modifier::empty()
            });

        buf.set_string(
            area.x,
            y,
            " ".repeat(area.width as usize),
            Style::default().bg(bg),
        );

        let actions = if is_new_row { "" } else { " [x]" };
        let actions_width = UnicodeWidthStr::width(actions);
        let text_width = area.width.saturating_sub(actions_width as u16) as usize;
        let text = if is_new_row {
            "+ New rule".to_string()
        } else if let Some(rule) = state.col_filter.rules.get(row_index) {
            let checkbox = if rule.enabled {
                format!("[{}]", symbols.valid)
            } else {
                "[ ]".to_string()
            };
            format!("{checkbox} {}", format_rule_summary(&state.col_name, rule))
        } else {
            String::new()
        };

        buf.set_string(
            area.x,
            y,
            truncate(&text, text_width, symbols.ellipsis),
            base_style,
        );
        if !actions.is_empty() {
            let action_x = area.x + area.width.saturating_sub(actions_width as u16);
            buf.set_string(
                action_x,
                y,
                actions,
                Style::default()
                    .fg(theme.red)
                    .bg(bg)
                    .add_modifier(if is_active {
                        Modifier::BOLD
                    } else {
                        Modifier::empty()
                    }),
            );
        }

        if is_active {
            buf.set_string(
                area.x,
                y,
                symbols.active_bar.to_string(),
                Style::default().fg(theme.accent).bg(bg),
            );
        }
    }
}

fn render_editor(
    frame: &mut Frame,
    layout: &FilterPopupLayout,
    state: &FilterPopupState,
    theme: &Theme,
    symbols: &Symbols,
) {
    let area = layout.editor_area;
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Length(3),
            Constraint::Length(3),
            Constraint::Min(2),
        ])
        .split(area);
    let heading = if state.is_new_rule_selected() {
        " New rule "
    } else {
        " Edit rule "
    };
    frame.buffer_mut().set_string(
        area.x,
        area.y,
        truncate(heading, area.width as usize, symbols.ellipsis),
        Style::default()
            .fg(theme.fg_faint)
            .bg(theme.bg_raised)
            .add_modifier(Modifier::BOLD),
    );

    let op_border = if state.focus == FilterPopupFocus::Operator {
        theme.accent
    } else {
        theme.line
    };
    let value_border = if state.focus == FilterPopupFocus::Value {
        theme.accent
    } else {
        theme.line
    };
    let op_block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(op_border).bg(theme.bg_soft))
        .title(" operator ")
        .style(Style::default().bg(theme.bg_soft));
    let value_block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(value_border).bg(theme.bg_soft))
        .title(" value ")
        .style(Style::default().bg(theme.bg_soft));
    frame.render_widget(op_block, layout.operator_box);
    frame.render_widget(value_block, layout.value_box);

    frame.buffer_mut().set_string(
        layout.operator_inner.x,
        layout.operator_inner.y,
        truncate(
            &format!("{} {}", popup_op_label(state.draft_op), symbols.dropdown),
            layout.operator_inner.width as usize,
            symbols.ellipsis,
        ),
        Style::default()
            .fg(if state.focus == FilterPopupFocus::Operator {
                theme.fg
            } else {
                theme.fg_dim
            })
            .bg(theme.bg_soft),
    );

    let editor_value = if state.focus == FilterPopupFocus::Value {
        state.display_draft_value(symbols.cursor)
    } else {
        state.draft_value.clone()
    };
    frame.buffer_mut().set_string(
        layout.value_inner.x,
        layout.value_inner.y,
        truncate(
            &editor_value,
            layout.value_inner.width as usize,
            symbols.ellipsis,
        ),
        Style::default()
            .fg(if state.focus == FilterPopupFocus::Value {
                theme.fg
            } else {
                theme.fg_dim
            })
            .bg(theme.bg_soft),
    );

    let status = if state.is_new_rule_selected() {
        Line::from(vec![
            Span::styled("Create", Style::default().fg(theme.accent)),
            Span::styled(
                " a new rule for this column.",
                Style::default().fg(theme.fg_dim),
            ),
        ])
    } else if state
        .col_filter
        .rules
        .get(state.selected_rule)
        .is_some_and(|rule| rule.enabled)
    {
        Line::from(vec![
            Span::styled("Enabled", Style::default().fg(theme.green)),
            Span::styled(
                " rule. Space toggles it off.",
                Style::default().fg(theme.fg_dim),
            ),
        ])
    } else {
        Line::from(vec![
            Span::styled("Disabled", Style::default().fg(theme.red)),
            Span::styled(
                " rule. Space toggles it on.",
                Style::default().fg(theme.fg_dim),
            ),
        ])
    };
    frame.render_widget(
        Paragraph::new(vec![
            status,
            Line::from(vec![Span::styled(
                "Tab moves between list, operator,",
                Style::default().fg(theme.fg_faint),
            )]),
            Line::from(vec![Span::styled(
                "and value.",
                Style::default().fg(theme.fg_faint),
            )]),
        ])
        .style(Style::default().bg(theme.bg_raised)),
        chunks[3],
    );
}

fn render_footer(
    frame: &mut Frame,
    area: Rect,
    state: &FilterPopupState,
    theme: &Theme,
    symbols: &Symbols,
) {
    frame.buffer_mut().set_string(
        area.x,
        area.y,
        symbols
            .box_horizontal
            .to_string()
            .repeat(area.width as usize),
        Style::default().fg(theme.line).bg(theme.bg_raised),
    );

    let save_label = if state.is_new_rule_selected() {
        "Enter add rule"
    } else {
        "Enter edit rule"
    };
    let action_line = Line::from(vec![
        Span::styled(
            format!("{} ", symbols.valid),
            Style::default().fg(theme.green),
        ),
        Span::styled(save_label, Style::default().fg(theme.green)),
        Span::styled(
            symbols.padded_separator(),
            Style::default().fg(theme.fg_faint),
        ),
        Span::styled("Space activate", Style::default().fg(theme.yellow)),
        Span::styled(
            symbols.padded_separator(),
            Style::default().fg(theme.fg_faint),
        ),
        Span::styled("Del remove", Style::default().fg(theme.red)),
        Span::styled(
            symbols.padded_separator(),
            Style::default().fg(theme.fg_faint),
        ),
        Span::styled("Esc close", Style::default().fg(theme.fg)),
    ]);

    frame.render_widget(
        Paragraph::new(vec![
            action_line,
            Line::from(vec![Span::styled(
                format!(
                    "Rules in this column use OR {} different columns use AND.",
                    symbols.separator
                ),
                Style::default().fg(theme.fg_dim),
            )]),
        ])
        .style(Style::default().bg(theme.bg_raised)),
        Rect {
            x: area.x,
            y: area.y.saturating_add(1),
            width: area.width,
            height: area.height.saturating_sub(1),
        },
    );
}

fn truncate(text: &str, max_width: usize, ellipsis: char) -> String {
    if max_width == 0 {
        return String::new();
    }
    let mut out = String::new();
    let mut used = 0usize;
    for ch in text.chars() {
        let width = UnicodeWidthChar::width(ch).unwrap_or(1);
        if used + width > max_width {
            break;
        }
        out.push(ch);
        used += width;
    }
    if UnicodeWidthStr::width(text) > max_width && max_width > 1 {
        out.pop();
        out.push(ellipsis);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::{hit_test, popup_layout, FilterPopupFocus, FilterPopupHit, FilterPopupState};
    use crate::{
        db::types::SqlValue,
        filter::{rule::FilterRule, ColumnFilter, FilterOp},
    };
    use ratatui::layout::Rect;

    #[test]
    fn add_rule_parses_numeric_needles() {
        let mut state = FilterPopupState::new(
            "amount".to_string(),
            "INTEGER".to_string(),
            ColumnFilter::default(),
        );
        state.draft_op = FilterOp::Gt;
        state.draft_value = "42".to_string();
        state.draft_cursor_pos = 2;

        state.add_rule().expect("expected valid rule");

        assert_eq!(state.col_filter.rules.len(), 1);
        assert!(matches!(
            state.col_filter.rules[0].value,
            crate::filter::FilterValue::Literal(SqlValue::Integer(42))
        ));
    }

    #[test]
    fn delete_selected_rule_clamps_selection() {
        let mut state = FilterPopupState::new(
            "name".to_string(),
            "TEXT".to_string(),
            ColumnFilter {
                rules: vec![
                    FilterRule {
                        op: FilterOp::Contains,
                        value: crate::filter::FilterValue::Pattern("a".to_string()),
                        enabled: true,
                        label: None,
                    },
                    FilterRule {
                        op: FilterOp::Contains,
                        value: crate::filter::FilterValue::Pattern("b".to_string()),
                        enabled: true,
                        label: None,
                    },
                ],
            },
        );
        state.selected_rule = 1;
        state.focus = FilterPopupFocus::RuleList;

        assert!(state.delete_selected_rule());
        assert_eq!(state.selected_rule, 0);
        assert_eq!(state.col_filter.rules.len(), 1);
    }

    #[test]
    fn needle_cursor_edits_in_place() {
        let mut state = FilterPopupState::new(
            "name".to_string(),
            "TEXT".to_string(),
            ColumnFilter::default(),
        );
        state.push_char('a');
        state.push_char('c');
        state.move_cursor_left();
        state.push_char('b');

        assert_eq!(state.draft_value, "abc");
        assert_eq!(state.draft_cursor_pos, 2);
    }

    #[test]
    fn selecting_existing_rule_loads_it_into_editor() {
        let state = FilterPopupState::new(
            "name".to_string(),
            "TEXT".to_string(),
            ColumnFilter {
                rules: vec![FilterRule {
                    op: FilterOp::Contains,
                    value: crate::filter::FilterValue::Pattern("gon".to_string()),
                    enabled: true,
                    label: None,
                }],
            },
        );

        assert_eq!(state.draft_op, FilterOp::Contains);
        assert_eq!(state.draft_value, "gon");
        assert_eq!(state.draft_cursor_pos, 3);
    }

    #[test]
    fn toggle_selected_rule_enabled_flips_state() {
        let mut state = FilterPopupState::new(
            "name".to_string(),
            "TEXT".to_string(),
            ColumnFilter {
                rules: vec![FilterRule {
                    op: FilterOp::Contains,
                    value: crate::filter::FilterValue::Pattern("gon".to_string()),
                    enabled: true,
                    label: None,
                }],
            },
        );

        assert!(state.toggle_selected_rule_enabled());
        assert!(!state.col_filter.rules[0].enabled);
    }

    #[test]
    fn select_next_rule_can_reach_new_rule_row() {
        let mut state = FilterPopupState::new(
            "name".to_string(),
            "TEXT".to_string(),
            ColumnFilter {
                rules: vec![FilterRule {
                    op: FilterOp::Contains,
                    value: crate::filter::FilterValue::Pattern("gon".to_string()),
                    enabled: true,
                    label: None,
                }],
            },
        );

        state.select_next_rule();

        assert!(state.is_new_rule_selected());
        assert_eq!(state.draft_value, "");
    }

    #[test]
    fn add_rule_updates_selected_rule_when_editing_existing() {
        let mut state = FilterPopupState::new(
            "name".to_string(),
            "TEXT".to_string(),
            ColumnFilter {
                rules: vec![FilterRule {
                    op: FilterOp::Contains,
                    value: crate::filter::FilterValue::Pattern("gon".to_string()),
                    enabled: true,
                    label: None,
                }],
            },
        );
        state.draft_op = FilterOp::Regex;
        state.draft_value = "^g.*".to_string();
        state.draft_cursor_pos = 4;

        state.add_rule().expect("expected updated rule");

        assert_eq!(state.col_filter.rules.len(), 1);
        assert_eq!(state.col_filter.rules[0].op, FilterOp::Regex);
    }

    #[test]
    fn hit_test_finds_checkbox_and_operator_chevron() {
        let state = FilterPopupState::new(
            "name".to_string(),
            "TEXT".to_string(),
            ColumnFilter {
                rules: vec![FilterRule {
                    op: FilterOp::Contains,
                    value: crate::filter::FilterValue::Pattern("gon".to_string()),
                    enabled: true,
                    label: None,
                }],
            },
        );
        let area = Rect {
            x: 0,
            y: 0,
            width: 120,
            height: 40,
        };
        let layout = popup_layout(area);
        let checkbox_x = layout.rule_rows_area.x + 1;
        let row_y = layout.rule_rows_area.y;
        let chevron_x = layout.operator_inner.x + layout.operator_inner.width.saturating_sub(1);
        let chevron_y = layout.operator_inner.y;

        assert_eq!(
            hit_test(area, &state, checkbox_x, row_y),
            Some(FilterPopupHit::RuleToggle(0))
        );
        assert_eq!(
            hit_test(area, &state, chevron_x, chevron_y),
            Some(FilterPopupHit::OperatorChevron)
        );
    }
}
