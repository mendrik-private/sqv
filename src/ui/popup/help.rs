use ratatui::{
    layout::Rect,
    style::{Modifier, Style},
    text::{Line, Span, Text},
    widgets::{block::BorderType, Block, Paragraph},
    Frame,
};
use unicode_width::UnicodeWidthStr;

use crate::{symbols::Symbols, theme::Theme};

pub struct HelpState {
    pub scroll: usize,
    pub max_scroll: usize,
}

impl HelpState {
    pub fn new() -> Self {
        Self {
            scroll: 0,
            max_scroll: 0,
        }
    }

    pub fn scroll_up(&mut self, n: usize) {
        self.scroll = self.scroll.saturating_sub(n);
    }

    pub fn scroll_down(&mut self, n: usize) {
        self.scroll = (self.scroll + n).min(self.max_scroll);
    }
}

pub fn render(
    frame: &mut Frame,
    area: Rect,
    state: &mut HelpState,
    theme: &Theme,
    symbols: &Symbols,
) {
    let help_text = help_text(symbols);
    let popup_area = popup_area(area, &help_text);

    super::paint_popup_surface(frame, popup_area, theme);

    let block = Block::bordered()
        .border_type(BorderType::Rounded)
        .title(format!(" {}  Help  ", symbols.help_icon))
        .border_style(Style::default().fg(theme.accent));

    let inner = block.inner(popup_area);

    let lines: Vec<Line> = help_text
        .lines()
        .skip(state.scroll)
        .map(|line| {
            if line.contains("Navigation")
                || line.contains("Filtering")
                || line.contains("Misc")
                || line.contains("Tabs")
                || line.contains("Command Palette")
            {
                Line::from(Span::styled(
                    line.trim(),
                    Style::default()
                        .fg(theme.accent)
                        .add_modifier(Modifier::BOLD),
                ))
            } else {
                Line::from(Span::styled(
                    line.to_string(),
                    Style::default().fg(theme.fg),
                ))
            }
        })
        .collect();

    let visible = inner.height.saturating_sub(1) as usize;
    state.max_scroll = help_text.lines().count().saturating_sub(visible);

    let paragraph = Paragraph::new(Text::from(lines)).block(block);

    frame.render_widget(paragraph, popup_area);

    let hint = format!(
        " {} scroll {} / {}",
        symbols.help_icon,
        state.scroll + 1,
        state.max_scroll + 1
    );
    let hint_width = hint.chars().count() as u16;
    if hint_width < inner.width {
        frame.buffer_mut().set_string(
            inner.x + inner.width - hint_width - 1,
            inner.y + inner.height - 1,
            hint,
            Style::default().fg(theme.fg_mute),
        );
    }
}

fn popup_area(area: Rect, help_text: &str) -> Rect {
    let content_width = help_text
        .lines()
        .map(UnicodeWidthStr::width)
        .max()
        .unwrap_or(0) as u16;
    let content_height = help_text.lines().count() as u16;

    let desired_width = content_width.saturating_add(4);
    let desired_height = content_height.saturating_add(4);
    let popup_width = desired_width.min(area.width.saturating_sub(4));
    let popup_height = desired_height.min(area.height.saturating_sub(4));
    let x = area.x + (area.width.saturating_sub(popup_width)) / 2;
    let y = area.y + (area.height.saturating_sub(popup_height)) / 2;

    Rect {
        x,
        y,
        width: popup_width,
        height: popup_height,
    }
}

fn help_text(symbols: &Symbols) -> String {
    let all_arrows = format!(
        "{}{}{}{}",
        symbols.arrow_up, symbols.arrow_down, symbols.arrow_left, symbols.arrow_right
    );
    let sections = vec![
        HelpSection {
            left_header: "Navigation".to_string(),
            right_header: "Editing".to_string(),
            rows: vec![
                help_row(
                    format!("{all_arrows} / h j k l"),
                    "Move cell",
                    "Enter",
                    "Open picker / smart editor",
                ),
                help_row("Home / End", "Col start/end", "Esc", "Clear selection"),
                help_row(
                    "Ctrl-Home/End",
                    "Table bounds",
                    "Alt-Enter",
                    "Text-editor newline",
                ),
                help_row("PgUp / PgDn", "Scroll page", "Alt-Enter", "Save staged row"),
                help_row(
                    format!("Shift-{} / Shift-{}", symbols.arrow_up, symbols.arrow_down),
                    "Select rows left behind",
                    "Delete / d",
                    "Delete selected row(s)",
                ),
                help_row(
                    format!("Ctrl-{} / Ctrl-{}", symbols.arrow_up, symbols.arrow_down),
                    "Scroll page",
                    "Ctrl-A",
                    "Select all rows",
                ),
                help_row("Mouse wheel", "Scroll rows", "Ins / i", "Insert row below"),
                help_row("Shift-wheel", "Scroll cols", "e", "Edit value directly"),
                help_row("Click gutter", "Select row", "n", "Set NULL"),
                help_row(
                    "Ctrl-click gutter",
                    "Toggle row",
                    "Ctrl-Z",
                    "Undo last write",
                ),
                help_row("Click cell", "Focus cell", "", ""),
            ],
        },
        HelpSection {
            left_header: "Filtering & Sorting".to_string(),
            right_header: "Tabs & Sidebar".to_string(),
            rows: vec![
                help_row("s", "Cycle sort", "Ctrl-B", "Toggle sidebar"),
                help_row("f", "Filter col", "Tab", "Switch focus"),
                help_row("Shift-F", "Clear filters", "BackTab", "Switch focus"),
                help_row(
                    "Ctrl-F",
                    "Find in table",
                    "1-9 / 0",
                    format!("Activate tab 1{}10", symbols.range_dash),
                ),
                help_row("j (on FK)", "Jump to FK", "Click tab", "Switch / close tab"),
                help_row("Backspace", "Jump back", "Ctrl-W", "Close current tab"),
                help_row("Enter (sidebar)", "Open table", "", ""),
            ],
        },
        HelpSection {
            left_header: "Navigation (sidebar)".to_string(),
            right_header: "Command Palette  (Ctrl-P / Ctrl-Shift-P)".to_string(),
            rows: vec![
                help_row(
                    format!("{}{} / k j", symbols.arrow_up, symbols.arrow_down),
                    "Move up/down",
                    "Export CSV",
                    "Save to ~/sqview_export.csv",
                ),
                help_row(
                    format!("{}{} / h l", symbols.arrow_left, symbols.arrow_right),
                    "Close/open",
                    "Export JSON",
                    "Save to ~/sqview_export.json",
                ),
                help_row(
                    "Enter",
                    "Open table",
                    "Export SQL",
                    "Save to ~/sqview_export.sql",
                ),
            ],
        },
        HelpSection {
            left_header: "Misc".to_string(),
            right_header: String::new(),
            rows: vec![
                help_row("Ctrl-Q", "Quit", "y", "Copy cell to clipboard"),
                help_row(
                    "Ctrl-H / ?",
                    "Help (this)",
                    "Y",
                    "Copy row JSON to clipboard",
                ),
                help_row("", "", "Toggle sidebar", "Show/hide schema panel"),
                help_row("", "", "Toggle read-only", "Safe inspection mode"),
                help_row("", "", "Reload schema", "Refresh table list"),
                help_row("", "", "Reset col widths", "Recalculate layout"),
                help_row("", "", "Clear filters", "Remove all filters"),
                help_row("", "", "Switch Table", "Jump to another table"),
            ],
        },
    ];

    render_help_sections(&sections)
}

struct HelpSection {
    left_header: String,
    right_header: String,
    rows: Vec<HelpRow>,
}

struct HelpRow {
    left_key: String,
    left_desc: String,
    right_key: String,
    right_desc: String,
}

fn help_row(
    left_key: impl Into<String>,
    left_desc: impl Into<String>,
    right_key: impl Into<String>,
    right_desc: impl Into<String>,
) -> HelpRow {
    HelpRow {
        left_key: left_key.into(),
        left_desc: left_desc.into(),
        right_key: right_key.into(),
        right_desc: right_desc.into(),
    }
}

fn render_help_sections(sections: &[HelpSection]) -> String {
    let left_key_width = sections
        .iter()
        .flat_map(|section| section.rows.iter())
        .map(|row| UnicodeWidthStr::width(row.left_key.as_str()))
        .max()
        .unwrap_or(0);
    let left_desc_width = sections
        .iter()
        .flat_map(|section| section.rows.iter())
        .map(|row| UnicodeWidthStr::width(row.left_desc.as_str()))
        .max()
        .unwrap_or(0);
    let right_key_width = sections
        .iter()
        .flat_map(|section| section.rows.iter())
        .map(|row| UnicodeWidthStr::width(row.right_key.as_str()))
        .max()
        .unwrap_or(0);
    let left_panel_width = 2 + left_key_width + 2 + left_desc_width;

    let mut lines = Vec::new();
    for (section_idx, section) in sections.iter().enumerate() {
        if section_idx > 0 {
            lines.push(String::new());
        }
        lines.push(section_header(
            &section.left_header,
            &section.right_header,
            left_panel_width,
        ));
        lines.extend(
            section
                .rows
                .iter()
                .map(|row| render_help_row(row, left_key_width, left_desc_width, right_key_width)),
        );
    }

    lines.join("\n")
}

fn section_header(left: &str, right: &str, left_panel_width: usize) -> String {
    if right.is_empty() {
        left.to_string()
    } else {
        format!("{}  {}", pad_display_width(left, left_panel_width), right)
    }
}

fn render_help_row(
    row: &HelpRow,
    left_key_width: usize,
    left_desc_width: usize,
    right_key_width: usize,
) -> String {
    format!(
        "  {}  {}  {}  {}",
        pad_display_width(&row.left_key, left_key_width),
        pad_display_width(&row.left_desc, left_desc_width),
        pad_display_width(&row.right_key, right_key_width),
        row.right_desc
    )
}

fn pad_display_width(text: &str, width: usize) -> String {
    let display_width = UnicodeWidthStr::width(text);
    let padding = width.saturating_sub(display_width);
    format!("{text}{}", " ".repeat(padding))
}

#[cfg(test)]
mod tests {
    use ratatui::layout::Rect;
    use unicode_width::UnicodeWidthStr;

    use super::{help_text, popup_area, HelpState};
    use crate::symbols::Symbols;

    #[test]
    fn help_popup_grows_to_fit_content_when_space_allows() {
        let symbols = Symbols::default_with_nerd_font(false);
        let help = help_text(&symbols);
        let area = popup_area(
            Rect {
                x: 0,
                y: 0,
                width: 120,
                height: 50,
            },
            &help,
        );
        let max_line_width = help.lines().map(UnicodeWidthStr::width).max().unwrap_or(0) as u16;
        let line_count = help.lines().count() as u16;

        assert!(area.width >= max_line_width + 2);
        assert!(area.height >= line_count + 3);
    }

    #[test]
    fn help_popup_respects_available_viewport() {
        let symbols = Symbols::default_with_nerd_font(false);
        let help = help_text(&symbols);
        let area = popup_area(
            Rect {
                x: 0,
                y: 0,
                width: 80,
                height: 20,
            },
            &help,
        );

        assert_eq!(area.width, 76);
        assert_eq!(area.height, 16);
    }

    #[test]
    fn help_state_scroll_down_allows_reaching_last_page() {
        let mut state = HelpState::new();
        state.max_scroll = 5;

        state.scroll_down(10);

        assert_eq!(state.scroll, 5);
    }

    #[test]
    fn help_text_uses_uniform_column_starts() {
        let symbols = Symbols::default_with_nerd_font(false);
        let help = help_text(&symbols);
        let move_line = help
            .lines()
            .find(|line| line.contains("Move up/down"))
            .expect("move line");
        let open_line = help
            .lines()
            .find(|line| line.contains("Open table") && line.contains("Export SQL"))
            .expect("open line");
        let quit_line = help
            .lines()
            .find(|line| line.contains("Ctrl-Q") && line.contains("Copy cell"))
            .expect("quit line");
        let help_line = help
            .lines()
            .find(|line| line.contains("Help (this)") && line.contains("Copy row JSON"))
            .expect("help line");

        let left_desc_start = |line: &str, needle: &str| {
            let idx = line.find(needle).expect("left desc");
            UnicodeWidthStr::width(&line[..idx])
        };
        let right_key_start = |line: &str, needle: &str| {
            let idx = line.find(needle).expect("right key");
            UnicodeWidthStr::width(&line[..idx])
        };

        assert_eq!(
            left_desc_start(move_line, "Move up/down"),
            left_desc_start(open_line, "Open table")
        );
        assert_eq!(
            left_desc_start(open_line, "Open table"),
            left_desc_start(quit_line, "Quit")
        );
        assert_eq!(
            right_key_start(move_line, "Export CSV"),
            right_key_start(open_line, "Export SQL")
        );
        assert_eq!(
            right_key_start(open_line, "Export SQL"),
            right_key_start(help_line, "Y")
        );
    }

    #[test]
    fn help_text_lists_new_selection_and_tab_shortcuts() {
        let symbols = Symbols::default_with_nerd_font(false);
        let help = help_text(&symbols);

        assert!(help.contains("Select rows left behind"));
        assert!(help.contains("Delete / d"));
        assert!(help.contains("Ctrl-A"));
        assert!(help.contains("Ctrl-W"));
        assert!(help.contains("Ctrl-click gutter"));
    }
}
