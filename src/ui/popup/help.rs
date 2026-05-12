use ratatui::{
    layout::Rect,
    style::{Modifier, Style},
    text::{Line, Span, Text},
    widgets::{block::BorderType, Block, Paragraph},
    Frame,
};

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
        let max = self.max_scroll.saturating_sub(1);
        self.scroll = (self.scroll + n).min(max);
    }
}

pub fn render(
    frame: &mut Frame,
    area: Rect,
    state: &mut HelpState,
    theme: &Theme,
    symbols: &Symbols,
) {
    let popup_width = 72u16.min(area.width.saturating_sub(4));
    let popup_height = 24u16.min(area.height.saturating_sub(4));
    let x = area.x + (area.width.saturating_sub(popup_width)) / 2;
    let y = area.y + (area.height.saturating_sub(popup_height)) / 2;
    let popup_area = Rect {
        x,
        y,
        width: popup_width,
        height: popup_height,
    };

    super::paint_popup_surface(frame, popup_area, theme);

    let block = Block::bordered()
        .border_type(BorderType::Rounded)
        .title(format!(" {}  Help  ", symbols.help_icon))
        .border_style(Style::default().fg(theme.accent));

    let inner = block.inner(popup_area);
    let help_text = help_text(symbols);

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

    let visible = inner.height.saturating_sub(2) as usize;
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

fn help_text(symbols: &Symbols) -> String {
    let all_arrows = format!(
        "{}{}{}{}",
        symbols.arrow_up, symbols.arrow_down, symbols.arrow_left, symbols.arrow_right
    );
    format!(
        r#"
Navigation                     Editing
  {all_arrows} / h j k l  Move cell     Enter            Open picker / smart editor
  Home / End       Col start/end Esc              Close popup / focus sidebar
  Ctrl-Home/End    Table bounds  Alt-Enter        New line in text editor
  PgUp / PgDn      Scroll page   Ctrl-Enter       Save staged row
  Ctrl-{up} / Ctrl-{down}  Scroll page   d                Delete row (confirm)
  Mouse wheel      Scroll rows   i                Insert row (staged)
  Shift-wheel      Scroll cols   e                Edit value directly
  Click cell       Focus cell    n                Set NULL
                                 Ctrl-Z           Undo last write

Filtering & Sorting            Tabs & Sidebar
  s                Cycle sort    Ctrl-B           Toggle sidebar
  f                Filter col    Tab              Switch focus
  Shift-F          Clear filters BackTab          Switch focus
  Ctrl-F           Find in table 1-9 / 0          Activate tab 1{range_dash}10
  j (on FK)        Jump to FK    Click tab        Switch / close tab
  Backspace        Jump back     Enter (sidebar)  Open table

Navigation (sidebar)           Command Palette  (Ctrl-P / Ctrl-Shift-P)
  {up}{down} / k j          Move up/down  Export CSV       Save to ~/sqview_export.csv
  {left}{right} / h l          Close/open    Export JSON      Save to ~/sqview_export.json
  Enter            Open table    Export SQL       Save to ~/sqview_export.sql
Misc                            Copy cell        Clip to OSC52 clipboard
  Ctrl-Q           Quit         Copy row JSON    Clip to OSC52 clipboard
  ?                Help (this)  Toggle sidebar   Show/hide schema panel
                                 Toggle read-only Safe inspection mode
                                 Reload schema    Refresh table list
                                 Reset col widths Recalculate layout
                                 Clear filters    Remove all filters
                                 Switch Table     Jump to another table
 "#,
        all_arrows = all_arrows,
        up = symbols.arrow_up,
        down = symbols.arrow_down,
        left = symbols.arrow_left,
        right = symbols.arrow_right,
        range_dash = symbols.range_dash,
    )
}
