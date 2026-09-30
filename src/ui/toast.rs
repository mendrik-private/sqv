use std::collections::VecDeque;
use std::time::Instant;

use ratatui::{
    layout::Rect,
    style::{Color, Style},
    text::{Line, Span},
    widgets::Paragraph,
    Frame,
};

use unicode_width::UnicodeWidthStr;

use crate::theme::Theme;

#[derive(Debug, Clone, PartialEq)]
pub enum ToastKind {
    Success,
    Error,
    Info,
}

pub struct Toast {
    pub message: String,
    pub kind: ToastKind,
    pub created: Instant,
}

pub struct ToastState {
    pub toasts: VecDeque<Toast>,
}

impl ToastState {
    pub fn new() -> Self {
        Self {
            toasts: VecDeque::new(),
        }
    }

    pub fn push(&mut self, message: impl Into<String>, kind: ToastKind) {
        self.toasts.push_back(Toast {
            message: message.into(),
            kind,
            created: Instant::now(),
        });
        while self.toasts.len() > 5 {
            self.toasts.pop_front();
        }
    }

    pub fn tick(&mut self) {
        let now = Instant::now();
        self.toasts.retain(|t| {
            let duration = match t.kind {
                ToastKind::Success | ToastKind::Info => std::time::Duration::from_secs(3),
                ToastKind::Error => std::time::Duration::from_secs(5),
            };
            now.duration_since(t.created) < duration
        });
    }
}

impl Default for ToastState {
    fn default() -> Self {
        Self::new()
    }
}

pub fn render_toasts(frame: &mut Frame, area: Rect, state: &ToastState, theme: &Theme) {
    for (i, toast) in state.toasts.iter().enumerate() {
        let y = area.y + i as u16;
        if y >= area.y + area.height {
            break;
        }
        let bg = match toast.kind {
            ToastKind::Success => theme.green,
            ToastKind::Error => theme.red,
            ToastKind::Info => theme.fg_mute,
        };
        render_banner(frame, area, y, &toast.message, bg, theme);
    }
}

pub fn render_confirm(frame: &mut Frame, area: Rect, message: &str, theme: &Theme) {
    let y = area.y + area.height.saturating_sub(3);
    render_banner(frame, area, y, message, theme.yellow, theme);
}

/// A one-line message right-aligned in `area` at row `y`, at most 60 cells wide.
fn render_banner(frame: &mut Frame, area: Rect, y: u16, message: &str, bg: Color, theme: &Theme) {
    let text = format!("  {message}  ");
    let width = (UnicodeWidthStr::width(text.as_str()) as u16)
        .min(60)
        .min(area.width);
    let banner_area = Rect {
        x: area.x + area.width.saturating_sub(width),
        y,
        width,
        height: 1,
    };
    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(
            text,
            Style::default().fg(theme.bg).bg(bg),
        ))),
        banner_area,
    );
}
