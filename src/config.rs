use anyhow::Context;
use serde::{Deserialize, Serialize};

use crate::{
    symbols::{SymbolOverrides, Symbols},
    theme::{Theme, ThemeOverrides},
};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub nerd_font: bool,
    pub theme: ThemeOverrides,
    pub symbols: SymbolOverrides,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            nerd_font: true,
            theme: ThemeOverrides::default(),
            symbols: SymbolOverrides::default(),
        }
    }
}

impl Config {
    pub fn load() -> anyhow::Result<Self> {
        Self::ensure_current_config_file()?;

        if let Some(path) = crate::app_dirs::config_file() {
            let content = std::fs::read_to_string(&path)
                .with_context(|| format!("Failed to read config file {}", path.display()))?;
            let config: Self = toml::from_str(&content)
                .with_context(|| format!("Failed to parse config file {}", path.display()))?;
            config.resolve_theme()?;
            config.resolve_symbols()?;
            return Ok(config);
        }

        Ok(Self::default())
    }

    pub fn resolve_theme(&self) -> anyhow::Result<Theme> {
        self.theme.resolve()
    }

    pub fn resolve_symbols(&self) -> anyhow::Result<Symbols> {
        self.symbols.resolve(self.nerd_font)
    }

    pub fn default_toml() -> String {
        let theme = Theme::default();
        let symbols = Symbols::default_with_nerd_font(true);

        format!(
            concat!(
                "nerd_font = true\n\n",
                "[theme]\n",
                "bg = {bg}\n",
                "bg_soft = {bg_soft}\n",
                "bg_raised = {bg_raised}\n",
                "line = {line}\n",
                "line_soft = {line_soft}\n",
                "fg = {fg}\n",
                "fg_dim = {fg_dim}\n",
                "fg_mute = {fg_mute}\n",
                "fg_faint = {fg_faint}\n",
                "accent = {accent}\n",
                "red = {red}\n",
                "yellow = {yellow}\n",
                "green = {green}\n",
                "teal = {teal}\n",
                "blue = {blue}\n",
                "purple = {purple}\n",
                "pink = {pink}\n\n",
                "[symbols]\n",
                "arrow_up = {arrow_up}\n",
                "arrow_down = {arrow_down}\n",
                "arrow_left = {arrow_left}\n",
                "arrow_right = {arrow_right}\n",
                "range_dash = {range_dash}\n",
                "sort_asc = {sort_asc}\n",
                "sort_desc = {sort_desc}\n",
                "separator = {separator}\n",
                "breadcrumb_prefix = {breadcrumb_prefix}\n",
                "breadcrumb_separator = {breadcrumb_separator}\n",
                "empty_placeholder = {empty_placeholder}\n",
                "missing_placeholder = {missing_placeholder}\n",
                "ellipsis = {ellipsis}\n",
                "cursor = {cursor}\n",
                "active_bar = {active_bar}\n",
                "selection = {selection}\n",
                "loading = {loading}\n",
                "enter = {enter}\n",
                "foreign_key_arrow = {foreign_key_arrow}\n",
                "bool_true = {bool_true}\n",
                "bool_false = {bool_false}\n",
                "valid = {valid}\n",
                "invalid = {invalid}\n",
                "readonly = {readonly}\n",
                "dropdown = {dropdown}\n",
                "table_icon = {table_icon}\n",
                "view_icon = {view_icon}\n",
                "index_icon = {index_icon}\n",
                "help_icon = {help_icon}\n",
                "filter_icon = {filter_icon}\n",
                "filter_marker = {filter_marker}\n",
                "folder_open = {folder_open}\n",
                "folder_closed = {folder_closed}\n",
                "pk_icon = {pk_icon}\n",
                "fk_icon = {fk_icon}\n",
                "tab_shortcuts = [{tab_shortcuts}]\n",
                "box_horizontal = {box_horizontal}\n",
                "box_vertical = {box_vertical}\n",
                "focus_top_left = {focus_top_left}\n",
                "focus_top_right = {focus_top_right}\n",
                "focus_bottom_left = {focus_bottom_left}\n",
                "focus_bottom_right = {focus_bottom_right}\n",
                "tab_top_left = {tab_top_left}\n",
                "tab_top_right = {tab_top_right}\n",
                "tab_join_left = {tab_join_left}\n",
                "tab_join_right = {tab_join_right}\n",
                "tab_close = {tab_close}\n",
                "table_rule_header_mid = {table_rule_header_mid}\n",
                "table_rule_cross_mid = {table_rule_cross_mid}\n",
                "scrollbar_thumb = {scrollbar_thumb}\n",
            ),
            bg = toml_string(&color_to_hex(theme.bg)),
            bg_soft = toml_string(&color_to_hex(theme.bg_soft)),
            bg_raised = toml_string(&color_to_hex(theme.bg_raised)),
            line = toml_string(&color_to_hex(theme.line)),
            line_soft = toml_string(&color_to_hex(theme.line_soft)),
            fg = toml_string(&color_to_hex(theme.fg)),
            fg_dim = toml_string(&color_to_hex(theme.fg_dim)),
            fg_mute = toml_string(&color_to_hex(theme.fg_mute)),
            fg_faint = toml_string(&color_to_hex(theme.fg_faint)),
            accent = toml_string(&color_to_hex(theme.accent)),
            red = toml_string(&color_to_hex(theme.red)),
            yellow = toml_string(&color_to_hex(theme.yellow)),
            green = toml_string(&color_to_hex(theme.green)),
            teal = toml_string(&color_to_hex(theme.teal)),
            blue = toml_string(&color_to_hex(theme.blue)),
            purple = toml_string(&color_to_hex(theme.purple)),
            pink = toml_string(&color_to_hex(theme.pink)),
            arrow_up = toml_char(symbols.arrow_up),
            arrow_down = toml_char(symbols.arrow_down),
            arrow_left = toml_char(symbols.arrow_left),
            arrow_right = toml_char(symbols.arrow_right),
            range_dash = toml_char(symbols.range_dash),
            sort_asc = toml_char(symbols.sort_asc),
            sort_desc = toml_char(symbols.sort_desc),
            separator = toml_char(symbols.separator),
            breadcrumb_prefix = toml_char(symbols.breadcrumb_prefix),
            breadcrumb_separator = toml_string(&symbols.breadcrumb_separator),
            empty_placeholder = toml_char(symbols.empty_placeholder),
            missing_placeholder = toml_char(symbols.missing_placeholder),
            ellipsis = toml_char(symbols.ellipsis),
            cursor = toml_char(symbols.cursor),
            active_bar = toml_char(symbols.active_bar),
            selection = toml_char(symbols.selection),
            loading = toml_char(symbols.loading),
            enter = toml_char(symbols.enter),
            foreign_key_arrow = toml_char(symbols.foreign_key_arrow),
            bool_true = toml_char(symbols.bool_true),
            bool_false = toml_char(symbols.bool_false),
            valid = toml_char(symbols.valid),
            invalid = toml_char(symbols.invalid),
            readonly = toml_char(symbols.readonly),
            dropdown = toml_char(symbols.dropdown),
            table_icon = toml_string(&symbols.table_icon),
            view_icon = toml_string(&symbols.view_icon),
            index_icon = toml_string(&symbols.index_icon),
            help_icon = toml_string(&symbols.help_icon),
            filter_icon = toml_string(&symbols.filter_icon),
            filter_marker = toml_string(&symbols.filter_marker),
            folder_open = toml_string(&symbols.folder_open),
            folder_closed = toml_string(&symbols.folder_closed),
            pk_icon = toml_string(&symbols.pk_icon),
            fk_icon = toml_string(&symbols.fk_icon),
            tab_shortcuts = symbols
                .tab_shortcuts
                .iter()
                .map(|shortcut| toml_string(shortcut))
                .collect::<Vec<_>>()
                .join(", "),
            box_horizontal = toml_char(symbols.box_horizontal),
            box_vertical = toml_char(symbols.box_vertical),
            focus_top_left = toml_char(symbols.focus_top_left),
            focus_top_right = toml_char(symbols.focus_top_right),
            focus_bottom_left = toml_char(symbols.focus_bottom_left),
            focus_bottom_right = toml_char(symbols.focus_bottom_right),
            tab_top_left = toml_char(symbols.tab_top_left),
            tab_top_right = toml_char(symbols.tab_top_right),
            tab_join_left = toml_char(symbols.tab_join_left),
            tab_join_right = toml_char(symbols.tab_join_right),
            tab_close = toml_char(symbols.tab_close),
            table_rule_header_mid = toml_char(symbols.table_rule_header_mid),
            table_rule_cross_mid = toml_char(symbols.table_rule_cross_mid),
            scrollbar_thumb = toml_char(symbols.scrollbar_thumb),
        )
    }

    fn ensure_current_config_file() -> anyhow::Result<()> {
        let current =
            crate::app_dirs::current_config_file().context("Config path is unavailable")?;
        if current.exists() {
            return Ok(());
        }

        let parent = current
            .parent()
            .context("Config path has no parent directory")?;
        std::fs::create_dir_all(parent)
            .with_context(|| format!("Failed to create config directory {}", parent.display()))?;

        if let Some(legacy) = crate::app_dirs::legacy_config_file().filter(|path| path.exists()) {
            std::fs::copy(&legacy, &current).with_context(|| {
                format!(
                    "Failed to copy legacy config {} to {}",
                    legacy.display(),
                    current.display()
                )
            })?;
        } else {
            std::fs::write(&current, Self::default_toml()).with_context(|| {
                format!("Failed to create default config {}", current.display())
            })?;
        }

        Ok(())
    }
}

fn toml_string(value: &str) -> String {
    toml::Value::String(value.to_string()).to_string()
}

fn toml_char(value: char) -> String {
    toml_string(&value.to_string())
}

fn color_to_hex(color: ratatui::style::Color) -> String {
    match color {
        ratatui::style::Color::Rgb(r, g, b) => format!("#{r:02x}{g:02x}{b:02x}"),
        _ => "#000000".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use ratatui::style::Color;

    use super::Config;
    use crate::theme::Theme;

    #[test]
    fn config_supports_theme_and_symbol_overrides() {
        let config: Config = toml::from_str(
            r##"
nerd_font = false

[theme]
accent = "#ff8800"

[symbols]
table_icon = "TAB"
sort_asc = ">"
"##,
        )
        .expect("config");

        let theme = config.resolve_theme().expect("theme");
        let symbols = config.resolve_symbols().expect("symbols");

        assert_eq!(theme.accent, Color::Rgb(0xff, 0x88, 0x00));
        assert_eq!(symbols.table_icon, "TAB");
        assert_eq!(symbols.sort_asc, '>');
        assert_eq!(symbols.view_icon, "[V]");
    }

    #[test]
    fn invalid_single_cell_symbol_is_rejected() {
        let config: Config = toml::from_str(
            r#"
[symbols]
selection = ">>"
"#,
        )
        .expect("config");

        let err = config
            .resolve_symbols()
            .expect_err("invalid symbol should fail");
        assert!(err.to_string().contains("symbols.selection"));
    }

    #[test]
    fn default_config_toml_round_trips() {
        let config: Config = toml::from_str(&Config::default_toml()).expect("default config");

        let theme = config.resolve_theme().expect("theme");
        let symbols = config.resolve_symbols().expect("symbols");

        assert_eq!(theme, Theme::default());
        assert_eq!(symbols.table_icon, "󰓫");
        assert_eq!(symbols.tab_shortcuts[0], "¹");
    }
}
