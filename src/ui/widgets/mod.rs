//! Building blocks shared by every panel and popup.

pub mod cell;
pub mod frame;
pub mod hints;
pub mod input;
pub mod list;
pub mod scrollbar;
pub mod state;
pub mod table;
pub mod text;

use std::cmp::Reverse;

use fuzzy_matcher::{skim::SkimMatcherV2, FuzzyMatcher};
use ratatui::{style::Style, text::Span};

use text::{char_width, sanitize, text_width};

/// A fuzzy-filtered entry: the item, its label and the matched char indices.
pub(crate) type FuzzyMatch<T> = (T, String, Vec<usize>);

/// Items whose label fuzzy-matches `query`, best match first; every item in its
/// original order when the query is empty.
pub(crate) fn fuzzy_filter<T>(
    items: impl IntoIterator<Item = (T, String)>,
    query: &str,
) -> Vec<FuzzyMatch<T>> {
    if query.is_empty() {
        return items
            .into_iter()
            .map(|(item, label)| (item, label, Vec::new()))
            .collect();
    }
    let matcher = SkimMatcherV2::default();
    let mut scored = items
        .into_iter()
        .filter_map(|(item, label)| {
            let (score, matched) = matcher.fuzzy_indices(&label, query)?;
            Some((Reverse(score), (item, label, matched)))
        })
        .collect::<Vec<_>>();
    scored.sort_by_key(|(score, _)| *score);
    scored.into_iter().map(|(_, entry)| entry).collect()
}

/// One span per char of `value`, styled as matched where `is_matched(char index)`
/// holds, cut to `max_width` cells with a trailing `...` when it does not fit.
pub(crate) fn highlighted_spans(
    value: &str,
    is_matched: impl Fn(usize) -> bool,
    max_width: usize,
    base_style: Style,
    matched_style: Style,
) -> Vec<Span<'static>> {
    if max_width == 0 {
        return Vec::new();
    }
    let value = sanitize(value);
    let needs_ellipsis = text_width(&value) > max_width;
    let content_limit = if needs_ellipsis {
        max_width - 1
    } else {
        max_width
    };

    let mut spans = Vec::new();
    let mut used_width = 0usize;
    for (idx, ch) in value.chars().enumerate() {
        let ch_width = char_width(ch);
        if used_width + ch_width > content_limit {
            break;
        }
        let style = if is_matched(idx) {
            matched_style
        } else {
            base_style
        };
        spans.push(Span::styled(ch.to_string(), style));
        used_width += ch_width;
    }
    if needs_ellipsis {
        spans.push(Span::styled("…", base_style));
    }
    spans
}

#[cfg(test)]
mod tests {
    use ratatui::style::Style;

    use super::highlighted_spans;

    fn rendered(value: &str, max_width: usize) -> String {
        highlighted_spans(
            value,
            |_| false,
            max_width,
            Style::default(),
            Style::default(),
        )
        .into_iter()
        .map(|span| span.content)
        .collect()
    }

    #[test]
    fn truncates_long_values_with_ellipsis() {
        assert_eq!(
            rendered("Embraer - Empresa Brasileira de Aeronáutica S.A.", 12),
            "Embraer - E…"
        );
    }

    #[test]
    fn leaves_short_values_untouched() {
        assert_eq!(rendered("Apple Inc.", 20), "Apple Inc.");
    }
}
