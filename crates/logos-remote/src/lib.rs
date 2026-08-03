#![no_std]

pub const PROTOCOL_VERSION: u16 = 1;
pub const MAX_FRAME: usize = 1024;
pub const SESSION_ID_LEN: usize = 16;
pub const REQUEST_DIGEST_LEN: usize = 32;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Error {
    Frame,
    Stale,
    Mismatch,
    Busy,
}

pub fn frame_encode(output: &mut [u8], message: &[u8]) -> Result<usize, Error> {
    if message.is_empty() || message.len() > MAX_FRAME || output.len() < message.len() + 2 {
        return Err(Error::Frame);
    }
    output[..2].copy_from_slice(&(message.len() as u16).to_be_bytes());
    output[2..message.len() + 2].copy_from_slice(message);
    Ok(message.len() + 2)
}

pub fn frame_decode(frame: &[u8]) -> Result<&[u8], Error> {
    if frame.len() < 3 {
        return Err(Error::Frame);
    }
    let length = usize::from(u16::from_be_bytes([frame[0], frame[1]]));
    if length == 0 || length > MAX_FRAME || length + 2 != frame.len() {
        return Err(Error::Frame);
    }
    Ok(&frame[2..])
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Reply {
    Pong,
    Indeterminate,
    Denied,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Replay {
    Empty,
    Pending {
        session: [u8; SESSION_ID_LEN],
        sequence: u64,
        digest: [u8; REQUEST_DIGEST_LEN],
    },
    Complete {
        session: [u8; SESSION_ID_LEN],
        sequence: u64,
        digest: [u8; REQUEST_DIGEST_LEN],
        reply: Reply,
    },
}

impl Replay {
    pub fn begin(
        &mut self,
        session: [u8; SESSION_ID_LEN],
        sequence: u64,
        digest: [u8; REQUEST_DIGEST_LEN],
    ) -> Result<(), Error> {
        match *self {
            Self::Empty => {
                *self = Self::Pending { session, sequence, digest };
                Ok(())
            }
            Self::Pending { session: old_session, sequence: old_sequence, digest: old_digest }
            | Self::Complete {
                session: old_session,
                sequence: old_sequence,
                digest: old_digest,
                ..
            } if old_session == session && old_sequence == sequence && old_digest == digest => {
                Err(Error::Busy)
            }
            Self::Pending { session: old_session, sequence: old_sequence, .. }
            | Self::Complete { session: old_session, sequence: old_sequence, .. }
                if old_session == session && old_sequence == sequence =>
            {
                Err(Error::Mismatch)
            }
            Self::Pending { session: old_session, sequence: old_sequence, .. }
            | Self::Complete { session: old_session, sequence: old_sequence, .. }
                if old_session == session && sequence <= old_sequence =>
            {
                Err(Error::Stale)
            }
            _ => Err(Error::Mismatch),
        }
    }

    pub fn complete(&mut self, reply: Reply) -> Result<(), Error> {
        let Self::Pending { session, sequence, digest } = *self else { return Err(Error::Stale) };
        *self = Self::Complete { session, sequence, digest, reply };
        Ok(())
    }

    pub fn recover(
        &self,
        session: [u8; SESSION_ID_LEN],
        sequence: u64,
        digest: [u8; REQUEST_DIGEST_LEN],
    ) -> Result<Option<Reply>, Error> {
        match *self {
            Self::Empty => Ok(None),
            Self::Pending { session: old_session, sequence: old_sequence, digest: old_digest }
                if old_session == session && old_sequence == sequence && old_digest == digest =>
            {
                Ok(Some(Reply::Indeterminate))
            }
            Self::Complete {
                session: old_session,
                sequence: old_sequence,
                digest: old_digest,
                reply,
            } if old_session == session && old_sequence == sequence && old_digest == digest => {
                Ok(Some(reply))
            }
            Self::Pending { session: old_session, sequence: old_sequence, .. }
            | Self::Complete { session: old_session, sequence: old_sequence, .. }
                if old_session == session && old_sequence == sequence =>
            {
                Err(Error::Mismatch)
            }
            Self::Pending { session: old_session, sequence: old_sequence, .. }
            | Self::Complete { session: old_session, sequence: old_sequence, .. }
                if old_session == session && sequence <= old_sequence =>
            {
                Err(Error::Stale)
            }
            _ => Err(Error::Mismatch),
        }
    }
}

#[cfg(test)]
extern crate std;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frame_is_exact_and_bounded() {
        let mut output = [0; MAX_FRAME + 2];
        let length = frame_encode(&mut output, b"ping").unwrap();
        assert_eq!(frame_decode(&output[..length]), Ok(&b"ping"[..]));
        assert_eq!(frame_decode(&output[..length - 1]), Err(Error::Frame));
        assert_eq!(frame_encode(&mut output, &[]), Err(Error::Frame));
    }

    #[test]
    fn pending_is_never_reexecuted_after_reboot() {
        let session = [1; SESSION_ID_LEN];
        let digest = [2; REQUEST_DIGEST_LEN];
        let mut replay = Replay::Empty;
        replay.begin(session, 1, digest).unwrap();
        assert_eq!(replay.recover(session, 1, digest), Ok(Some(Reply::Indeterminate)));
        replay.complete(Reply::Pong).unwrap();
        assert_eq!(replay.recover(session, 1, digest), Ok(Some(Reply::Pong)));
        assert_eq!(replay.begin(session, 1, digest), Err(Error::Busy));
        assert_eq!(replay.recover(session, 1, [3; REQUEST_DIGEST_LEN]), Err(Error::Mismatch));
    }
}
