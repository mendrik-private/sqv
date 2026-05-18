use ratatui::{
    buffer::Buffer,
    layout::{Constraint, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{block::BorderType, Block, Borders, List, ListItem, ListState},
    Frame,
};

use crate::{db::schema::Schema, symbols::Symbols, theme::Theme};

pub enum SidebarAction {
    OpenTable(String),
    Toggle,
}

pub struct SidebarState {
    pub selected: usize,
    pub tables_expanded: bool,
    pub views_expanded: bool,
    pub indexes_expanded: bool,
    list_state: ListState,
}

impl Default for SidebarState {
    fn default() -> Self {
        let mut list_state = ListState::default();
        list_state.select(Some(0));
        Self {
            selected: 0,
            tables_expanded: true,
            views_expanded: true,
            indexes_expanded: true,
            list_state,
        }
    }
}

impl SidebarState {
    pub fn visible_count(&self, schema: &Schema) -> usize {
        let mut count = 3; // three section headers always visible
        if self.tables_expanded {
            count += schema.tables.len();
        }
        if self.views_expanded {
            count += schema.views.len();
        }
        if self.indexes_expanded {
            count += schema.indexes.len();
        }
        count
    }

    fn views_header_idx(&self, schema: &Schema) -> usize {
        1 + if self.tables_expanded {
            schema.tables.len()
        } else {
            0
        }
    }

    fn indexes_header_idx(&self, schema: &Schema) -> usize {
        self.views_header_idx(schema)
            + 1
            + if self.views_expanded {
                schema.views.len()
            } else {
                0
            }
    }

    pub fn move_down(&mut self, schema: &Schema) {
        let total = self.visible_count(schema);
        self.selected = (self.selected + 1) % total;
        self.list_state.select(Some(self.selected));
    }

    pub fn move_up(&mut self, schema: &Schema) {
        let total = self.visible_count(schema);
        self.selected = self.selected.checked_sub(1).unwrap_or(total - 1);
        self.list_state.select(Some(self.selected));
    }

    pub fn enter(&mut self, schema: &Schema) -> Option<SidebarAction> {
        let views_header = self.views_header_idx(schema);
        let indexes_header = self.indexes_header_idx(schema);

        if self.selected == 0 {
            self.tables_expanded = !self.tables_expanded;
            self.clamp_selection(schema);
            return Some(SidebarAction::Toggle);
        }

        if self.tables_expanded && self.selected > 0 && self.selected < views_header {
            let idx = self.selected - 1;
            return schema
                .tables
                .get(idx)
                .map(|t| SidebarAction::OpenTable(t.name.clone()));
        }

        if self.selected == views_header {
            self.views_expanded = !self.views_expanded;
            self.clamp_selection(schema);
            return Some(SidebarAction::Toggle);
        }

        if self.selected == indexes_header {
            self.indexes_expanded = !self.indexes_expanded;
            self.clamp_selection(schema);
            return Some(SidebarAction::Toggle);
        }

        None
    }

    pub fn collapse_selected_section(&mut self, schema: &Schema) -> bool {
        let views_header = self.views_header_idx(schema);
        let indexes_header = self.indexes_header_idx(schema);

        let changed = if self.selected == 0 {
            let changed = self.tables_expanded;
            self.tables_expanded = false;
            changed
        } else if self.selected == views_header {
            let changed = self.views_expanded;
            self.views_expanded = false;
            changed
        } else if self.selected == indexes_header {
            let changed = self.indexes_expanded;
            self.indexes_expanded = false;
            changed
        } else {
            false
        };

        if changed {
            self.clamp_selection(schema);
        }
        changed
    }

    pub fn expand_selected_section(&mut self, schema: &Schema) -> bool {
        let views_header = self.views_header_idx(schema);
        let indexes_header = self.indexes_header_idx(schema);

        let changed = if self.selected == 0 {
            let changed = !self.tables_expanded;
            self.tables_expanded = true;
            changed
        } else if self.selected == views_header {
            let changed = !self.views_expanded;
            self.views_expanded = true;
            changed
        } else if self.selected == indexes_header {
            let changed = !self.indexes_expanded;
            self.indexes_expanded = true;
            changed
        } else {
            false
        };

        if changed {
            self.clamp_selection(schema);
        }
        changed
    }

    pub fn scroll_down(&mut self, schema: &Schema, viewport_rows: usize, n: usize) {
        self.scroll_by(schema, viewport_rows, n as isize);
    }

    pub fn scroll_up(&mut self, schema: &Schema, viewport_rows: usize, n: usize) {
        self.scroll_by(schema, viewport_rows, -(n as isize));
    }

    pub fn click_at(
        &mut self,
        area: Rect,
        schema: &Schema,
        x: u16,
        y: u16,
    ) -> Option<SidebarAction> {
        let inner = Rect {
            x: area.x.saturating_add(1),
            y: area.y.saturating_add(1),
            width: area.width.saturating_sub(2),
            height: area.height.saturating_sub(2),
        };
        if x < inner.x || x >= inner.x + inner.width || y < inner.y || y >= inner.y + inner.height {
            return None;
        }

        let idx = self.list_state.offset() + (y - inner.y) as usize;
        if idx >= self.visible_count(schema) {
            return None;
        }
        self.selected = idx;
        self.list_state.select(Some(idx));
        self.enter(schema)
    }

    fn clamp_selection(&mut self, schema: &Schema) {
        let total = self.visible_count(schema);
        if self.selected >= total {
            self.selected = total.saturating_sub(1);
        }
        self.list_state.select(Some(self.selected));
    }

    fn scroll_by(&mut self, schema: &Schema, viewport_rows: usize, delta: isize) {
        let total = self.visible_count(schema);
        if total == 0 {
            self.selected = 0;
            self.list_state.select(Some(0));
            *self.list_state.offset_mut() = 0;
            return;
        }

        let viewport_rows = viewport_rows.max(1);
        let max_offset = total.saturating_sub(viewport_rows);
        let current_offset = self.list_state.offset() as isize;
        let new_offset = (current_offset + delta).clamp(0, max_offset as isize) as usize;

        let mut selected = self.selected.min(total - 1);
        if selected < new_offset {
            selected = new_offset;
        } else if selected >= new_offset + viewport_rows {
            selected = new_offset + viewport_rows - 1;
        }

        self.selected = selected;
        self.list_state.select(Some(selected));
        *self.list_state.offset_mut() = new_offset;
    }
}

pub fn render_sidebar(
    frame: &mut Frame,
    area: Rect,
    schema: &Schema,
    state: &mut SidebarState,
    theme: &Theme,
    symbols: &Symbols,
    focused: bool,
) {
    let border_color = if focused { theme.accent } else { theme.line };
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .style(Style::default().bg(theme.bg_soft))
        .border_style(Style::default().fg(border_color))
        .title(Span::styled(
            format!("{} SCHEMA ", symbols.box_horizontal),
            Style::default()
                .fg(if focused { theme.accent } else { theme.fg_mute })
                .add_modifier(Modifier::BOLD),
        ));

    let inner = block.inner(area);
    frame.render_widget(block, area);
    let content = if inner.width > 1 {
        Layout::horizontal([Constraint::Min(0), Constraint::Length(1)]).split(inner)
    } else {
        Layout::horizontal([Constraint::Min(0), Constraint::Length(0)]).split(inner)
    };
    let list_area = content[0];
    let scrollbar_area = content[1];

    let header_style = Style::default()
        .fg(theme.fg_mute)
        .add_modifier(Modifier::BOLD);
    let name_style = Style::default()
        .fg(theme.fg_dim)
        .add_modifier(Modifier::DIM);
    let accent_style = Style::default()
        .fg(theme.accent)
        .bg(theme.bg_raised)
        .add_modifier(Modifier::BOLD);

    clear_area(
        frame.buffer_mut(),
        list_area,
        Style::default().bg(theme.bg_soft),
    );

    let items = build_sidebar_items(schema, state, theme, symbols, header_style, name_style);
    let highlight_symbol = format!("{} ", symbols.selection);
    let list = build_sidebar_list(items, theme, accent_style, &highlight_symbol);

    frame.render_stateful_widget(list, list_area, &mut state.list_state);
    if scrollbar_area.width > 0 {
        render_scrollbar(
            frame.buffer_mut(),
            scrollbar_area,
            state.list_state.offset(),
            state.visible_count(schema),
            list_area.height as usize,
            theme,
            symbols,
        );
    }
}

fn build_sidebar_items(
    schema: &Schema,
    state: &SidebarState,
    theme: &Theme,
    symbols: &Symbols,
    header_style: Style,
    name_style: Style,
) -> Vec<ListItem<'static>> {
    let mut items = Vec::new();

    let tables_arrow = if state.tables_expanded {
        &symbols.folder_open
    } else {
        &symbols.folder_closed
    };
    items.push(ListItem::new(Line::from(Span::styled(
        format!("{} TABLES ({})", tables_arrow, schema.tables.len()),
        header_style,
    ))));
    if state.tables_expanded {
        for table in &schema.tables {
            let icon_span = Span::styled(
                format!(" {} ", symbols.table_icon),
                Style::default().fg(theme.teal),
            );
            let name_span = Span::styled(table.name.clone(), name_style);
            items.push(ListItem::new(Line::from(vec![icon_span, name_span])));
        }
    }

    let views_arrow = if state.views_expanded {
        &symbols.folder_open
    } else {
        &symbols.folder_closed
    };
    items.push(ListItem::new(Line::from(Span::styled(
        format!("{} VIEWS ({})", views_arrow, schema.views.len()),
        header_style,
    ))));
    if state.views_expanded {
        for view in &schema.views {
            let icon_span = Span::styled(
                format!(" {} ", symbols.view_icon),
                Style::default().fg(theme.purple),
            );
            let name_span = Span::styled(view.name.clone(), name_style);
            items.push(ListItem::new(Line::from(vec![icon_span, name_span])));
        }
    }

    let indexes_arrow = if state.indexes_expanded {
        &symbols.folder_open
    } else {
        &symbols.folder_closed
    };
    items.push(ListItem::new(Line::from(Span::styled(
        format!("{} INDEXES ({})", indexes_arrow, schema.indexes.len()),
        header_style,
    ))));
    if state.indexes_expanded {
        for index in &schema.indexes {
            let icon_span = Span::styled(
                format!(" {} ", symbols.index_icon),
                Style::default().fg(theme.yellow),
            );
            let name_span = Span::styled(index.name.clone(), name_style);
            items.push(ListItem::new(Line::from(vec![icon_span, name_span])));
        }
    }

    items
}

fn build_sidebar_list<'a>(
    items: Vec<ListItem<'a>>,
    theme: &Theme,
    accent_style: Style,
    highlight_symbol: &'a str,
) -> List<'a> {
    List::new(items)
        .style(Style::default().bg(theme.bg_soft))
        .highlight_style(accent_style)
        .highlight_symbol(highlight_symbol)
}

fn clear_area(buf: &mut Buffer, area: Rect, style: Style) {
    if area.width == 0 || area.height == 0 {
        return;
    }

    let blank = " ".repeat(area.width as usize);
    for y in area.y..area.y + area.height {
        buf.set_string(area.x, y, &blank, style);
    }
}

fn render_scrollbar(
    buf: &mut Buffer,
    area: Rect,
    offset: usize,
    total: usize,
    viewport: usize,
    theme: &Theme,
    symbols: &Symbols,
) {
    if area.width == 0 || area.height == 0 || total <= viewport || viewport == 0 {
        return;
    }

    let track_height = area.height as usize;
    let thumb_height = ((viewport * track_height) / total).max(1).min(track_height);
    let max_offset = total.saturating_sub(viewport);
    let thumb_top = if max_offset == 0 {
        0
    } else {
        offset
            .min(max_offset)
            .checked_mul(track_height.saturating_sub(thumb_height))
            .and_then(|n| n.checked_div(max_offset))
            .unwrap_or(0)
            .min(track_height.saturating_sub(thumb_height))
    };

    for row in 0..track_height {
        let y = area.y + row as u16;
        let style = if row >= thumb_top && row < thumb_top + thumb_height {
            Style::default().fg(theme.fg_mute).bg(theme.bg_soft)
        } else {
            Style::default().fg(theme.line).bg(theme.bg_soft)
        };
        let glyph = if row >= thumb_top && row < thumb_top + thumb_height {
            symbols.scrollbar_thumb.to_string()
        } else {
            symbols.box_vertical.to_string()
        };
        buf.set_string(area.x, y, glyph, style);
    }
}

#[cfg(test)]
mod tests {
    use ratatui::{buffer::Buffer, widgets::StatefulWidget};

    use super::*;
    use crate::db::schema::{IndexMeta, Schema, TableMeta, ViewMeta};

    fn make_schema() -> Schema {
        Schema {
            tables: vec![TableMeta {
                name: "users".to_string(),
                columns: vec![],
                foreign_keys: vec![],
                indexes: vec![],
            }],
            views: vec![ViewMeta {
                name: "active_users".to_string(),
                sql: None,
            }],
            indexes: vec![IndexMeta {
                name: "users_name_idx".to_string(),
                table: "users".to_string(),
                unique: false,
            }],
        }
    }

    #[test]
    fn sidebar_list_background_fills_full_panel() {
        let theme = Theme::default();
        let symbols = Symbols::default_with_nerd_font(false);
        let state = SidebarState::default();
        let schema = make_schema();
        let header_style = Style::default()
            .fg(theme.fg_mute)
            .add_modifier(Modifier::BOLD);
        let name_style = Style::default()
            .fg(theme.fg_dim)
            .add_modifier(Modifier::DIM);
        let accent_style = Style::default()
            .fg(theme.accent)
            .bg(theme.bg_raised)
            .add_modifier(Modifier::BOLD);
        let items =
            build_sidebar_items(&schema, &state, &theme, &symbols, header_style, name_style);
        let highlight_symbol = format!("{} ", symbols.selection);
        let list = build_sidebar_list(items, &theme, accent_style, &highlight_symbol);
        let area = Rect {
            x: 0,
            y: 0,
            width: 18,
            height: 8,
        };
        let mut buf = Buffer::empty(area);
        let mut list_state = ListState::default();
        list_state.select(Some(0));

        StatefulWidget::render(&list, area, &mut buf, &mut list_state);

        assert_eq!(
            buf[(area.right() - 1, area.top())].style().bg,
            Some(theme.bg_raised)
        );
        assert_eq!(
            buf[(area.right() - 1, area.bottom() - 1)].style().bg,
            Some(theme.bg_soft)
        );
    }

    #[test]
    fn clear_area_replaces_stale_symbols() {
        let theme = Theme::default();
        let area = Rect {
            x: 0,
            y: 0,
            width: 6,
            height: 3,
        };
        let mut buf = Buffer::empty(area);
        for y in area.y..area.bottom() {
            buf.set_string(area.x, y, "stale!", Style::default());
        }

        clear_area(&mut buf, area, Style::default().bg(theme.bg_soft));

        for y in area.y..area.bottom() {
            for x in area.x..area.right() {
                assert_eq!(buf[(x, y)].symbol(), " ");
                assert_eq!(buf[(x, y)].style().bg, Some(theme.bg_soft));
            }
        }
    }
}
