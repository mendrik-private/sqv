use std::collections::BTreeSet;

use crossterm::event::{KeyCode, KeyEvent, MouseButton, MouseEventKind};
use ratatui::{
    layout::{Position, Rect},
    style::Style,
    Frame,
};
use serde_json::Value;

use super::PopupAction;
use crate::{
    symbols::Symbols,
    theme::Theme,
    ui::widgets::{
        frame::PopupFrame,
        hints::{hint, render_hints},
        list::{paint_row, ListCursor},
        scrollbar::Scrollbar,
        text::{put, sanitize, truncate_with_ellipsis},
    },
};

/// A JSON document as a collapsible tree.
pub struct JsonViewState {
    pub title: String,
    root: Value,
    /// Paths of expanded containers; the root starts expanded.
    expanded: BTreeSet<String>,
    pub list: ListCursor,
    list_area: Rect,
}

struct Node<'a> {
    path: String,
    depth: usize,
    key: String,
    value: &'a Value,
}

impl JsonViewState {
    pub fn new(title: String, root: Value) -> Self {
        Self {
            title,
            root,
            expanded: BTreeSet::from([String::new()]),
            list: ListCursor::default(),
            list_area: Rect::default(),
        }
    }

    fn nodes(&self) -> Vec<Node<'_>> {
        let mut nodes = Vec::new();
        self.collect(&self.root, String::new(), "$".to_string(), 0, &mut nodes);
        nodes
    }

    fn collect<'a>(
        &self,
        value: &'a Value,
        path: String,
        key: String,
        depth: usize,
        out: &mut Vec<Node<'a>>,
    ) {
        let open = self.expanded.contains(&path);
        out.push(Node {
            path: path.clone(),
            depth,
            key,
            value,
        });
        if !open {
            return;
        }
        match value {
            Value::Object(map) => {
                for (key, child) in map {
                    self.collect(child, format!("{path}.{key}"), key.clone(), depth + 1, out);
                }
            }
            Value::Array(items) => {
                for (index, child) in items.iter().enumerate() {
                    self.collect(
                        child,
                        format!("{path}[{index}]"),
                        format!("[{index}]"),
                        depth + 1,
                        out,
                    );
                }
            }
            _ => {}
        }
    }

    /// The selected node as JSON text, for copying.
    pub fn selected_text(&self) -> Option<String> {
        let nodes = self.nodes();
        let node = nodes.get(self.list.selected)?;
        Some(match node.value {
            Value::String(text) => text.clone(),
            value => serde_json::to_string_pretty(value).ok()?,
        })
    }

    fn toggle(&mut self, open: Option<bool>) {
        let nodes = self.nodes();
        let Some(node) = nodes.get(self.list.selected) else {
            return;
        };
        if !matches!(node.value, Value::Object(_) | Value::Array(_)) {
            return;
        }
        let path = node.path.clone();
        let is_open = self.expanded.contains(&path);
        match open.unwrap_or(!is_open) {
            true => self.expanded.insert(path),
            false => self.expanded.remove(&path),
        };
    }

    pub fn handle_key(&mut self, key: &KeyEvent) -> PopupAction {
        let len = self.nodes().len();
        match key.code {
            KeyCode::Esc => return PopupAction::Close,
            KeyCode::Char('y') => return PopupAction::Copy,
            KeyCode::Enter | KeyCode::Char(' ') => self.toggle(None),
            KeyCode::Right | KeyCode::Char('l') => self.toggle(Some(true)),
            KeyCode::Left | KeyCode::Char('h') => self.toggle(Some(false)),
            KeyCode::Char('k') => self.list.move_by(-1, len),
            KeyCode::Char('j') => self.list.move_by(1, len),
            _ if self.list.handle_key(key, len) => {}
            _ => return PopupAction::Ignored,
        }
        PopupAction::Handled
    }

    pub fn handle_mouse(&mut self, kind: MouseEventKind, x: u16, y: u16) -> PopupAction {
        let len = self.nodes().len();
        match kind {
            MouseEventKind::ScrollDown | MouseEventKind::ScrollUp => {
                self.list.scroll(kind == MouseEventKind::ScrollDown, len);
                PopupAction::Handled
            }
            MouseEventKind::Down(MouseButton::Left)
                if self.list_area.contains(Position { x, y }) =>
            {
                if let Some(index) = self.list.hit((y - self.list_area.y) as usize, len) {
                    if index == self.list.selected {
                        self.toggle(None);
                    }
                    self.list.select(index, self.nodes().len());
                }
                PopupAction::Handled
            }
            _ => PopupAction::Ignored,
        }
    }
}

fn preview(value: &Value, open: bool) -> String {
    match value {
        Value::Object(_) if open => "{".to_string(),
        Value::Object(map) => format!("{{…}} {} keys", map.len()),
        Value::Array(_) if open => "[".to_string(),
        Value::Array(items) => format!("[…] {} items", items.len()),
        Value::String(text) => format!("\"{}\"", sanitize(text)),
        value => value.to_string(),
    }
}

pub fn render(
    frame: &mut Frame,
    area: Rect,
    state: &mut JsonViewState,
    theme: &Theme,
    symbols: &Symbols,
) {
    let inner = PopupFrame::new(
        "JSON",
        Some(&state.title),
        (area.width * 7 / 10).max(50),
        (area.height * 8 / 10).max(10),
    )
    .render(frame, area, theme);
    if inner.height < 3 {
        return;
    }
    let bg = theme.bg_raised;
    let list = Rect::new(
        inner.x,
        inner.y,
        inner.width.saturating_sub(1),
        inner.height - 1,
    );
    state.list_area = list;
    let len = state.nodes().len();
    let visible = state.list.visible(len, list.height as usize);
    let offset = state.list.offset();
    let selected_index = state.list.selected;
    let nodes = state.nodes();
    let buf = frame.buffer_mut();
    for (row, index) in visible.enumerate() {
        let node = &nodes[index];
        let y = list.y + row as u16;
        let selected = index == selected_index;
        paint_row(
            buf,
            Rect::new(list.x, y, list.width, 1),
            selected,
            theme,
            symbols,
        );
        let row_bg = if selected { theme.bg_soft } else { bg };
        let container = matches!(node.value, Value::Object(_) | Value::Array(_));
        let open = state.expanded.contains(&node.path);
        let fold = if !container {
            " "
        } else if open {
            "▾"
        } else {
            "▸"
        };
        let x = list.x + 2 + (node.depth * 2) as u16;
        let key = format!("{fold} {}: ", sanitize(&node.key));
        let x = put(
            buf,
            x,
            y,
            list.right(),
            &key,
            Style::default().fg(theme.fg_mute).bg(row_bg),
        );
        let color = match node.value {
            Value::String(_) => theme.green,
            Value::Number(_) => theme.blue,
            Value::Bool(_) => theme.yellow,
            Value::Null => theme.fg_faint,
            _ => theme.fg_dim,
        };
        let text = truncate_with_ellipsis(
            &preview(node.value, open),
            list.right().saturating_sub(x) as usize,
            symbols.ellipsis,
        );
        put(
            buf,
            x,
            y,
            list.right(),
            &text,
            Style::default().fg(color).bg(row_bg),
        );
    }
    Scrollbar {
        offset,
        total: len,
        viewport: list.height as usize,
    }
    .render(
        buf,
        Rect::new(inner.right() - 1, list.y, 1, list.height),
        bg,
        theme,
        symbols,
    );
    render_hints(
        buf,
        Rect::new(
            inner.x + 1,
            inner.bottom() - 1,
            inner.width.saturating_sub(1),
            1,
        ),
        &[
            hint("Enter", "fold"),
            hint("←→", "collapse/expand"),
            hint("y", "copy node"),
            hint("Esc", "close"),
        ],
        theme,
        bg,
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nodes_expand_and_copy() {
        let mut state = JsonViewState::new(
            "doc".into(),
            serde_json::json!({"a": {"b": [1, 2]}, "c": "x"}),
        );
        assert_eq!(state.nodes().len(), 3, "root plus its two keys");
        state.list.select(1, 3);
        state.handle_key(&KeyEvent::from(KeyCode::Enter));
        assert_eq!(state.nodes().len(), 4);
        state.list.select(3, 4);
        assert_eq!(state.selected_text().as_deref(), Some("x"));
    }
}
