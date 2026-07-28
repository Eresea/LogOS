pub const MAX_FRAME: usize = 256;

#[derive(Debug, PartialEq, Eq)]
pub enum Request<'a> {
    Hello,
    Run(&'a str),
    Inject { point: &'a str, action: &'a str },
    Input(&'a str),
    Query(&'a str),
    Advance(u64),
    Reset(&'a str),
    Shutdown,
}

#[derive(Debug, PartialEq, Eq)]
pub enum Error {
    TooLong,
    InvalidUtf8,
    BadVersion,
    Malformed,
    Unknown,
}

pub fn parse(frame: &[u8]) -> Result<Request<'_>, Error> {
    if frame.len() > MAX_FRAME {
        return Err(Error::TooLong);
    }
    let line = core::str::from_utf8(frame).map_err(|_| Error::InvalidUtf8)?.trim();
    let Some(rest) = line.strip_prefix("LOGOS/1 ") else { return Err(Error::BadVersion) };
    if let Some(value) = rest.strip_prefix("INPUT ") {
        return (!value.trim().is_empty())
            .then_some(Request::Input(value.trim()))
            .ok_or(Error::Malformed);
    }
    let mut words = rest.split_ascii_whitespace();
    match words.next() {
        Some("HELLO") if words.next().is_none() => Ok(Request::Hello),
        Some("RUN") => one(words).map(Request::Run),
        Some("INJECT") => Ok(Request::Inject {
            point: words.next().ok_or(Error::Malformed)?,
            action: one(words)?,
        }),
        Some("QUERY") => one(words).map(Request::Query),
        Some("ADVANCE") => one(words)?.parse().map(Request::Advance).map_err(|_| Error::Malformed),
        Some("RESET") => one(words).map(Request::Reset),
        Some("SHUTDOWN") if words.next().is_none() => Ok(Request::Shutdown),
        Some(_) => Err(Error::Unknown),
        None => Err(Error::Malformed),
    }
}

fn one(mut words: core::str::SplitAsciiWhitespace<'_>) -> Result<&str, Error> {
    let value = words.next().ok_or(Error::Malformed)?;
    if words.next().is_some() { Err(Error::Malformed) } else { Ok(value) }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn parses_and_rejects_bounded_frames() {
        assert_eq!(parse(b"LOGOS/1 RUN core/boot-normal"), Ok(Request::Run("core/boot-normal")));
        assert_eq!(parse(b"LOGOS/1 INPUT echo hello"), Ok(Request::Input("echo hello")));
        assert_eq!(parse(b"LOGOS/2 HELLO"), Err(Error::BadVersion));
        assert_eq!(parse(&[b'x'; MAX_FRAME + 1]), Err(Error::TooLong));
        assert_eq!(parse(b"LOGOS/1 ADVANCE nope"), Err(Error::Malformed));
    }
}
