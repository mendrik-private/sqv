use chrono::{
    DateTime, Datelike, Duration, FixedOffset, NaiveDate, NaiveDateTime, TimeZone, Timelike,
};
use ratatui::{
    layout::Rect,
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{block::BorderType, Block, Borders, Paragraph},
    Frame,
};

use crate::{db::types::SqlValue, symbols::Symbols, theme::Theme};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DatetimeFocus {
    Day,
    Month,
    Year,
    Calendar,
    Hour,
    Minute,
    Second,
}

#[allow(dead_code)]
pub struct DatetimePickerState {
    pub table: String,
    pub rowid: i64,
    pub col_name: String,
    pub date: Option<NaiveDate>,
    pub hour: u8,
    pub minute: u8,
    pub second: u8,
    pub view_month: NaiveDate,
    pub original: SqlValue,
    pub focus: DatetimeFocus,
    pub epoch_millis: bool,
    text_format: DatetimeTextFormat,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DatetimeTextFormat {
    SpaceSeparated,
    IsoNaive,
    IsoUtc,
    IsoOffset(i32),
}

impl DatetimePickerState {
    pub fn new(table: String, rowid: i64, col_name: String, original: SqlValue) -> Self {
        let (dt, text_format) = parse_datetime_value(&original)
            .map(|(dt, format)| (Some(dt), format))
            .unwrap_or((None, DatetimeTextFormat::SpaceSeparated));
        let epoch_millis = matches!(&original, SqlValue::Integer(n) if *n > 1_000_000_000_000);
        let today = chrono::Local::now().naive_local();
        let (date, hour, minute, second) = if let Some(d) = dt {
            (
                Some(d.date()),
                d.hour() as u8,
                d.minute() as u8,
                d.second() as u8,
            )
        } else {
            (None, 0u8, 0u8, 0u8)
        };
        let base = date.unwrap_or_else(|| today.date());
        let view_month = first_of_month(base).unwrap_or(base);
        Self {
            table,
            rowid,
            col_name,
            date,
            hour,
            minute,
            second,
            view_month,
            original,
            focus: DatetimeFocus::Day,
            epoch_millis,
            text_format,
        }
    }

    pub fn supports_value(value: &SqlValue) -> bool {
        parse_datetime_value(value).is_some()
    }

    pub fn prev_month(&mut self) {
        self.view_month = shift_month(self.view_month, -1);
        self.sync_date_into_month();
    }

    pub fn next_month(&mut self) {
        self.view_month = shift_month(self.view_month, 1);
        self.sync_date_into_month();
    }

    pub fn move_day(&mut self, delta: i64) {
        let current = self.selected_date();
        let next = current + Duration::days(delta);
        self.date = Some(next);
        self.view_month = first_of_month(next).unwrap_or(self.view_month);
    }

    pub fn focus_next(&mut self) {
        self.focus = match self.focus {
            DatetimeFocus::Day => DatetimeFocus::Month,
            DatetimeFocus::Month => DatetimeFocus::Year,
            DatetimeFocus::Year => DatetimeFocus::Calendar,
            DatetimeFocus::Calendar => DatetimeFocus::Hour,
            DatetimeFocus::Hour => DatetimeFocus::Minute,
            DatetimeFocus::Minute => DatetimeFocus::Second,
            DatetimeFocus::Second => DatetimeFocus::Day,
        };
    }

    pub fn focus_prev(&mut self) {
        self.focus = match self.focus {
            DatetimeFocus::Day => DatetimeFocus::Second,
            DatetimeFocus::Month => DatetimeFocus::Day,
            DatetimeFocus::Year => DatetimeFocus::Month,
            DatetimeFocus::Calendar => DatetimeFocus::Year,
            DatetimeFocus::Hour => DatetimeFocus::Calendar,
            DatetimeFocus::Minute => DatetimeFocus::Hour,
            DatetimeFocus::Second => DatetimeFocus::Minute,
        };
    }

    pub fn adjust_focused(&mut self, delta: i32) {
        match self.focus {
            DatetimeFocus::Day => self.adjust_day(delta),
            DatetimeFocus::Month => self.adjust_month(delta),
            DatetimeFocus::Year => self.adjust_year(delta),
            DatetimeFocus::Calendar => self.move_day(delta as i64),
            DatetimeFocus::Hour => self.adjust_hour(delta),
            DatetimeFocus::Minute => self.adjust_minute(delta),
            DatetimeFocus::Second => self.adjust_second(delta),
        }
    }

    pub fn calendar_left(&mut self) {
        if self.focus == DatetimeFocus::Calendar {
            self.move_day(-1);
        }
    }

    pub fn calendar_right(&mut self) {
        if self.focus == DatetimeFocus::Calendar {
            self.move_day(1);
        }
    }

    pub fn clear(&mut self) {
        self.date = None;
    }

    pub fn as_sql_value(&self) -> SqlValue {
        match self.date {
            Some(d) => {
                let dt = d
                    .and_hms_opt(self.hour as u32, self.minute as u32, self.second as u32)
                    .unwrap_or_else(|| {
                        d.and_hms_opt(0, 0, 0).unwrap_or_else(|| {
                            NaiveDate::from_ymd_opt(1970, 1, 1)
                                .and_then(|d| d.and_hms_opt(0, 0, 0))
                                .unwrap_or_default()
                        })
                    });
                if matches!(self.original, SqlValue::Integer(_)) {
                    let epoch = dt.and_utc().timestamp();
                    if self.epoch_millis {
                        SqlValue::Integer(epoch.saturating_mul(1000))
                    } else {
                        SqlValue::Integer(epoch)
                    }
                } else {
                    SqlValue::Text(format_datetime_text(dt, self.text_format))
                }
            }
            None => SqlValue::Null,
        }
    }

    fn selected_date(&self) -> NaiveDate {
        self.date.unwrap_or(self.view_month)
    }

    fn adjust_day(&mut self, delta: i32) {
        let date = self.selected_date();
        let max_day = days_in_month(date.year(), date.month());
        let day = (date.day() as i32 + delta).clamp(1, max_day as i32) as u32;
        self.date = NaiveDate::from_ymd_opt(date.year(), date.month(), day);
        self.sync_date_into_month();
    }

    fn adjust_month(&mut self, delta: i32) {
        let date = self.selected_date();
        let shifted = shift_month(date, delta);
        self.date = NaiveDate::from_ymd_opt(
            shifted.year(),
            shifted.month(),
            date.day()
                .min(days_in_month(shifted.year(), shifted.month())),
        );
        self.sync_date_into_month();
    }

    fn adjust_year(&mut self, delta: i32) {
        let date = self.selected_date();
        let year = date.year().saturating_add(delta);
        self.date = NaiveDate::from_ymd_opt(
            year,
            date.month(),
            date.day().min(days_in_month(year, date.month())),
        );
        self.sync_date_into_month();
    }

    fn adjust_hour(&mut self, delta: i32) {
        self.hour = wrap_component(self.hour, delta, 24);
    }

    fn adjust_minute(&mut self, delta: i32) {
        self.minute = wrap_component(self.minute, delta, 60);
    }

    fn adjust_second(&mut self, delta: i32) {
        self.second = wrap_component(self.second, delta, 60);
    }

    fn sync_date_into_month(&mut self) {
        if let Some(date) = self.date {
            self.view_month = first_of_month(date).unwrap_or(self.view_month);
        }
    }
}

fn parse_datetime_value(value: &SqlValue) -> Option<(NaiveDateTime, DatetimeTextFormat)> {
    match value {
        SqlValue::Text(text) => parse_datetime_text(text),
        _ => None,
    }
}

fn parse_datetime_text(text: &str) -> Option<(NaiveDateTime, DatetimeTextFormat)> {
    let trimmed = text.trim();

    if let Ok(dt) = DateTime::parse_from_rfc3339(trimmed) {
        let format = if trimmed.ends_with('Z') {
            DatetimeTextFormat::IsoUtc
        } else {
            DatetimeTextFormat::IsoOffset(dt.offset().local_minus_utc())
        };
        return Some((dt.naive_local(), format));
    }

    for (pattern, format) in [
        ("%Y-%m-%dT%H:%M:%S%.f", DatetimeTextFormat::IsoNaive),
        ("%Y-%m-%dT%H:%M:%S", DatetimeTextFormat::IsoNaive),
        ("%Y-%m-%d %H:%M:%S%.f", DatetimeTextFormat::SpaceSeparated),
        ("%Y-%m-%d %H:%M:%S", DatetimeTextFormat::SpaceSeparated),
    ] {
        if let Ok(dt) = NaiveDateTime::parse_from_str(trimmed, pattern) {
            return Some((dt, format));
        }
    }

    None
}

fn format_datetime_text(dt: NaiveDateTime, format: DatetimeTextFormat) -> String {
    match format {
        DatetimeTextFormat::SpaceSeparated => dt.format("%Y-%m-%d %H:%M:%S").to_string(),
        DatetimeTextFormat::IsoNaive => dt.format("%Y-%m-%dT%H:%M:%S").to_string(),
        DatetimeTextFormat::IsoUtc => format!("{}Z", dt.format("%Y-%m-%dT%H:%M:%S")),
        DatetimeTextFormat::IsoOffset(offset_seconds) => FixedOffset::east_opt(offset_seconds)
            .and_then(|offset| offset.from_local_datetime(&dt).single())
            .map(|dt| dt.to_rfc3339())
            .unwrap_or_else(|| dt.format("%Y-%m-%dT%H:%M:%S").to_string()),
    }
}

pub fn render(
    frame: &mut Frame,
    area: Rect,
    state: &DatetimePickerState,
    theme: &Theme,
    symbols: &Symbols,
) {
    let popup_width = 42u16.min(area.width);
    let popup_height = 19u16.min(area.height);
    let x = area.x + (area.width.saturating_sub(popup_width)) / 2;
    let y = area.y + (area.height.saturating_sub(popup_height)) / 2;
    let popup_area = Rect {
        x,
        y,
        width: popup_width,
        height: popup_height,
    };

    super::paint_popup_surface(frame, popup_area, theme);

    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(theme.accent))
        .title(format!(" DateTime: {} ", state.col_name))
        .style(Style::default().bg(theme.bg_raised));

    let inner = block.inner(popup_area);
    frame.render_widget(block, popup_area);

    let selected = state.selected_date();
    let calendar_pad = " ".repeat(calendar_left_padding(inner.width));
    let mut lines = vec![
        Line::from(""),
        render_date_inputs(state, selected, inner.width, theme),
        divider(inner.width, theme, symbols),
        Line::from(""),
        Line::from(vec![
            Span::styled(calendar_pad.clone(), Style::default().bg(theme.bg_raised)),
            Span::styled(
                format!("{:^28}", state.view_month.format("%B %Y")),
                Style::default().fg(theme.accent).bg(theme.bg_raised),
            ),
        ]),
        Line::from(vec![
            Span::styled(calendar_pad, Style::default().bg(theme.bg_raised)),
            Span::styled(
                " Mo Tue Wed Thu Fri Sat Sun",
                Style::default().fg(theme.fg_dim).bg(theme.bg_raised),
            ),
        ]),
    ];
    lines.extend(render_calendar_lines(state, theme, inner.width));
    lines.push(Line::from(""));
    lines.push(divider(inner.width, theme, symbols));
    lines.push(render_time_inputs(state, inner.width, theme));
    lines.push(divider(inner.width, theme, symbols));
    lines.push(Line::from(Span::styled(
        format!(
            " Tab next {} Shift-Tab prev {} PgUp/PgDn month {} Enter ok",
            symbols.separator, symbols.separator, symbols.separator
        ),
        Style::default().fg(theme.fg_faint).bg(theme.bg_raised),
    )));

    frame.render_widget(
        Paragraph::new(lines).style(Style::default().bg(theme.bg_raised)),
        inner,
    );
}

fn render_date_inputs(
    state: &DatetimePickerState,
    selected: NaiveDate,
    area_width: u16,
    theme: &Theme,
) -> Line<'static> {
    centered_line(
        Line::from(vec![
            field_span(
                "Day",
                &format!("{:02}", selected.day()),
                state.focus == DatetimeFocus::Day,
                theme,
            ),
            Span::raw("  "),
            field_span(
                "Month",
                &format!("{:02}", selected.month()),
                state.focus == DatetimeFocus::Month,
                theme,
            ),
            Span::raw("  "),
            field_span(
                "Year",
                &format!("{:04}", selected.year()),
                state.focus == DatetimeFocus::Year,
                theme,
            ),
        ]),
        area_width,
        27,
        theme,
    )
}

fn render_time_inputs(
    state: &DatetimePickerState,
    area_width: u16,
    theme: &Theme,
) -> Line<'static> {
    centered_line(
        Line::from(vec![
            field_span(
                "Hour",
                &format!("{:02}", state.hour),
                state.focus == DatetimeFocus::Hour,
                theme,
            ),
            Span::raw("  "),
            field_span(
                "Minutes",
                &format!("{:02}", state.minute),
                state.focus == DatetimeFocus::Minute,
                theme,
            ),
            Span::raw("  "),
            field_span(
                "Seconds",
                &format!("{:02}", state.second),
                state.focus == DatetimeFocus::Second,
                theme,
            ),
        ]),
        area_width,
        31,
        theme,
    )
}

fn field_span(label: &str, value: &str, focused: bool, theme: &Theme) -> Span<'static> {
    let style = if focused {
        Style::default()
            .fg(theme.bg)
            .bg(theme.accent)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(theme.fg).bg(theme.bg_soft)
    };
    Span::styled(format!("{label}:{value}"), style)
}

fn render_calendar_lines(
    state: &DatetimePickerState,
    theme: &Theme,
    area_width: u16,
) -> Vec<Line<'static>> {
    let first_dow = state.view_month.weekday().num_days_from_monday();
    let days = days_in_month(state.view_month.year(), state.view_month.month());
    let today = chrono::Local::now().date_naive();
    let left_padding = " ".repeat(calendar_left_padding(area_width));

    let mut day = 1u32;
    let mut col = first_dow;
    let mut out = Vec::new();
    while day <= days {
        let mut spans = vec![Span::styled(
            left_padding.clone(),
            Style::default().bg(theme.bg_raised),
        )];
        for week_col in 0..7 {
            if (day == 1 && week_col < col) || day > days {
                spans.push(Span::styled("    ", Style::default().bg(theme.bg_raised)));
            } else {
                let date =
                    NaiveDate::from_ymd_opt(state.view_month.year(), state.view_month.month(), day);
                let style = if state.date == date && state.focus == DatetimeFocus::Calendar {
                    Style::default()
                        .fg(theme.bg)
                        .bg(theme.accent)
                        .add_modifier(Modifier::BOLD)
                } else if state.date == date {
                    Style::default().fg(theme.accent).bg(theme.bg_soft)
                } else if date == Some(today) {
                    Style::default().fg(theme.accent).bg(theme.bg_raised)
                } else {
                    Style::default().fg(theme.fg).bg(theme.bg_raised)
                };
                spans.push(Span::styled(format!("{:>3} ", day), style));
                day += 1;
            }
        }
        col = 0;
        out.push(Line::from(spans));
    }
    out
}

fn calendar_left_padding(area_width: u16) -> usize {
    area_width.saturating_sub(28) as usize / 2
}

fn centered_line(
    content: Line<'static>,
    area_width: u16,
    content_width: usize,
    theme: &Theme,
) -> Line<'static> {
    let pad = area_width.saturating_sub(content_width as u16) as usize / 2;
    if pad == 0 {
        return content;
    }

    let mut spans = Vec::with_capacity(content.spans.len() + 1);
    spans.push(Span::styled(
        " ".repeat(pad),
        Style::default().bg(theme.bg_raised),
    ));
    spans.extend(content.spans);
    Line::from(spans)
}

fn divider(width: u16, theme: &Theme, symbols: &Symbols) -> Line<'static> {
    Line::from(Span::styled(
        symbols.box_horizontal.to_string().repeat(width as usize),
        Style::default().fg(theme.line).bg(theme.bg_raised),
    ))
}

fn wrap_component(value: u8, delta: i32, modulo: i32) -> u8 {
    (value as i32 + delta).rem_euclid(modulo) as u8
}

fn first_of_month(date: NaiveDate) -> Option<NaiveDate> {
    NaiveDate::from_ymd_opt(date.year(), date.month(), 1)
}

fn shift_month(date: NaiveDate, delta: i32) -> NaiveDate {
    let base_month = date.month0() as i32 + delta;
    let year = date.year() + base_month.div_euclid(12);
    let month0 = base_month.rem_euclid(12) as u32;
    let month = month0 + 1;
    let day = date.day().min(days_in_month(year, month));
    NaiveDate::from_ymd_opt(year, month, day).unwrap_or(date)
}

fn days_in_month(year: i32, month: u32) -> u32 {
    let next_month = if month == 12 { 1 } else { month + 1 };
    let next_year = if month == 12 { year + 1 } else { year };
    NaiveDate::from_ymd_opt(next_year, next_month, 1)
        .and_then(|d| d.pred_opt())
        .map_or(28, |d| d.day())
}

#[cfg(test)]
mod tests {
    use super::DatetimePickerState;
    use crate::db::types::SqlValue;

    #[test]
    fn detects_iso_datetime_text_values() {
        assert!(DatetimePickerState::supports_value(&SqlValue::Text(
            "2026-04-24T12:34:56Z".into()
        )));
        assert!(DatetimePickerState::supports_value(&SqlValue::Text(
            "2026-04-24T12:34:56+02:30".into()
        )));
        assert!(DatetimePickerState::supports_value(&SqlValue::Text(
            "2026-04-24 12:34:56".into()
        )));
        assert!(!DatetimePickerState::supports_value(&SqlValue::Text(
            "2026-04-24".into()
        )));
        assert!(!DatetimePickerState::supports_value(&SqlValue::Integer(42)));
    }

    #[test]
    fn preserves_iso_datetime_text_format_on_commit() {
        let state = DatetimePickerState::new(
            "events".into(),
            1,
            "starts_at".into(),
            SqlValue::Text("2026-04-24T12:34:56Z".into()),
        );

        assert_eq!(
            state.as_sql_value(),
            SqlValue::Text("2026-04-24T12:34:56Z".into())
        );
    }
}
