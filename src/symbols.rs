use anyhow::{bail, Context};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone)]
pub struct Symbols {
    pub arrow_up: char,
    pub arrow_down: char,
    pub arrow_left: char,
    pub arrow_right: char,
    pub range_dash: char,
    pub sort_asc: char,
    pub sort_desc: char,
    pub separator: char,
    pub breadcrumb_prefix: char,
    pub breadcrumb_separator: String,
    pub empty_placeholder: char,
    pub missing_placeholder: char,
    pub ellipsis: char,
    pub cursor: char,
    pub active_bar: char,
    pub selection: char,
    pub loading: char,
    pub enter: char,
    pub foreign_key_arrow: char,
    pub bool_true: char,
    pub bool_false: char,
    pub valid: char,
    pub invalid: char,
    pub readonly: char,
    pub dropdown: char,
    pub table_icon: String,
    pub view_icon: String,
    pub index_icon: String,
    pub help_icon: String,
    pub filter_icon: String,
    pub filter_marker: String,
    pub folder_open: String,
    pub folder_closed: String,
    pub pk_icon: String,
    pub fk_icon: String,
    pub tab_shortcuts: [String; 10],
    pub box_horizontal: char,
    pub box_vertical: char,
    pub focus_top_left: char,
    pub focus_top_right: char,
    pub focus_bottom_left: char,
    pub focus_bottom_right: char,
    pub tab_top_left: char,
    pub tab_top_right: char,
    pub tab_join_left: char,
    pub tab_join_right: char,
    pub tab_close: char,
    pub table_rule_header_mid: char,
    pub table_rule_cross_mid: char,
    pub scrollbar_thumb: char,
}

impl Symbols {
    pub fn default_with_nerd_font(nerd_font: bool) -> Self {
        Self {
            arrow_up: '↑',
            arrow_down: '↓',
            arrow_left: '←',
            arrow_right: '→',
            range_dash: '–',
            sort_asc: '▲',
            sort_desc: '▼',
            separator: '·',
            breadcrumb_prefix: '↩',
            breadcrumb_separator: " › ".to_string(),
            empty_placeholder: '—',
            missing_placeholder: '–',
            ellipsis: '…',
            cursor: '▌',
            active_bar: '▌',
            selection: '⏵',
            loading: '•',
            enter: '↵',
            foreign_key_arrow: '→',
            bool_true: '✓',
            bool_false: '·',
            valid: '✓',
            invalid: '✗',
            readonly: '⊘',
            dropdown: '▾',
            table_icon: if nerd_font { "󰓫" } else { "[T]" }.to_string(),
            view_icon: if nerd_font { "󰈈" } else { "[V]" }.to_string(),
            index_icon: if nerd_font { "󰓹" } else { "[I]" }.to_string(),
            help_icon: if nerd_font { "󰋖" } else { "?" }.to_string(),
            filter_icon: if nerd_font { "󰈲" } else { "[f]" }.to_string(),
            filter_marker: "ƒ".to_string(),
            folder_open: "📂".to_string(),
            folder_closed: "📁".to_string(),
            pk_icon: "🔑".to_string(),
            fk_icon: "🔗".to_string(),
            tab_shortcuts: [
                "¹".to_string(),
                "²".to_string(),
                "³".to_string(),
                "⁴".to_string(),
                "⁵".to_string(),
                "⁶".to_string(),
                "⁷".to_string(),
                "⁸".to_string(),
                "⁹".to_string(),
                "⁰".to_string(),
            ],
            box_horizontal: '─',
            box_vertical: '│',
            focus_top_left: '┌',
            focus_top_right: '┐',
            focus_bottom_left: '└',
            focus_bottom_right: '┘',
            tab_top_left: '╭',
            tab_top_right: '╮',
            tab_join_left: '┘',
            tab_join_right: '└',
            tab_close: '×',
            table_rule_header_mid: '┬',
            table_rule_cross_mid: '┼',
            scrollbar_thumb: '█',
        }
    }

    pub fn tab_shortcut(&self, idx: usize) -> Option<&str> {
        self.tab_shortcuts.get(idx).map(String::as_str)
    }

    pub fn inline_separator(&self) -> String {
        format!(" {} ", self.separator)
    }

    pub fn padded_separator(&self) -> String {
        format!("  {}  ", self.separator)
    }

    pub fn segment_separator(&self) -> String {
        format!("  {}  ", self.box_vertical)
    }

    pub fn loading_label(&self, label: &str) -> String {
        format!(" {label}{}", self.ellipsis)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct SymbolOverrides {
    pub arrow_up: Option<String>,
    pub arrow_down: Option<String>,
    pub arrow_left: Option<String>,
    pub arrow_right: Option<String>,
    pub range_dash: Option<String>,
    pub sort_asc: Option<String>,
    pub sort_desc: Option<String>,
    pub separator: Option<String>,
    pub breadcrumb_prefix: Option<String>,
    pub breadcrumb_separator: Option<String>,
    pub empty_placeholder: Option<String>,
    pub missing_placeholder: Option<String>,
    pub ellipsis: Option<String>,
    pub cursor: Option<String>,
    pub active_bar: Option<String>,
    pub selection: Option<String>,
    pub loading: Option<String>,
    pub enter: Option<String>,
    pub foreign_key_arrow: Option<String>,
    pub bool_true: Option<String>,
    pub bool_false: Option<String>,
    pub valid: Option<String>,
    pub invalid: Option<String>,
    pub readonly: Option<String>,
    pub dropdown: Option<String>,
    pub table_icon: Option<String>,
    pub view_icon: Option<String>,
    pub index_icon: Option<String>,
    pub help_icon: Option<String>,
    pub filter_icon: Option<String>,
    pub filter_marker: Option<String>,
    pub folder_open: Option<String>,
    pub folder_closed: Option<String>,
    pub pk_icon: Option<String>,
    pub fk_icon: Option<String>,
    pub tab_shortcuts: Option<Vec<String>>,
    pub box_horizontal: Option<String>,
    pub box_vertical: Option<String>,
    pub focus_top_left: Option<String>,
    pub focus_top_right: Option<String>,
    pub focus_bottom_left: Option<String>,
    pub focus_bottom_right: Option<String>,
    pub tab_top_left: Option<String>,
    pub tab_top_right: Option<String>,
    pub tab_join_left: Option<String>,
    pub tab_join_right: Option<String>,
    pub tab_close: Option<String>,
    pub table_rule_header_mid: Option<String>,
    pub table_rule_cross_mid: Option<String>,
    pub scrollbar_thumb: Option<String>,
}

impl SymbolOverrides {
    pub fn resolve(&self, nerd_font: bool) -> anyhow::Result<Symbols> {
        let mut symbols = Symbols::default_with_nerd_font(nerd_font);

        apply_char_override("symbols.arrow_up", &mut symbols.arrow_up, &self.arrow_up)?;
        apply_char_override(
            "symbols.arrow_down",
            &mut symbols.arrow_down,
            &self.arrow_down,
        )?;
        apply_char_override(
            "symbols.arrow_left",
            &mut symbols.arrow_left,
            &self.arrow_left,
        )?;
        apply_char_override(
            "symbols.arrow_right",
            &mut symbols.arrow_right,
            &self.arrow_right,
        )?;
        apply_char_override(
            "symbols.range_dash",
            &mut symbols.range_dash,
            &self.range_dash,
        )?;
        apply_char_override("symbols.sort_asc", &mut symbols.sort_asc, &self.sort_asc)?;
        apply_char_override("symbols.sort_desc", &mut symbols.sort_desc, &self.sort_desc)?;
        apply_char_override("symbols.separator", &mut symbols.separator, &self.separator)?;
        apply_char_override(
            "symbols.breadcrumb_prefix",
            &mut symbols.breadcrumb_prefix,
            &self.breadcrumb_prefix,
        )?;
        apply_string_override(
            &mut symbols.breadcrumb_separator,
            &self.breadcrumb_separator,
        );
        apply_char_override(
            "symbols.empty_placeholder",
            &mut symbols.empty_placeholder,
            &self.empty_placeholder,
        )?;
        apply_char_override(
            "symbols.missing_placeholder",
            &mut symbols.missing_placeholder,
            &self.missing_placeholder,
        )?;
        apply_char_override("symbols.ellipsis", &mut symbols.ellipsis, &self.ellipsis)?;
        apply_char_override("symbols.cursor", &mut symbols.cursor, &self.cursor)?;
        apply_char_override(
            "symbols.active_bar",
            &mut symbols.active_bar,
            &self.active_bar,
        )?;
        apply_char_override("symbols.selection", &mut symbols.selection, &self.selection)?;
        apply_char_override("symbols.loading", &mut symbols.loading, &self.loading)?;
        apply_char_override("symbols.enter", &mut symbols.enter, &self.enter)?;
        apply_char_override(
            "symbols.foreign_key_arrow",
            &mut symbols.foreign_key_arrow,
            &self.foreign_key_arrow,
        )?;
        apply_char_override("symbols.bool_true", &mut symbols.bool_true, &self.bool_true)?;
        apply_char_override(
            "symbols.bool_false",
            &mut symbols.bool_false,
            &self.bool_false,
        )?;
        apply_char_override("symbols.valid", &mut symbols.valid, &self.valid)?;
        apply_char_override("symbols.invalid", &mut symbols.invalid, &self.invalid)?;
        apply_char_override("symbols.readonly", &mut symbols.readonly, &self.readonly)?;
        apply_char_override("symbols.dropdown", &mut symbols.dropdown, &self.dropdown)?;
        apply_string_override(&mut symbols.table_icon, &self.table_icon);
        apply_string_override(&mut symbols.view_icon, &self.view_icon);
        apply_string_override(&mut symbols.index_icon, &self.index_icon);
        apply_string_override(&mut symbols.help_icon, &self.help_icon);
        apply_string_override(&mut symbols.filter_icon, &self.filter_icon);
        apply_string_override(&mut symbols.filter_marker, &self.filter_marker);
        apply_string_override(&mut symbols.folder_open, &self.folder_open);
        apply_string_override(&mut symbols.folder_closed, &self.folder_closed);
        apply_string_override(&mut symbols.pk_icon, &self.pk_icon);
        apply_string_override(&mut symbols.fk_icon, &self.fk_icon);
        apply_char_override(
            "symbols.box_horizontal",
            &mut symbols.box_horizontal,
            &self.box_horizontal,
        )?;
        apply_char_override(
            "symbols.box_vertical",
            &mut symbols.box_vertical,
            &self.box_vertical,
        )?;
        apply_char_override(
            "symbols.focus_top_left",
            &mut symbols.focus_top_left,
            &self.focus_top_left,
        )?;
        apply_char_override(
            "symbols.focus_top_right",
            &mut symbols.focus_top_right,
            &self.focus_top_right,
        )?;
        apply_char_override(
            "symbols.focus_bottom_left",
            &mut symbols.focus_bottom_left,
            &self.focus_bottom_left,
        )?;
        apply_char_override(
            "symbols.focus_bottom_right",
            &mut symbols.focus_bottom_right,
            &self.focus_bottom_right,
        )?;
        apply_char_override(
            "symbols.tab_top_left",
            &mut symbols.tab_top_left,
            &self.tab_top_left,
        )?;
        apply_char_override(
            "symbols.tab_top_right",
            &mut symbols.tab_top_right,
            &self.tab_top_right,
        )?;
        apply_char_override(
            "symbols.tab_join_left",
            &mut symbols.tab_join_left,
            &self.tab_join_left,
        )?;
        apply_char_override(
            "symbols.tab_join_right",
            &mut symbols.tab_join_right,
            &self.tab_join_right,
        )?;
        apply_char_override("symbols.tab_close", &mut symbols.tab_close, &self.tab_close)?;
        apply_char_override(
            "symbols.table_rule_header_mid",
            &mut symbols.table_rule_header_mid,
            &self.table_rule_header_mid,
        )?;
        apply_char_override(
            "symbols.table_rule_cross_mid",
            &mut symbols.table_rule_cross_mid,
            &self.table_rule_cross_mid,
        )?;
        apply_char_override(
            "symbols.scrollbar_thumb",
            &mut symbols.scrollbar_thumb,
            &self.scrollbar_thumb,
        )?;

        if let Some(overrides) = &self.tab_shortcuts {
            if overrides.len() != 10 {
                bail!(
                    "symbols.tab_shortcuts must contain exactly 10 entries, got {}",
                    overrides.len()
                );
            }
            symbols.tab_shortcuts = overrides
                .clone()
                .try_into()
                .map_err(|_| anyhow::anyhow!("symbols.tab_shortcuts must contain 10 entries"))?;
        }

        Ok(symbols)
    }
}

fn apply_string_override(target: &mut String, value: &Option<String>) {
    if let Some(value) = value {
        *target = value.clone();
    }
}

fn apply_char_override(
    field: &str,
    target: &mut char,
    value: &Option<String>,
) -> anyhow::Result<()> {
    if let Some(value) = value {
        *target = parse_single_char(field, value)?;
    }
    Ok(())
}

fn parse_single_char(field: &str, value: &str) -> anyhow::Result<char> {
    let mut chars = value.chars();
    let ch = chars
        .next()
        .with_context(|| format!("{field} must not be empty"))?;
    if chars.next().is_some() {
        bail!("{field} must be exactly one character");
    }
    Ok(ch)
}
