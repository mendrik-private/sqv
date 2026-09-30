//! Editing a string at a cursor measured in chars rather than bytes.

fn byte_index(text: &str, char_index: usize) -> usize {
    text.char_indices()
        .nth(char_index)
        .map_or(text.len(), |(index, _)| index)
}

pub(crate) fn insert(text: &mut String, cursor: &mut usize, ch: char) {
    text.insert(byte_index(text, *cursor), ch);
    *cursor += 1;
}

/// Deletes the char before the cursor; false when the cursor is at the start.
pub(crate) fn delete_backward(text: &mut String, cursor: &mut usize) -> bool {
    if *cursor == 0 {
        return false;
    }
    let start = byte_index(text, *cursor - 1);
    let end = byte_index(text, *cursor);
    text.replace_range(start..end, "");
    *cursor -= 1;
    true
}

pub(crate) fn move_right(text: &str, cursor: &mut usize) {
    if *cursor < text.chars().count() {
        *cursor += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn edits_multibyte_text_by_char_position() {
        let mut text = "añb".to_string();
        let mut cursor = 2;
        insert(&mut text, &mut cursor, 'é');
        assert_eq!((text.as_str(), cursor), ("añéb", 3));
        assert!(delete_backward(&mut text, &mut cursor));
        assert!(delete_backward(&mut text, &mut cursor));
        assert_eq!((text.as_str(), cursor), ("ab", 1));
        move_right(&text, &mut cursor);
        move_right(&text, &mut cursor);
        assert_eq!(cursor, 2);
    }
}
