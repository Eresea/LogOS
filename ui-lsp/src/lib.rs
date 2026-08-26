use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::io::{self, BufRead, Write};

use logos_ui_compiler::{
    UI_BINDING_NAMES, UI_COMPONENT_NAMES, UI_EVENT_NAMES, UI_STYLE_NAMES, compile,
};

const MAX_LSP_MESSAGE_BYTES: usize = 256 * 1024;
const MAX_DOCUMENTS: usize = 32;

pub fn diagnostics_json(uri: &str, source: &str) -> String {
    let build = compile(source);
    let mut output = String::from(
        "{\"jsonrpc\":\"2.0\",\"method\":\"textDocument/publishDiagnostics\",\"params\":{\"uri\":",
    );
    push_json_string(&mut output, uri);
    output.push_str(",\"diagnostics\":[");
    for index in 0..build.diagnostics.len() {
        if index != 0 {
            output.push(',');
        }
        let Some(diagnostic) = build.diagnostics.get(index) else { continue };
        let (line, column) = diagnostic.line_column(source);
        write!(
            output,
            "{{\"range\":{{\"start\":{{\"line\":{},\"character\":{}}},\"end\":{{\"line\":{},\"character\":{}}}}},\"severity\":1,\"code\":",
            line.saturating_sub(1),
            column.saturating_sub(1),
            line.saturating_sub(1),
            column.saturating_sub(1) + usize::from(diagnostic.span.length),
        )
        .expect("String formatting cannot fail");
        push_json_string(&mut output, diagnostic.code());
        output.push_str(",\"source\":\"logos-ui\",\"message\":");
        push_json_string(&mut output, diagnostic.message());
        output.push('}');
    }
    output.push_str("]}}");
    output
}

pub fn completion_json(source: &str, line: usize, character: usize) -> String {
    let offset = offset_at_position(source, line, character);
    let (items, prefix) = completion_context(source, offset);
    let mut output = String::from("{\"isIncomplete\":false,\"items\":[");
    let mut first = true;
    for item in items {
        if !item.starts_with(prefix) {
            continue;
        }
        if !first {
            output.push(',');
        }
        first = false;
        output.push_str("{\"label\":");
        push_json_string(&mut output, item);
        output.push_str(",\"kind\":10}");
    }
    output.push_str("]}");
    output
}

pub fn hover_json(source: &str, line: usize, character: usize) -> String {
    let offset = offset_at_position(source, line, character);
    let Some(word) = word_at(source, offset) else { return "null".to_owned() };
    let description = if UI_COMPONENT_NAMES.contains(&word) {
        "component"
    } else if UI_STYLE_NAMES.contains(&word) || word.starts_with("gap-") {
        "style utility"
    } else {
        return "null".to_owned();
    };
    let mut output = String::from("{\"contents\":{\"kind\":\"markdown\",\"value\":");
    let mut value = String::new();
    write!(&mut value, "`{word}` {description}").expect("String formatting cannot fail");
    push_json_string(&mut output, &value);
    output.push_str("}}");
    output
}

pub fn run_stdio<R: BufRead, W: Write>(mut reader: R, mut writer: W) -> io::Result<()> {
    let mut server = Server::default();
    while let Some(message) = read_message(&mut reader)? {
        if !server.handle(&message, &mut writer)? {
            break;
        }
    }
    Ok(())
}

#[derive(Default)]
struct Server {
    documents: BTreeMap<String, String>,
}

impl Server {
    fn handle<W: Write>(&mut self, message: &str, writer: &mut W) -> io::Result<bool> {
        let method = json_string_field(message, "method").unwrap_or_default();
        match method.as_str() {
            "initialize" => {
                if let Some(id) = json_field(message, "id") {
                    send_message(
                        writer,
                        &format!(
                            "{{\"jsonrpc\":\"2.0\",\"id\":{id},\"result\":{{\"capabilities\":{{\"textDocumentSync\":1,\"completionProvider\":{{\"triggerCharacters\":[\"<\",\"{{\",\"[\",\"(\"]}},\"hoverProvider\":true}},\"serverInfo\":{{\"name\":\"logos-ui-lsp\",\"version\":\"0.1.0\"}}}}}}"
                        ),
                    )?;
                }
            }
            "shutdown" => {
                if let Some(id) = json_field(message, "id") {
                    send_message(
                        writer,
                        &format!("{{\"jsonrpc\":\"2.0\",\"id\":{id},\"result\":null}}"),
                    )?;
                }
            }
            "exit" => return Ok(false),
            "textDocument/didOpen" => {
                self.update_document(message);
                self.publish(message, writer)?;
            }
            "textDocument/didChange" => {
                self.update_document(message);
                self.publish(message, writer)?;
            }
            "textDocument/didClose" => {
                if let Some(uri) = document_uri(message) {
                    self.documents.remove(&uri);
                }
            }
            "textDocument/completion" => {
                if let Some(id) = json_field(message, "id") {
                    let source = self.source_for(message);
                    let line = json_number_field(message, "line").unwrap_or(0);
                    let character = json_number_field(message, "character").unwrap_or(0);
                    let result = completion_json(&source, line, character);
                    send_message(
                        writer,
                        &format!("{{\"jsonrpc\":\"2.0\",\"id\":{id},\"result\":{result}}}"),
                    )?;
                }
            }
            "textDocument/hover" => {
                if let Some(id) = json_field(message, "id") {
                    let source = self.source_for(message);
                    let line = json_number_field(message, "line").unwrap_or(0);
                    let character = json_number_field(message, "character").unwrap_or(0);
                    let result = hover_json(&source, line, character);
                    send_message(
                        writer,
                        &format!("{{\"jsonrpc\":\"2.0\",\"id\":{id},\"result\":{result}}}"),
                    )?;
                }
            }
            _ => {
                if let Some(id) = json_field(message, "id") {
                    send_message(
                        writer,
                        &format!(
                            "{{\"jsonrpc\":\"2.0\",\"id\":{id},\"error\":{{\"code\":-32601,\"message\":\"method not supported\"}}}}"
                        ),
                    )?;
                }
            }
        }
        Ok(true)
    }

    fn update_document(&mut self, message: &str) {
        let Some(uri) = document_uri(message) else { return };
        let Some(source) = json_string_field(message, "text") else { return };
        if !self.documents.contains_key(&uri) && self.documents.len() == MAX_DOCUMENTS {
            return;
        }
        self.documents.insert(uri, source);
    }

    fn source_for(&self, message: &str) -> String {
        document_uri(message).and_then(|uri| self.documents.get(&uri).cloned()).unwrap_or_default()
    }

    fn publish<W: Write>(&self, message: &str, writer: &mut W) -> io::Result<()> {
        let Some(uri) = document_uri(message) else { return Ok(()) };
        let source = self.documents.get(&uri).map(String::as_str).unwrap_or("");
        send_message(writer, &diagnostics_json(&uri, source))
    }
}

fn completion_context(source: &str, offset: usize) -> (&'static [&'static str], &str) {
    let prefix_start = source[..offset.min(source.len())]
        .rfind(|character: char| {
            character.is_ascii_whitespace() || "<>[]{}()=\"".contains(character)
        })
        .map_or(0, |index| index + 1);
    let prefix = &source[prefix_start..offset.min(source.len())];
    let before = &source[..offset.min(source.len())];
    let last = before.rfind(['<', '{', '[', '(']);
    let close = before.rfind(['>', '}', ']', ')']);
    let items = match (last, close) {
        (Some(open), Some(close)) if close > open => UI_COMPONENT_NAMES.as_slice(),
        (Some(open), _) => match before.as_bytes()[open] {
            b'{' => UI_STYLE_NAMES.as_slice(),
            b'[' => UI_BINDING_NAMES.as_slice(),
            b'(' => UI_EVENT_NAMES.as_slice(),
            _ => UI_COMPONENT_NAMES.as_slice(),
        },
        _ => UI_COMPONENT_NAMES.as_slice(),
    };
    (items, prefix)
}

fn offset_at_position(source: &str, line: usize, character: usize) -> usize {
    let mut current_line = 0;
    let mut current_character = 0;
    for (index, value) in source.char_indices() {
        if current_line == line && current_character >= character {
            return index;
        }
        if value == '\n' {
            current_line += 1;
            current_character = 0;
        } else {
            current_character += value.len_utf16();
        }
    }
    source.len()
}

fn word_at(source: &str, offset: usize) -> Option<&str> {
    let offset = offset.min(source.len());
    let mut start = offset;
    while start > 0 && is_word_byte(source.as_bytes()[start - 1]) {
        start -= 1;
    }
    let mut end = offset;
    while end < source.len() && is_word_byte(source.as_bytes()[end]) {
        end += 1;
    }
    source.get(start..end).filter(|word| !word.is_empty())
}

fn is_word_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-' | b':')
}

fn document_uri(message: &str) -> Option<String> {
    json_string_field(message, "uri").or_else(|| json_string_field(message, "document"))
}

fn read_message<R: BufRead>(reader: &mut R) -> io::Result<Option<String>> {
    let mut content_length = None;
    let mut line = String::new();
    loop {
        line.clear();
        if reader.read_line(&mut line)? == 0 {
            return Ok(None);
        }
        let header = line.trim_end_matches(['\r', '\n']);
        if header.is_empty() {
            break;
        }
        if let Some((name, value)) = header.split_once(':') {
            if name.eq_ignore_ascii_case("content-length") {
                content_length = value.trim().parse::<usize>().ok();
            }
        }
    }
    let length = content_length
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "missing Content-Length"))?;
    if length > MAX_LSP_MESSAGE_BYTES {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "LSP message exceeds bound"));
    }
    let mut bytes = vec![0; length];
    reader.read_exact(&mut bytes)?;
    String::from_utf8(bytes)
        .map(Some)
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "LSP message is not UTF-8"))
}

fn send_message<W: Write>(writer: &mut W, message: &str) -> io::Result<()> {
    write!(writer, "Content-Length: {}\r\n\r\n{}", message.len(), message)?;
    writer.flush()
}

fn json_string_field(message: &str, key: &str) -> Option<String> {
    let (start, end) = json_field_span(message, key)?;
    let bytes = message.as_bytes();
    (bytes.get(start) == Some(&b'"')).then(|| unescape_json_string(&message[start + 1..end - 1]))?
}

fn json_field<'a>(message: &'a str, key: &str) -> Option<&'a str> {
    let (start, end) = json_field_span(message, key)?;
    message.get(start..end)
}

fn json_number_field(message: &str, key: &str) -> Option<usize> {
    json_field(message, key)?.parse().ok()
}

fn json_field_span(message: &str, key: &str) -> Option<(usize, usize)> {
    let needle = format!("\"{key}\"");
    let start = message.find(&needle)? + needle.len();
    let bytes = message.as_bytes();
    let mut value_start = start;
    while bytes.get(value_start).is_some_and(u8::is_ascii_whitespace) {
        value_start += 1;
    }
    if bytes.get(value_start) != Some(&b':') {
        return None;
    }
    value_start += 1;
    while bytes.get(value_start).is_some_and(u8::is_ascii_whitespace) {
        value_start += 1;
    }
    let value_end = json_value_end(bytes, value_start)?;
    Some((value_start, value_end))
}

fn json_value_end(bytes: &[u8], start: usize) -> Option<usize> {
    match bytes.get(start).copied()? {
        b'"' => {
            let mut index = start + 1;
            while index < bytes.len() {
                match bytes[index] {
                    b'\\' => index += 2,
                    b'"' => return Some(index + 1),
                    _ => index += 1,
                }
            }
            None
        }
        b'{' | b'[' => {
            let mut depth = 0usize;
            let mut string = false;
            let mut index = start;
            while index < bytes.len() {
                match bytes[index] {
                    b'"' if !string => string = true,
                    b'"' if string && (index == 0 || bytes[index - 1] != b'\\') => string = false,
                    b'{' | b'[' if !string => depth += 1,
                    b'}' | b']' if !string => {
                        depth = depth.checked_sub(1)?;
                        if depth == 0 {
                            return Some(index + 1);
                        }
                    }
                    _ => {}
                }
                index += 1;
            }
            None
        }
        _ => {
            let mut index = start;
            while index < bytes.len()
                && !matches!(bytes[index], b',' | b'}' | b']' | b' ' | b'\r' | b'\n' | b'\t')
            {
                index += 1;
            }
            Some(index)
        }
    }
}

fn unescape_json_string(value: &str) -> Option<String> {
    let mut output = String::new();
    let mut chars = value.chars();
    while let Some(character) = chars.next() {
        if character != '\\' {
            output.push(character);
            continue;
        }
        match chars.next()? {
            '"' => output.push('"'),
            '\\' => output.push('\\'),
            '/' => output.push('/'),
            'b' => output.push('\u{0008}'),
            'f' => output.push('\u{000c}'),
            'n' => output.push('\n'),
            'r' => output.push('\r'),
            't' => output.push('\t'),
            'u' => {
                let mut value = 0u32;
                for _ in 0..4 {
                    value = value.checked_mul(16)?.checked_add(chars.next()?.to_digit(16)?)?;
                }
                output.push(char::from_u32(value)?);
            }
            _ => return None,
        }
    }
    Some(output)
}

fn push_json_string(output: &mut String, value: &str) {
    output.push('"');
    for character in value.chars() {
        match character {
            '"' => output.push_str("\\\""),
            '\\' => output.push_str("\\\\"),
            '\n' => output.push_str("\\n"),
            '\r' => output.push_str("\\r"),
            '\t' => output.push_str("\\t"),
            character if character.is_control() => {
                write!(output, "\\u{:04x}", character as u32)
                    .expect("String formatting cannot fail");
            }
            character => output.push(character),
        }
    }
    output.push('"');
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn emits_compiler_diagnostics_as_lsp_json() {
        let json = diagnostics_json("file:///test.ui", "<ui.unknown />");
        assert!(json.contains("UI004"));
        assert!(json.contains("\"line\":0"));
        assert!(json.contains("file:///test.ui"));
    }

    #[test]
    fn completes_contextual_ui_vocabulary() {
        assert!(completion_json("<ui.", 0, 5).contains("ui.button"));
        assert!(completion_json("<ui.button {f", 0, 14).contains("flex-x"));
        assert!(!completion_json("<ui.button {f", 0, 14).contains("ui.input"));
    }

    #[test]
    fn serves_initialize_open_and_shutdown_over_stdio() {
        let messages = [
            r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}"#,
            r#"{"jsonrpc":"2.0","method":"textDocument/didOpen","params":{"textDocument":{"uri":"file:///test.ui","text":"<ui.unknown />"}}}"#,
            r#"{"jsonrpc":"2.0","id":2,"method":"shutdown","params":null}"#,
            r#"{"jsonrpc":"2.0","method":"exit"}"#,
        ]
        .into_iter()
        .map(|message| format!("Content-Length: {}\r\n\r\n{message}", message.len()))
        .collect::<String>();
        let mut output = Vec::new();
        run_stdio(Cursor::new(messages), &mut output).unwrap();
        let output = String::from_utf8(output).unwrap();
        assert!(output.contains("logos-ui-lsp"));
        assert!(output.contains("publishDiagnostics"));
        assert!(output.contains("\"id\":2"));
    }
}
