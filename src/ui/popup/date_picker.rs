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
pub enum DateFocus {
    Day,
    Month,
    Year,
    Calendar,
    Hour,
    Minute,
    Second,
}

/// The representation the edited value is written back in, matching the
/// column's existing value so a commit never changes its storage format.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ValueFormat {
    Date,
    Datetime(DatetimeTextFormat),
    EpochSeconds,
    EpochMillis,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DatetimeTextFormat {
    SpaceSeparated,
    IsoNaive,
    IsoUtc,
    IsoOffset(i32),
}

/// Calendar editor for date columns, and for datetime columns with an extra
/// hour/minute/second row.
pub struct DatePickerState {
    pub table: String,
    pub rowid: i64,
    pub col_name: String,
    pub date: Option<NaiveDate>,
    pub hour: u8,
    pub minute: u8,
    pub second: u8,
    pub view_month: NaiveDate,
    pub original: SqlValue,
    pub focus: DateFocus,
    format: ValueFormat,
}

impl DatePickerState {
    pub fn date(table: String, rowid: i64, col_name: String, original: SqlValue) -> Self {
        let date = parse_date_value(&original);
        Self::build(
            table,
            rowid,
            col_name,
            original,
            date.and_then(|d| d.and_hms_opt(0, 0, 0)),
            ValueFormat::Date,
        )
    }

    pub fn datetime(table: String, rowid: i64, col_name: String, original: SqlValue) -> Self {
        let (value, format) = match &original {
            SqlValue::Integer(n) if *n > 1_000_000_000_000 => (
                DateTime::from_timestamp_millis(*n).map(|dt| dt.naive_utc()),
                ValueFormat::EpochMillis,
            ),
            SqlValue::Integer(n) => (
                DateTime::from_timestamp(*n, 0).map(|dt| dt.naive_utc()),
                ValueFormat::EpochSeconds,
            ),
            _ => match parse_datetime_value(&original) {
                Some((dt, format)) => (Some(dt), ValueFormat::Datetime(format)),
                None => (
                    None,
                    ValueFormat::Datetime(DatetimeTextFormat::SpaceSeparated),
                ),
            },
        };
        Self::build(table, rowid, col_name, original, value, format)
    }

    fn build(
        table: String,
        rowid: i64,
        col_name: String,
        original: SqlValue,
        value: Option<NaiveDateTime>,
        format: ValueFormat,
    ) -> Self {
        let today = chrono::Local::now().date_naive();
        let date = value.map(|dt| dt.date());
        let base = date.unwrap_or(today);
        Self {
            table,
            rowid,
            col_name,
            date,
            hour: value.map_or(0, |dt| dt.hour() as u8),
            minute: value.map_or(0, |dt| dt.minute() as u8),
            second: value.map_or(0, |dt| dt.second() as u8),
            view_month: first_of_month(base).unwrap_or(base),
            original,
            focus: DateFocus::Day,
            format,
        }
    }

    pub fn supports_date(value: &SqlValue) -> bool {
        parse_date_value(value).is_some()
    }

    pub fn supports_datetime(value: &SqlValue) -> bool {
        parse_datetime_value(value).is_some()
    }

    pub fn has_time(&self) -> bool {
        self.format != ValueFormat::Date
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
        let next = self.selected_date() + Duration::days(delta);
        self.date = Some(next);
        self.view_month = first_of_month(next).unwrap_or(self.view_month);
    }

    pub fn clear(&mut self) {
        self.date = None;
    }

    pub fn focus_next(&mut self) {
        self.focus = match self.focus {
            DateFocus::Day => DateFocus::Month,
            DateFocus::Month => DateFocus::Year,
            DateFocus::Year => DateFocus::Calendar,
            DateFocus::Calendar if self.has_time() => DateFocus::Hour,
            DateFocus::Calendar => DateFocus::Day,
            DateFocus::Hour => DateFocus::Minute,
            DateFocus::Minute => DateFocus::Second,
            DateFocus::Second => DateFocus::Day,
        };
    }

    pub fn focus_prev(&mut self) {
        self.focus = match self.focus {
            DateFocus::Day if self.has_time() => DateFocus::Second,
            DateFocus::Day => DateFocus::Calendar,
            DateFocus::Month => DateFocus::Day,
            DateFocus::Year => DateFocus::Month,
            DateFocus::Calendar => DateFocus::Year,
            DateFocus::Hour => DateFocus::Calendar,
            DateFocus::Minute => DateFocus::Hour,
            DateFocus::Second => DateFocus::Minute,
        };
    }

    pub fn adjust_focused(&mut self, delta: i32) {
        match self.focus {
            DateFocus::Day => self.adjust_day(delta),
            DateFocus::Month => self.adjust_month(delta),
            DateFocus::Year => self.adjust_year(delta),
            DateFocus::Calendar => self.move_day(delta as i64),
            DateFocus::Hour => self.hour = wrap_component(self.hour, delta, 24),
            DateFocus::Minute => self.minute = wrap_component(self.minute, delta, 60),
            DateFocus::Second => self.second = wrap_component(self.second, delta, 60),
        }
    }

    pub fn as_sql_value(&self) -> SqlValue {
        let Some(date) = self.date else {
            return SqlValue::Null;
        };
        let dt = date
            .and_hms_opt(self.hour.into(), self.minute.into(), self.second.into())
            .unwrap_or_else(|| date.and_time(chrono::NaiveTime::MIN));
        match self.format {
            ValueFormat::Date => SqlValue::Text(date.format("%Y-%m-%d").to_string()),
            ValueFormat::Datetime(format) => SqlValue::Text(format_datetime_text(dt, format)),
            ValueFormat::EpochSeconds => SqlValue::Integer(dt.and_utc().timestamp()),
            ValueFormat::EpochMillis => SqlValue::Integer(dt.and_utc().timestamp_millis()),
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
        self.date = Some(shift_month(self.selected_date(), delta));
        self.sync_date_into_month();
    }

    fn adjust_year(&mut self, delta: i32) {
        self.date = Some(shift_month(self.selected_date(), delta.saturating_mul(12)));
        self.sync_date_into_month();
    }

    fn sync_date_into_month(&mut self) {
        if let Some(date) = self.date {
            self.view_month = first_of_month(date).unwrap_or(self.view_month);
        }
    }
}

fn parse_date_value(value: &SqlValue) -> Option<NaiveDate> {
    match value {
        SqlValue::Text(text) => parse_date(text),
        _ => None,
    }
}

/// Parses the text form the date editor understands.
pub(crate) fn parse_date(text: &str) -> Option<NaiveDate> {
    NaiveDate::parse_from_str(text.trim(), "%Y-%m-%d").ok()
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

/// Parses the text forms the datetime editor understands.
pub(crate) fn parse_datetime(text: &str) -> Option<NaiveDateTime> {
    parse_datetime_text(text).map(|(dt, _)| dt)
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
    state: &DatePickerState,
    theme: &Theme,
    symbols: &Symbols,
) {
    let (popup_width, popup_height, title) = if state.has_time() {
        (42u16, 19u16, "DateTime")
    } else {
        (34, 16, "Date")
    };
    let popup_area = super::centered_rect(area, popup_width, popup_height);

    super::paint_popup_surface(frame, popup_area, theme);

    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(theme.accent))
        .title(format!(" {title}: {} ", state.col_name))
        .style(Style::default().bg(theme.bg_raised));

    let inner = block.inner(popup_area);
    frame.render_widget(block, popup_area);

    let selected = state.selected_date();
    let calendar_pad = " ".repeat(calendar_left_padding(inner.width));
    let mut lines = vec![
        Line::from(""),
        render_fields(
            [
                ("Day", format!("{:02}", selected.day()), DateFocus::Day),
                (
                    "Month",
                    format!("{:02}", selected.month()),
                    DateFocus::Month,
                ),
                ("Year", format!("{:04}", selected.year()), DateFocus::Year),
            ],
            state.focus,
            inner.width,
            theme,
        ),
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
    if state.has_time() {
        lines.push(render_fields(
            [
                ("Hour", format!("{:02}", state.hour), DateFocus::Hour),
                ("Minutes", format!("{:02}", state.minute), DateFocus::Minute),
                ("Seconds", format!("{:02}", state.second), DateFocus::Second),
            ],
            state.focus,
            inner.width,
            theme,
        ));
        lines.push(divider(inner.width, theme, symbols));
    }
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

/// A centered row of `label:value` fields, highlighting the focused one.
fn render_fields(
    fields: [(&str, String, DateFocus); 3],
    focus: DateFocus,
    area_width: u16,
    theme: &Theme,
) -> Line<'static> {
    let mut spans = Vec::new();
    for (index, (label, value, field)) in fields.into_iter().enumerate() {
        if index > 0 {
            spans.push(Span::raw("  "));
        }
        let style = if focus == field {
            Style::default()
                .fg(theme.bg)
                .bg(theme.accent)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(theme.fg).bg(theme.bg_soft)
        };
        spans.push(Span::styled(format!("{label}:{value}"), style));
    }
    let content_width = spans.iter().map(|span| span.width()).sum::<usize>();
    let pad = area_width.saturating_sub(content_width as u16) as usize / 2;
    if pad > 0 {
        spans.insert(
            0,
            Span::styled(" ".repeat(pad), Style::default().bg(theme.bg_raised)),
        );
    }
    Line::from(spans)
}

fn render_calendar_lines(
    state: &DatePickerState,
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
                let style = if state.date == date && state.focus == DateFocus::Calendar {
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

/// Moves by whole months, clamping the day to the target month's length.
fn shift_month(date: NaiveDate, delta: i32) -> NaiveDate {
    let base_month = date.month0() as i32 + delta;
    let year = date.year() + base_month.div_euclid(12);
    let month = base_month.rem_euclid(12) as u32 + 1;
    let day = date.day().min(days_in_month(year, month));
    NaiveDate::from_ymd_opt(year, month, day).unwrap_or(date)
}

fn days_in_month(year: i32, month: u32) -> u32 {
    let (next_year, next_month) = if month == 12 {
        (year + 1, 1)
    } else {
        (year, month + 1)
    };
    NaiveDate::from_ymd_opt(next_year, next_month, 1)
        .and_then(|d| d.pred_opt())
        .map_or(28, |d| d.day())
}

#[cfg(test)]
mod tests {
    use super::DatePickerState;
    use crate::db::types::SqlValue;

    #[test]
    fn detects_iso_date_text_values() {
        assert!(DatePickerState::supports_date(&SqlValue::Text(
            "2026-04-24".into()
        )));
        assert!(!DatePickerState::supports_date(&SqlValue::Text(
            "2026-04-24T12:34:56Z".into()
        )));
    }

    #[test]
    fn detects_iso_datetime_text_values() {
        assert!(DatePickerState::supports_datetime(&SqlValue::Text(
            "2026-04-24T12:34:56Z".into()
        )));
        assert!(DatePickerState::supports_datetime(&SqlValue::Text(
            "2026-04-24T12:34:56+02:30".into()
        )));
        assert!(DatePickerState::supports_datetime(&SqlValue::Text(
            "2026-04-24 12:34:56".into()
        )));
        assert!(!DatePickerState::supports_datetime(&SqlValue::Text(
            "2026-04-24".into()
        )));
        assert!(!DatePickerState::supports_datetime(&SqlValue::Integer(42)));
    }

    #[test]
    fn preserves_iso_datetime_text_format_on_commit() {
        let state = DatePickerState::datetime(
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

    #[test]
    fn epoch_values_open_on_their_date_and_round_trip() {
        for original in [
            SqlValue::Integer(1_777_034_096),
            SqlValue::Integer(1_777_034_096_000),
        ] {
            let state = DatePickerState::datetime(
                "events".into(),
                1,
                "created_at".into(),
                original.clone(),
            );

            assert!(state.date.is_some());
            assert_eq!(state.as_sql_value(), original);
        }
    }

    #[test]
    fn date_values_commit_without_time() {
        let mut state = DatePickerState::date(
            "events".into(),
            1,
            "day".into(),
            SqlValue::Text("2024-01-31".into()),
        );
        state.adjust_focused(1);
        state.focus_next();
        state.adjust_focused(1);

        assert_eq!(state.as_sql_value(), SqlValue::Text("2024-02-29".into()));
    }
}
