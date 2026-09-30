use crossterm::event::{Event, KeyCode, KeyModifiers};

use crate::app::Message;

pub fn translate_event(event: Event) -> Option<Message> {
    match event {
        Event::Key(key) => {
            if key.code == KeyCode::Char('q') && key.modifiers.contains(KeyModifiers::CONTROL) {
                Some(Message::Quit)
            } else {
                Some(Message::Key(key))
            }
        }
        Event::Mouse(mouse) => Some(Message::Mouse(mouse)),
        Event::Resize(..) => Some(Message::Resize),
        _ => None,
    }
}
