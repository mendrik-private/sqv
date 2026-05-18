use anyhow::{bail, Context};
use ratatui::style::Color;
use serde::{Deserialize, Serialize};

#[allow(dead_code)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Theme {
    pub bg: Color,
    pub bg_soft: Color,
    pub bg_raised: Color,
    pub line: Color,
    pub line_soft: Color,
    pub fg: Color,
    pub fg_dim: Color,
    pub fg_mute: Color,
    pub fg_faint: Color,
    pub accent: Color,
    pub red: Color,
    pub yellow: Color,
    pub green: Color,
    pub teal: Color,
    pub blue: Color,
    pub purple: Color,
    pub pink: Color,
}

impl Default for Theme {
    fn default() -> Self {
        Self {
            bg: Color::Rgb(0x23, 0x21, 0x1f),
            bg_soft: Color::Rgb(0x26, 0x23, 0x21),
            bg_raised: Color::Rgb(0x29, 0x26, 0x24),
            line: Color::Rgb(0x3a, 0x33, 0x2f),
            line_soft: Color::Rgb(0x2f, 0x2a, 0x27),
            fg: Color::Rgb(0xe8, 0xdf, 0xd3),
            fg_dim: Color::Rgb(0xa8, 0x9c, 0x8a),
            fg_mute: Color::Rgb(0x6b, 0x64, 0x59),
            fg_faint: Color::Rgb(0x4a, 0x45, 0x3e),
            accent: Color::Rgb(0xd9, 0x9a, 0x5e),
            red: Color::Rgb(0xe0, 0x6c, 0x75),
            yellow: Color::Rgb(0xe5, 0xc0, 0x7b),
            green: Color::Rgb(0xa3, 0xb5, 0x65),
            teal: Color::Rgb(0x7c, 0xb7, 0xa8),
            blue: Color::Rgb(0x82, 0xaa, 0xdc),
            purple: Color::Rgb(0xc0, 0x8b, 0xc0),
            pink: Color::Rgb(0xd8, 0x8a, 0xa0),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct ThemeOverrides {
    pub bg: Option<String>,
    pub bg_soft: Option<String>,
    pub bg_raised: Option<String>,
    pub line: Option<String>,
    pub line_soft: Option<String>,
    pub fg: Option<String>,
    pub fg_dim: Option<String>,
    pub fg_mute: Option<String>,
    pub fg_faint: Option<String>,
    pub accent: Option<String>,
    pub red: Option<String>,
    pub yellow: Option<String>,
    pub green: Option<String>,
    pub teal: Option<String>,
    pub blue: Option<String>,
    pub purple: Option<String>,
    pub pink: Option<String>,
}

impl ThemeOverrides {
    pub fn resolve(&self) -> anyhow::Result<Theme> {
        let mut theme = Theme::default();
        apply_color_override("theme.bg", &mut theme.bg, &self.bg)?;
        apply_color_override("theme.bg_soft", &mut theme.bg_soft, &self.bg_soft)?;
        apply_color_override("theme.bg_raised", &mut theme.bg_raised, &self.bg_raised)?;
        apply_color_override("theme.line", &mut theme.line, &self.line)?;
        apply_color_override("theme.line_soft", &mut theme.line_soft, &self.line_soft)?;
        apply_color_override("theme.fg", &mut theme.fg, &self.fg)?;
        apply_color_override("theme.fg_dim", &mut theme.fg_dim, &self.fg_dim)?;
        apply_color_override("theme.fg_mute", &mut theme.fg_mute, &self.fg_mute)?;
        apply_color_override("theme.fg_faint", &mut theme.fg_faint, &self.fg_faint)?;
        apply_color_override("theme.accent", &mut theme.accent, &self.accent)?;
        apply_color_override("theme.red", &mut theme.red, &self.red)?;
        apply_color_override("theme.yellow", &mut theme.yellow, &self.yellow)?;
        apply_color_override("theme.green", &mut theme.green, &self.green)?;
        apply_color_override("theme.teal", &mut theme.teal, &self.teal)?;
        apply_color_override("theme.blue", &mut theme.blue, &self.blue)?;
        apply_color_override("theme.purple", &mut theme.purple, &self.purple)?;
        apply_color_override("theme.pink", &mut theme.pink, &self.pink)?;
        Ok(theme)
    }
}

fn apply_color_override(
    field: &str,
    target: &mut Color,
    value: &Option<String>,
) -> anyhow::Result<()> {
    if let Some(value) = value {
        *target = parse_hex_color(field, value)?;
    }
    Ok(())
}

fn parse_hex_color(field: &str, value: &str) -> anyhow::Result<Color> {
    let hex = value.strip_prefix('#').unwrap_or(value);
    if hex.len() != 6 {
        bail!("{field} must use #RRGGBB format");
    }

    let r = u8::from_str_radix(&hex[0..2], 16)
        .with_context(|| format!("{field} has an invalid red channel"))?;
    let g = u8::from_str_radix(&hex[2..4], 16)
        .with_context(|| format!("{field} has an invalid green channel"))?;
    let b = u8::from_str_radix(&hex[4..6], 16)
        .with_context(|| format!("{field} has an invalid blue channel"))?;

    Ok(Color::Rgb(r, g, b))
}
