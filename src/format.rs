use logos_terminal::terminal::Submission;

pub enum Style {
    Human,
    Table,
    Tree,
    Json,
}

pub fn render(value: Submission, style: Style) -> Option<Submission> {
    let text = value.as_bytes();
    match style {
        Style::Human => Submission::from_bytes(text),
        Style::Table => prefixed(b"value: ", text),
        Style::Tree => prefixed(b"- ", text),
        Style::Json => quoted(text),
    }
}

fn prefixed(prefix: &[u8], text: &[u8]) -> Option<Submission> {
    let mut bytes = [0; 64];
    let length = prefix.len().checked_add(text.len())?;
    bytes[..prefix.len()].copy_from_slice(prefix);
    bytes[prefix.len()..length].copy_from_slice(text);
    Submission::from_bytes(&bytes[..length])
}

fn quoted(text: &[u8]) -> Option<Submission> {
    let mut bytes = [0; 64];
    let mut length = 0;
    for byte in text {
        if matches!(byte, b'"' | b'\\') {
            bytes.get_mut(length)?.clone_from(&b'\\');
            length += 1;
        }
        *bytes.get_mut(length)? = *byte;
        length += 1;
    }
    bytes.copy_within(0..length, 1);
    bytes[0] = b'"';
    bytes.get_mut(length + 1)?.clone_from(&b'"');
    Submission::from_bytes(&bytes[..length + 2])
}

pub fn self_check() -> bool {
    let Some(value) = Submission::from_bytes(b"x") else {
        return false;
    };
    render(value, Style::Human).is_some_and(|value| value.as_bytes() == b"x")
        && render(value, Style::Table).is_some_and(|value| value.as_bytes() == b"value: x")
        && render(value, Style::Tree).is_some_and(|value| value.as_bytes() == b"- x")
        && render(value, Style::Json).is_some_and(|value| value.as_bytes() == b"\"x\"")
}
