//! How far a date or datetime value is from now, for the status-bar preview.

use chrono::{Local, NaiveDate, NaiveDateTime};

use super::date_picker::{parse_date, parse_datetime};

/// "yesterday", "3h ago", "in 7d"… for date and datetime text, else `None`.
pub(crate) fn relative_label(value: &str) -> Option<String> {
    relative_label_at(value, Local::now().naive_local())
}

fn relative_label_at(value: &str, now: NaiveDateTime) -> Option<String> {
    if let Some(dt) = parse_datetime(value) {
        return Some(format_relative_datetime(dt, now));
    }
    parse_date(value).map(|date| format_relative_date(date, now.date()))
}

fn format_relative_date(date: NaiveDate, today: NaiveDate) -> String {
    let delta_days = date.signed_duration_since(today).num_days();
    match delta_days {
        0 => "today".to_string(),
        -1 => "yesterday".to_string(),
        1 => "tomorrow".to_string(),
        days if days < 0 => format!("{} ago", compact_days(-days)),
        days => format!("in {}", compact_days(days)),
    }
}

fn format_relative_datetime(dt: NaiveDateTime, now: NaiveDateTime) -> String {
    let delta_seconds = dt.signed_duration_since(now).num_seconds();
    let abs_seconds = delta_seconds.unsigned_abs();

    let label = if abs_seconds < 60 {
        "just now".to_string()
    } else if abs_seconds < 60 * 60 {
        format!("{}m", abs_seconds / 60)
    } else if abs_seconds < 60 * 60 * 24 {
        format!("{}h", abs_seconds / (60 * 60))
    } else if abs_seconds < 60 * 60 * 24 * 30 {
        format!("{}d", abs_seconds / (60 * 60 * 24))
    } else if abs_seconds < 60 * 60 * 24 * 365 {
        format!("{}mo", abs_seconds / (60 * 60 * 24 * 30))
    } else {
        format!("{}y", abs_seconds / (60 * 60 * 24 * 365))
    };

    match delta_seconds.cmp(&0) {
        std::cmp::Ordering::Less => {
            if label == "just now" {
                label
            } else {
                format!("{} ago", label)
            }
        }
        std::cmp::Ordering::Equal => label,
        std::cmp::Ordering::Greater => {
            if label == "just now" {
                label
            } else {
                format!("in {}", label)
            }
        }
    }
}

fn compact_days(days: i64) -> String {
    if days < 30 {
        format!("{days}d")
    } else if days < 365 {
        format!("{}mo", days / 30)
    } else {
        format!("{}y", days / 365)
    }
}

#[cfg(test)]
mod tests {
    use super::relative_label_at;
    use chrono::NaiveDate;

    #[test]
    fn formats_date_values_relatively() {
        let now = NaiveDate::from_ymd_opt(2026, 4, 26)
            .unwrap()
            .and_hms_opt(12, 0, 0)
            .unwrap();

        assert_eq!(
            relative_label_at("2026-04-25", now).as_deref(),
            Some("yesterday")
        );
        assert_eq!(
            relative_label_at("2026-05-03", now).as_deref(),
            Some("in 7d")
        );
    }

    #[test]
    fn formats_datetime_values_relatively() {
        let now = NaiveDate::from_ymd_opt(2026, 4, 26)
            .unwrap()
            .and_hms_opt(12, 0, 0)
            .unwrap();

        assert_eq!(
            relative_label_at("2026-04-26T10:30:00", now).as_deref(),
            Some("1h ago")
        );
        assert_eq!(
            relative_label_at("2026-04-26T15:00:00", now).as_deref(),
            Some("in 3h")
        );
    }

    #[test]
    fn leaves_non_dates_untouched() {
        let now = NaiveDate::from_ymd_opt(2026, 4, 26)
            .unwrap()
            .and_hms_opt(12, 0, 0)
            .unwrap();

        assert_eq!(relative_label_at("Alice", now), None);
    }
}
