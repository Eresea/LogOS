#![no_std]

use logos_storage_service::MAX_FILE_BYTES;

pub const MAX_HEADER_BYTES: usize = 1024;
pub const MAX_PATH_BYTES: usize = 160;
pub const MAX_REQUEST_BYTES: usize = 192;
pub const MAX_BODY_BYTES: usize = MAX_FILE_BYTES;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Url {
    pub address: [u8; 4],
    pub port: u16,
    path: [u8; MAX_PATH_BYTES],
    path_len: u16,
}

impl Url {
    pub fn parse(input: &[u8]) -> Result<Self, UrlError> {
        if !input.starts_with(b"http://") {
            return Err(UrlError::Scheme);
        }
        let authority_start = 7;
        let path_start = input[authority_start..]
            .iter()
            .position(|byte| *byte == b'/')
            .map(|index| authority_start + index)
            .unwrap_or(input.len());
        let authority = &input[authority_start..path_start];
        if authority.is_empty() {
            return Err(UrlError::Authority);
        }
        let (host, port) = match authority.iter().position(|byte| *byte == b':') {
            Some(index) => {
                if authority[index + 1..].contains(&b':') {
                    return Err(UrlError::Authority);
                }
                let port = parse_decimal(&authority[index + 1..]).ok_or(UrlError::Port)?;
                if port == 0 {
                    return Err(UrlError::Port);
                }
                (&authority[..index], port)
            }
            None => (authority, 80),
        };
        let address = parse_ipv4(host).ok_or(UrlError::Address)?;
        let path = if path_start == input.len() { b"/" } else { &input[path_start..] };
        if path.is_empty()
            || path.len() > MAX_PATH_BYTES
            || path[0] != b'/'
            || path.iter().any(|byte| *byte == b'\r' || *byte == b'\n' || *byte < 0x20)
        {
            return Err(UrlError::Path);
        }
        let mut result =
            Self { address, port, path: [0; MAX_PATH_BYTES], path_len: path.len() as u16 };
        result.path[..path.len()].copy_from_slice(path);
        Ok(result)
    }

    pub fn path(&self) -> &[u8] {
        &self.path[..usize::from(self.path_len)]
    }

    pub fn request(&self) -> Result<([u8; MAX_REQUEST_BYTES], usize), RequestError> {
        let mut output = [0; MAX_REQUEST_BYTES];
        let mut len = 0;
        push(&mut output, &mut len, b"GET ")?;
        push(&mut output, &mut len, self.path())?;
        push(&mut output, &mut len, b" HTTP/1.1\r\nHost: ")?;
        push_ipv4(&mut output, &mut len, self.address)?;
        push(&mut output, &mut len, b":")?;
        push_decimal(&mut output, &mut len, self.port)?;
        push(&mut output, &mut len, b"\r\nConnection: close\r\n\r\n")?;
        Ok((output, len))
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum UrlError {
    Scheme,
    Authority,
    Address,
    Port,
    Path,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RequestError {
    TooLarge,
}

fn parse_ipv4(input: &[u8]) -> Option<[u8; 4]> {
    let mut result = [0; 4];
    let mut part = 0;
    let mut value = 0u16;
    let mut digits = 0;
    for byte in input.iter().copied().chain(core::iter::once(b'.')) {
        if byte.is_ascii_digit() {
            if digits == 3 {
                return None;
            }
            value = value * 10 + u16::from(byte - b'0');
            if value > 255 {
                return None;
            }
            digits += 1;
        } else if byte == b'.' && digits != 0 {
            if part == 4 {
                return None;
            }
            result[part] = value as u8;
            part += 1;
            value = 0;
            digits = 0;
        } else {
            return None;
        }
    }
    (part == 4).then_some(result)
}

fn parse_decimal(input: &[u8]) -> Option<u16> {
    if input.is_empty() {
        return None;
    }
    let mut value = 0u32;
    for byte in input {
        if !byte.is_ascii_digit() {
            return None;
        }
        value = value.checked_mul(10)?.checked_add(u32::from(byte - b'0'))?;
        if value > u32::from(u16::MAX) {
            return None;
        }
    }
    Some(value as u16)
}

fn push<const N: usize>(
    out: &mut [u8; N],
    len: &mut usize,
    bytes: &[u8],
) -> Result<(), RequestError> {
    if bytes.len() > N.saturating_sub(*len) {
        return Err(RequestError::TooLarge);
    }
    out[*len..*len + bytes.len()].copy_from_slice(bytes);
    *len += bytes.len();
    Ok(())
}

fn push_ipv4<const N: usize>(
    out: &mut [u8; N],
    len: &mut usize,
    address: [u8; 4],
) -> Result<(), RequestError> {
    for (index, octet) in address.into_iter().enumerate() {
        if index != 0 {
            push(out, len, b".")?;
        }
        push_decimal(out, len, u16::from(octet))?;
    }
    Ok(())
}

fn push_decimal<const N: usize>(
    out: &mut [u8; N],
    len: &mut usize,
    value: u16,
) -> Result<(), RequestError> {
    let mut digits = [0; 5];
    let mut cursor = digits.len();
    let mut value = value;
    loop {
        cursor -= 1;
        digits[cursor] = b'0' + (value % 10) as u8;
        value /= 10;
        if value == 0 {
            break;
        }
    }
    push(out, len, &digits[cursor..])
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ResponseError {
    HeaderTooLarge,
    Malformed,
    UnsupportedStatus,
    ConflictingLength,
    UnsupportedEncoding,
    UnsupportedTransfer,
    TooLarge,
    Overflow,
    Incomplete,
    Trailers,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum BodyMode {
    ContentLength(usize),
    Chunked,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ParseState {
    Headers,
    Body,
    ChunkSize,
    ChunkSizeLf,
    ChunkData,
    ChunkDataCr,
    ChunkDataLf,
    ChunkEnd,
    ChunkEndLf,
    Complete,
}

pub struct ResponseParser {
    headers: [u8; MAX_HEADER_BYTES],
    header_len: usize,
    body: [u8; MAX_BODY_BYTES],
    body_len: usize,
    mode: Option<BodyMode>,
    state: ParseState,
    remaining: usize,
    chunk_size: [u8; 16],
    chunk_len: usize,
    status: u16,
}

impl ResponseParser {
    pub const fn new() -> Self {
        Self {
            headers: [0; MAX_HEADER_BYTES],
            header_len: 0,
            body: [0; MAX_BODY_BYTES],
            body_len: 0,
            mode: None,
            state: ParseState::Headers,
            remaining: 0,
            chunk_size: [0; 16],
            chunk_len: 0,
            status: 0,
        }
    }

    pub fn feed(&mut self, input: &[u8]) -> Result<usize, ResponseError> {
        let mut consumed = 0;
        while consumed < input.len() {
            if self.state == ParseState::Complete {
                return Err(ResponseError::Malformed);
            }
            match self.state {
                ParseState::Headers => {
                    if self.header_len == MAX_HEADER_BYTES {
                        return Err(ResponseError::HeaderTooLarge);
                    }
                    self.headers[self.header_len] = input[consumed];
                    self.header_len += 1;
                    consumed += 1;
                    if self.header_len >= 4
                        && self.headers[self.header_len - 4..self.header_len] == *b"\r\n\r\n"
                    {
                        self.parse_headers()?;
                    }
                }
                ParseState::Body => {
                    let mode_remaining = self.remaining.min(input.len() - consumed);
                    self.append_body(&input[consumed..consumed + mode_remaining])?;
                    consumed += mode_remaining;
                    self.remaining -= mode_remaining;
                    if self.remaining == 0 {
                        self.state = ParseState::Complete;
                    }
                }
                ParseState::ChunkSize => {
                    let byte = input[consumed];
                    consumed += 1;
                    if byte == b'\r' {
                        if self.chunk_len == 0 {
                            return Err(ResponseError::Malformed);
                        }
                        if consumed >= input.len() {
                            self.state = ParseState::ChunkSizeLf;
                            return Ok(consumed);
                        }
                        if input[consumed] != b'\n' {
                            return Err(ResponseError::Malformed);
                        }
                        consumed += 1;
                        self.remaining = parse_hex(&self.chunk_size[..self.chunk_len])?;
                        if self.remaining == 0 {
                            self.state = ParseState::ChunkEnd;
                        } else {
                            self.state = ParseState::ChunkData;
                        }
                        self.chunk_len = 0;
                    } else if byte == b'\n' || byte == b';' || byte.is_ascii_whitespace() {
                        return Err(ResponseError::Malformed);
                    } else if self.chunk_len == self.chunk_size.len() {
                        return Err(ResponseError::Overflow);
                    } else {
                        self.chunk_size[self.chunk_len] = byte;
                        self.chunk_len += 1;
                    }
                }
                ParseState::ChunkSizeLf => {
                    if input[consumed] != b'\n' {
                        return Err(ResponseError::Malformed);
                    }
                    consumed += 1;
                    self.remaining = parse_hex(&self.chunk_size[..self.chunk_len])?;
                    if self.remaining == 0 {
                        self.state = ParseState::ChunkEnd;
                    } else {
                        self.state = ParseState::ChunkData;
                    }
                    self.chunk_len = 0;
                }
                ParseState::ChunkData => {
                    let count = self.remaining.min(input.len() - consumed);
                    self.append_body(&input[consumed..consumed + count])?;
                    consumed += count;
                    self.remaining -= count;
                    if self.remaining == 0 {
                        self.state = ParseState::ChunkDataCr;
                    }
                }
                ParseState::ChunkDataCr => {
                    if input[consumed] != b'\r' {
                        return Err(ResponseError::Malformed);
                    }
                    consumed += 1;
                    if consumed == input.len() {
                        self.state = ParseState::ChunkDataLf;
                        return Ok(consumed);
                    }
                    if input[consumed] != b'\n' {
                        return Err(ResponseError::Malformed);
                    }
                    consumed += 1;
                    self.state = ParseState::ChunkSize;
                }
                ParseState::ChunkDataLf => {
                    if input[consumed] != b'\n' {
                        return Err(ResponseError::Malformed);
                    }
                    consumed += 1;
                    self.state = ParseState::ChunkSize;
                }
                ParseState::ChunkEnd => {
                    if input[consumed] != b'\r' {
                        return Err(ResponseError::Trailers);
                    }
                    consumed += 1;
                    if consumed == input.len() {
                        self.state = ParseState::ChunkEndLf;
                        return Ok(consumed);
                    }
                    if input[consumed] != b'\n' {
                        return Err(ResponseError::Trailers);
                    }
                    consumed += 1;
                    self.state = ParseState::Complete;
                }
                ParseState::ChunkEndLf => {
                    if input[consumed] != b'\n' {
                        return Err(ResponseError::Trailers);
                    }
                    consumed += 1;
                    self.state = ParseState::Complete;
                }
                ParseState::Complete => unreachable!(),
            }
        }
        Ok(consumed)
    }

    fn parse_headers(&mut self) -> Result<(), ResponseError> {
        let split = self.headers[..self.header_len]
            .windows(4)
            .position(|window| window == b"\r\n\r\n")
            .ok_or(ResponseError::Malformed)?;
        let header_end = split + 2;
        let first_end = self.headers[..header_end]
            .windows(2)
            .position(|window| window == b"\r\n")
            .ok_or(ResponseError::Malformed)?;
        let status = &self.headers[..first_end];
        if status.len() < 13
            || &status[..9] != b"HTTP/1.1 "
            || status[12] != b' '
            || !status[9..12].iter().all(|byte| byte.is_ascii_digit())
        {
            return Err(ResponseError::Malformed);
        }
        self.status = u16::from(status[9] - b'0') * 100
            + u16::from(status[10] - b'0') * 10
            + u16::from(status[11] - b'0');
        if !(200..300).contains(&self.status) {
            return Err(ResponseError::UnsupportedStatus);
        }
        let mut content_length = None;
        let mut chunked = false;
        let mut cursor = first_end + 2;
        while cursor < header_end {
            let end = self.headers[cursor..header_end]
                .windows(2)
                .position(|window| window == b"\r\n")
                .ok_or(ResponseError::Malformed)?
                + cursor;
            if end == cursor {
                break;
            }
            let line = &self.headers[cursor..end];
            let colon =
                line.iter().position(|byte| *byte == b':').ok_or(ResponseError::Malformed)?;
            let name = &line[..colon];
            let value = trim_ows(&line[colon + 1..]);
            if name.eq_ignore_ascii_case(b"content-length") {
                let parsed = parse_decimal_usize(value).ok_or(ResponseError::Malformed)?;
                if parsed > MAX_BODY_BYTES {
                    return Err(ResponseError::TooLarge);
                }
                if content_length.is_some_and(|old| old != parsed) {
                    return Err(ResponseError::ConflictingLength);
                }
                content_length = Some(parsed);
            } else if name.eq_ignore_ascii_case(b"transfer-encoding") {
                if !value.eq_ignore_ascii_case(b"chunked") {
                    return Err(ResponseError::UnsupportedTransfer);
                }
                chunked = true;
            } else if name.eq_ignore_ascii_case(b"content-encoding")
                || name.eq_ignore_ascii_case(b"location")
                || name.eq_ignore_ascii_case(b"trailer")
            {
                return Err(ResponseError::UnsupportedEncoding);
            }
            cursor = end + 2;
        }
        if chunked && content_length.is_some() {
            return Err(ResponseError::ConflictingLength);
        }
        self.mode = if chunked {
            Some(BodyMode::Chunked)
        } else {
            content_length.map(BodyMode::ContentLength)
        };
        match self.mode {
            Some(BodyMode::ContentLength(length)) => {
                self.remaining = length;
                self.state = if length == 0 { ParseState::Complete } else { ParseState::Body };
            }
            Some(BodyMode::Chunked) => self.state = ParseState::ChunkSize,
            None => return Err(ResponseError::Malformed),
        }
        Ok(())
    }

    fn append_body(&mut self, bytes: &[u8]) -> Result<(), ResponseError> {
        if bytes.len() > MAX_BODY_BYTES - self.body_len {
            return Err(ResponseError::TooLarge);
        }
        self.body[self.body_len..self.body_len + bytes.len()].copy_from_slice(bytes);
        self.body_len += bytes.len();
        Ok(())
    }

    pub fn complete(&self) -> bool {
        self.state == ParseState::Complete
    }
    pub fn status(&self) -> u16 {
        self.status
    }
    pub fn body(&self) -> &[u8] {
        &self.body[..self.body_len]
    }
    pub fn content_length(&self) -> Option<usize> {
        match self.mode {
            Some(BodyMode::ContentLength(length)) => Some(length),
            _ => None,
        }
    }
}

impl Default for ResponseParser {
    fn default() -> Self {
        Self::new()
    }
}

fn trim_ows(mut value: &[u8]) -> &[u8] {
    while value.first().is_some_and(|byte| *byte == b' ' || *byte == b'\t') {
        value = &value[1..];
    }
    while value.last().is_some_and(|byte| *byte == b' ' || *byte == b'\t') {
        value = &value[..value.len() - 1];
    }
    value
}

fn parse_decimal_usize(input: &[u8]) -> Option<usize> {
    if input.is_empty() {
        return None;
    }
    let mut value = 0usize;
    for byte in input {
        if !byte.is_ascii_digit() {
            return None;
        }
        value = value.checked_mul(10)?.checked_add(usize::from(byte - b'0'))?;
    }
    Some(value)
}

fn parse_hex(input: &[u8]) -> Result<usize, ResponseError> {
    let mut value = 0usize;
    for byte in input {
        let digit = match byte {
            b'0'..=b'9' => usize::from(byte - b'0'),
            b'a'..=b'f' => usize::from(byte - b'a' + 10),
            b'A'..=b'F' => usize::from(byte - b'A' + 10),
            _ => return Err(ResponseError::Malformed),
        };
        value = value
            .checked_mul(16)
            .and_then(|value| value.checked_add(digit))
            .ok_or(ResponseError::Overflow)?;
        if value > MAX_BODY_BYTES {
            return Err(ResponseError::TooLarge);
        }
    }
    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_numeric_http_url_and_builds_request() {
        let url = Url::parse(b"http://10.0.2.2:8080/readme").unwrap();
        assert_eq!(url.address, [10, 0, 2, 2]);
        assert_eq!(url.port, 8080);
        let (request, length) = url.request().unwrap();
        assert_eq!(
            &request[..length],
            b"GET /readme HTTP/1.1\r\nHost: 10.0.2.2:8080\r\nConnection: close\r\n\r\n"
        );
    }

    #[test]
    fn parses_content_length_split_across_frames() {
        let mut parser = ResponseParser::new();
        parser.feed(b"HTTP/1.1 200 OK\r\nContent-L").unwrap();
        parser.feed(b"ength: 5\r\n\r\nhe").unwrap();
        parser.feed(b"llo").unwrap();
        assert!(parser.complete());
        assert_eq!(parser.body(), b"hello");
    }

    #[test]
    fn parses_chunked_body_and_rejects_extensions_and_trailers() {
        let mut parser = ResponseParser::new();
        parser.feed(b"HTTP/1.1 201 Created\r\nTransfer-Encoding: chunked\r\n\r\n4\r\nWiki\r\n5\r\npedia\r\n0\r\n\r\n").unwrap();
        assert_eq!(parser.body(), b"Wikipedia");
        let mut bad = ResponseParser::new();
        assert_eq!(
            bad.feed(b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n1;x\r\na\r\n0\r\n\r\n"),
            Err(ResponseError::Malformed)
        );
    }

    #[test]
    fn chunk_crlf_boundaries_may_split_between_frames() {
        let mut parser = ResponseParser::new();
        parser.feed(b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n1\r").unwrap();
        parser.feed(b"\na\r").unwrap();
        parser.feed(b"\n0\r").unwrap();
        parser.feed(b"\n\r").unwrap();
        parser.feed(b"\n").unwrap();
        assert!(parser.complete());
        assert_eq!(parser.body(), b"a");
    }
}
