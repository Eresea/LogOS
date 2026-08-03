#![no_std]

use chacha20poly1305::{AeadInPlace, ChaCha20Poly1305, KeyInit, XChaCha20Poly1305};
use hkdf::Hkdf;
use noise_protocol::{Cipher, DH, Hash};
use sha2::Digest;
use x25519_dalek::{PublicKey, StaticSecret};

pub const PROTOCOL_VERSION: u16 = 1;
pub const MAX_FRAME: usize = 1024;
pub const SESSION_ID_LEN: usize = 16;
pub const REQUEST_DIGEST_LEN: usize = 32;
pub const MAX_FRAME_BUFFER: usize = MAX_FRAME + 2;
pub const ENROLLMENT_BYTES: usize = 42;
pub const SESSION_RECORD_BYTES: usize = 324;
pub const MAX_COMMAND: usize = 256;
pub const REMOTE_REQUEST_BYTES: usize = 2 + 8 + SESSION_ID_LEN + 8 + 2 + MAX_COMMAND;
pub const REMOTE_REPLY_BYTES: usize = 2 + 8 + 8 + 1 + 2 + MAX_COMMAND;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Error {
    Frame,
    Stale,
    Mismatch,
    Busy,
    Crypto,
    Invalid,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Enrollment {
    pub generation: u64,
    pub active: bool,
    pub client_key: [u8; 32],
}

impl Enrollment {
    pub fn encode(self, output: &mut [u8; ENROLLMENT_BYTES]) {
        *output = [0; ENROLLMENT_BYTES];
        output[0] = 1;
        output[1] = u8::from(self.active);
        output[2..10].copy_from_slice(&self.generation.to_be_bytes());
        output[10..].copy_from_slice(&self.client_key);
    }

    pub fn decode(input: &[u8]) -> Result<Self, Error> {
        if input.len() != ENROLLMENT_BYTES || input[0] != 1 || input[1] > 1 {
            return Err(Error::Frame);
        }
        let mut client_key = [0; 32];
        client_key.copy_from_slice(&input[10..]);
        Ok(Self {
            generation: u64::from_be_bytes(input[2..10].try_into().map_err(|_| Error::Frame)?),
            active: input[1] == 1,
            client_key,
        })
    }
}

/// Bounded machine trust state. A failed protected-record load is represented
/// by `available = false`; callers must not fall back to an older record.
pub struct TrustState {
    machine_secret: [u8; 32],
    machine_public: [u8; 32],
    enrollment: Enrollment,
    available: bool,
}

impl TrustState {
    pub fn new(machine_secret: [u8; 32]) -> Result<Self, Error> {
        if machine_secret.iter().all(|byte| *byte == 0) {
            return Err(Error::Invalid);
        }
        let machine_public = X25519::pubkey(&machine_secret);
        if machine_public.iter().all(|byte| *byte == 0) {
            return Err(Error::Invalid);
        }
        Ok(Self {
            machine_secret,
            machine_public,
            enrollment: Enrollment { generation: 1, active: false, client_key: [0; 32] },
            available: true,
        })
    }

    pub fn unavailable(machine_secret: [u8; 32]) -> Self {
        Self {
            machine_public: X25519::pubkey(&machine_secret),
            machine_secret,
            enrollment: Enrollment { generation: 0, active: false, client_key: [0; 32] },
            available: false,
        }
    }

    pub fn machine_public(&self) -> [u8; 32] {
        self.machine_public
    }

    pub fn enrollment(&self) -> Enrollment {
        self.enrollment
    }

    pub const fn available(&self) -> bool {
        self.available
    }

    pub fn enroll(&mut self, client_key: [u8; 32]) -> Result<u64, Error> {
        if !self.available || !valid_public_key(&client_key) {
            return Err(Error::Invalid);
        }
        self.enrollment.generation = self.enrollment.generation.saturating_add(1).max(1);
        self.enrollment.client_key = client_key;
        self.enrollment.active = true;
        Ok(self.enrollment.generation)
    }

    pub fn unenroll(&mut self) -> Result<u64, Error> {
        if !self.available {
            return Err(Error::Invalid);
        }
        self.enrollment.generation = self.enrollment.generation.saturating_add(1).max(1);
        self.enrollment.client_key = [0; 32];
        self.enrollment.active = false;
        Ok(self.enrollment.generation)
    }

    pub fn authorizes(&self, client_key: &[u8; 32], generation: u64) -> bool {
        self.available
            && self.enrollment.active
            && self.enrollment.generation == generation
            && &self.enrollment.client_key == client_key
    }

    pub fn seal_enrollment(
        &self,
        storage_key: &[u8; 32],
        nonce: &[u8; 24],
        output: &mut [u8; ENROLLMENT_BYTES + 16],
    ) -> Result<(), Error> {
        self.enrollment
            .encode((&mut output[..ENROLLMENT_BYTES]).try_into().map_err(|_| Error::Frame)?);
        seal(storage_key, nonce, b"enrollment", output, ENROLLMENT_BYTES)?;
        Ok(())
    }

    pub fn open_enrollment(
        machine_secret: [u8; 32],
        storage_key: &[u8; 32],
        nonce: &[u8; 24],
        input: &mut [u8; ENROLLMENT_BYTES + 16],
    ) -> Self {
        let Ok(mut state) = Self::new(machine_secret) else {
            return Self::unavailable(machine_secret);
        };
        let input_length = input.len();
        let Ok(length) = open(storage_key, nonce, b"enrollment", input, input_length) else {
            return Self::unavailable(machine_secret);
        };
        let Ok(enrollment) = Enrollment::decode(&input[..length]) else {
            return Self::unavailable(machine_secret);
        };
        if enrollment.active && !valid_public_key(&enrollment.client_key) {
            return Self::unavailable(machine_secret);
        }
        state.enrollment = enrollment;
        state
    }

    pub fn machine_secret(&self) -> [u8; 32] {
        self.machine_secret
    }
}

fn valid_public_key(key: &[u8; 32]) -> bool {
    !key.iter().all(|byte| *byte == 0) && X25519::dh(&[1; 32], key).is_ok()
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SessionRecord {
    pub enrollment_generation: u64,
    pub session: [u8; SESSION_ID_LEN],
    pub sequence: u64,
    pub pending: bool,
    pub digest: [u8; REQUEST_DIGEST_LEN],
    pub reply: [u8; 256],
    pub reply_length: u16,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum RemoteStatus {
    Ok = 1,
    Denied,
    Indeterminate,
    Invalid,
}

impl RemoteStatus {
    pub fn from_wire(value: u8) -> Result<Self, Error> {
        match value {
            1 => Ok(Self::Ok),
            2 => Ok(Self::Denied),
            3 => Ok(Self::Indeterminate),
            4 => Ok(Self::Invalid),
            _ => Err(Error::Frame),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RemoteRequest {
    pub version: u16,
    pub enrollment_generation: u64,
    pub session: [u8; SESSION_ID_LEN],
    pub sequence: u64,
    pub command: [u8; MAX_COMMAND],
    pub command_length: u16,
}

impl RemoteRequest {
    pub fn ping(enrollment_generation: u64, session: [u8; SESSION_ID_LEN], sequence: u64) -> Self {
        let mut command = [0; MAX_COMMAND];
        command[..4].copy_from_slice(b"ping");
        Self {
            version: PROTOCOL_VERSION,
            enrollment_generation,
            session,
            sequence,
            command,
            command_length: 4,
        }
    }

    pub fn encode(self, output: &mut [u8; REMOTE_REQUEST_BYTES]) -> Result<(), Error> {
        if self.version != PROTOCOL_VERSION
            || self.command_length == 0
            || self.command_length as usize > MAX_COMMAND
            || self.session.iter().all(|byte| *byte == 0)
            || self.sequence == 0
        {
            return Err(Error::Frame);
        }
        *output = [0; REMOTE_REQUEST_BYTES];
        output[0..2].copy_from_slice(&self.version.to_be_bytes());
        output[2..10].copy_from_slice(&self.enrollment_generation.to_be_bytes());
        output[10..26].copy_from_slice(&self.session);
        output[26..34].copy_from_slice(&self.sequence.to_be_bytes());
        output[34..36].copy_from_slice(&self.command_length.to_be_bytes());
        output[36..].copy_from_slice(&self.command);
        Ok(())
    }

    pub fn decode(input: &[u8]) -> Result<Self, Error> {
        if input.len() != REMOTE_REQUEST_BYTES {
            return Err(Error::Frame);
        }
        let mut session = [0; SESSION_ID_LEN];
        let mut command = [0; MAX_COMMAND];
        session.copy_from_slice(&input[10..26]);
        command.copy_from_slice(&input[36..]);
        let request = Self {
            version: u16::from_be_bytes(input[0..2].try_into().map_err(|_| Error::Frame)?),
            enrollment_generation: u64::from_be_bytes(
                input[2..10].try_into().map_err(|_| Error::Frame)?,
            ),
            session,
            sequence: u64::from_be_bytes(input[26..34].try_into().map_err(|_| Error::Frame)?),
            command,
            command_length: u16::from_be_bytes(input[34..36].try_into().map_err(|_| Error::Frame)?),
        };
        let mut encoded = [0; REMOTE_REQUEST_BYTES];
        request.encode(&mut encoded)?;
        (encoded == input).then_some(request).ok_or(Error::Frame)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RemoteReply {
    pub version: u16,
    pub enrollment_generation: u64,
    pub sequence: u64,
    pub status: RemoteStatus,
    pub payload: [u8; MAX_COMMAND],
    pub payload_length: u16,
}

impl RemoteReply {
    pub fn pong(enrollment_generation: u64, sequence: u64) -> Self {
        let mut payload = [0; MAX_COMMAND];
        payload[..4].copy_from_slice(b"pong");
        Self {
            version: PROTOCOL_VERSION,
            enrollment_generation,
            sequence,
            status: RemoteStatus::Ok,
            payload,
            payload_length: 4,
        }
    }

    pub fn encode(self, output: &mut [u8; REMOTE_REPLY_BYTES]) -> Result<(), Error> {
        if self.version != PROTOCOL_VERSION
            || self.sequence == 0
            || self.payload_length as usize > MAX_COMMAND
        {
            return Err(Error::Frame);
        }
        *output = [0; REMOTE_REPLY_BYTES];
        output[0..2].copy_from_slice(&self.version.to_be_bytes());
        output[2..10].copy_from_slice(&self.enrollment_generation.to_be_bytes());
        output[10..18].copy_from_slice(&self.sequence.to_be_bytes());
        output[18] = self.status as u8;
        output[19..21].copy_from_slice(&self.payload_length.to_be_bytes());
        output[21..].copy_from_slice(&self.payload);
        Ok(())
    }

    pub fn decode(input: &[u8]) -> Result<Self, Error> {
        if input.len() != REMOTE_REPLY_BYTES {
            return Err(Error::Frame);
        }
        let mut payload = [0; MAX_COMMAND];
        payload.copy_from_slice(&input[21..]);
        let reply = Self {
            version: u16::from_be_bytes(input[0..2].try_into().map_err(|_| Error::Frame)?),
            enrollment_generation: u64::from_be_bytes(
                input[2..10].try_into().map_err(|_| Error::Frame)?,
            ),
            sequence: u64::from_be_bytes(input[10..18].try_into().map_err(|_| Error::Frame)?),
            status: RemoteStatus::from_wire(input[18])?,
            payload,
            payload_length: u16::from_be_bytes(input[19..21].try_into().map_err(|_| Error::Frame)?),
        };
        let mut encoded = [0; REMOTE_REPLY_BYTES];
        reply.encode(&mut encoded)?;
        (encoded == input).then_some(reply).ok_or(Error::Frame)
    }
}

impl SessionRecord {
    pub fn encode(self, output: &mut [u8; SESSION_RECORD_BYTES]) -> Result<(), Error> {
        if self.reply_length as usize > self.reply.len() {
            return Err(Error::Frame);
        }
        *output = [0; SESSION_RECORD_BYTES];
        output[0] = 1;
        output[1..9].copy_from_slice(&self.enrollment_generation.to_be_bytes());
        output[9..25].copy_from_slice(&self.session);
        output[25..33].copy_from_slice(&self.sequence.to_be_bytes());
        output[33] = u8::from(self.pending);
        output[34..66].copy_from_slice(&self.digest);
        output[66..68].copy_from_slice(&self.reply_length.to_be_bytes());
        output[68..].copy_from_slice(&self.reply);
        Ok(())
    }

    pub fn decode(input: &[u8]) -> Result<Self, Error> {
        if input.len() != SESSION_RECORD_BYTES || input[0] != 1 || input[33] > 1 {
            return Err(Error::Frame);
        }
        let reply_length = u16::from_be_bytes(input[66..68].try_into().map_err(|_| Error::Frame)?);
        if reply_length as usize > 256 {
            return Err(Error::Frame);
        }
        let mut session = [0; SESSION_ID_LEN];
        let mut digest = [0; REQUEST_DIGEST_LEN];
        let mut reply = [0; 256];
        session.copy_from_slice(&input[9..25]);
        digest.copy_from_slice(&input[34..66]);
        reply.copy_from_slice(&input[68..]);
        Ok(Self {
            enrollment_generation: u64::from_be_bytes(
                input[1..9].try_into().map_err(|_| Error::Frame)?,
            ),
            session,
            sequence: u64::from_be_bytes(input[25..33].try_into().map_err(|_| Error::Frame)?),
            pending: input[33] == 1,
            digest,
            reply,
            reply_length,
        })
    }
}

pub fn derive_keys(root: &[u8; 32]) -> Result<([u8; 32], [u8; 32]), Error> {
    let hkdf = Hkdf::<sha2::Sha256>::new(None, root);
    let mut device = [0; 32];
    let mut storage = [0; 32];
    hkdf.expand(b"LogOS remote device v1", &mut device).map_err(|_| Error::Crypto)?;
    hkdf.expand(b"LogOS remote storage v1", &mut storage).map_err(|_| Error::Crypto)?;
    Ok((device, storage))
}

pub fn machine_public(secret: &[u8; 32]) -> [u8; 32] {
    X25519::pubkey(secret)
}

pub fn seal(
    key: &[u8; 32],
    nonce: &[u8; 24],
    associated_data: &[u8],
    buffer: &mut [u8],
    plaintext_len: usize,
) -> Result<usize, Error> {
    let ciphertext_len = plaintext_len.checked_add(16).ok_or(Error::Crypto)?;
    if ciphertext_len > buffer.len() {
        return Err(Error::Crypto);
    }
    let (plaintext, tag) = buffer[..ciphertext_len].split_at_mut(plaintext_len);
    let tag_value = XChaCha20Poly1305::new(key.into())
        .encrypt_in_place_detached(nonce.into(), associated_data, plaintext)
        .map_err(|_| Error::Crypto)?;
    tag.copy_from_slice(tag_value.as_slice());
    Ok(ciphertext_len)
}

pub fn open(
    key: &[u8; 32],
    nonce: &[u8; 24],
    associated_data: &[u8],
    buffer: &mut [u8],
    ciphertext_len: usize,
) -> Result<usize, Error> {
    if ciphertext_len < 16 || ciphertext_len > buffer.len() {
        return Err(Error::Crypto);
    }
    let (ciphertext, tag) = buffer[..ciphertext_len].split_at_mut(ciphertext_len - 16);
    XChaCha20Poly1305::new(key.into())
        .decrypt_in_place_detached(nonce.into(), associated_data, ciphertext, tag.as_ref().into())
        .map_err(|_| Error::Crypto)?;
    Ok(ciphertext.len())
}

pub enum X25519 {}

impl DH for X25519 {
    type Key = [u8; 32];
    type Pubkey = [u8; 32];
    type Output = [u8; 32];

    fn name() -> &'static str {
        "25519"
    }

    fn genkey() -> Self::Key {
        panic!("Noise ephemeral keys must be supplied from firmware entropy")
    }

    fn pubkey(key: &Self::Key) -> Self::Pubkey {
        *PublicKey::from(&StaticSecret::from(*key)).as_bytes()
    }

    fn dh(key: &Self::Key, public: &Self::Pubkey) -> Result<Self::Output, ()> {
        let shared = StaticSecret::from(*key).diffie_hellman(&PublicKey::from(*public));
        let output = *shared.as_bytes();
        (!output.iter().all(|byte| *byte == 0)).then_some(output).ok_or(())
    }
}

pub enum NoiseChaCha {}

impl Cipher for NoiseChaCha {
    fn name() -> &'static str {
        "ChaChaPoly"
    }

    type Key = [u8; 32];

    fn encrypt(key: &Self::Key, nonce: u64, ad: &[u8], plaintext: &[u8], output: &mut [u8]) {
        assert_eq!(output.len(), plaintext.len() + Self::tag_len());
        output[..plaintext.len()].copy_from_slice(plaintext);
        let _ = Self::encrypt_in_place(key, nonce, ad, output, plaintext.len());
    }

    fn encrypt_in_place(
        key: &Self::Key,
        nonce: u64,
        ad: &[u8],
        output: &mut [u8],
        plaintext_len: usize,
    ) -> usize {
        assert!(plaintext_len + Self::tag_len() <= output.len());
        let mut nonce_bytes = [0; 12];
        nonce_bytes[4..].copy_from_slice(&nonce.to_le_bytes());
        let (plaintext, tag) =
            output[..plaintext_len + Self::tag_len()].split_at_mut(plaintext_len);
        let tag_value = ChaCha20Poly1305::new(key.into())
            .encrypt_in_place_detached((&nonce_bytes).into(), ad, plaintext)
            .expect("fixed Noise buffer");
        tag.copy_from_slice(tag_value.as_slice());
        plaintext_len + Self::tag_len()
    }

    fn decrypt(
        key: &Self::Key,
        nonce: u64,
        ad: &[u8],
        ciphertext: &[u8],
        output: &mut [u8],
    ) -> Result<(), ()> {
        if ciphertext.len() < Self::tag_len() || output.len() != ciphertext.len() - Self::tag_len()
        {
            return Err(());
        }
        let mut nonce_bytes = [0; 12];
        nonce_bytes[4..].copy_from_slice(&nonce.to_le_bytes());
        output.copy_from_slice(&ciphertext[..output.len()]);
        ChaCha20Poly1305::new(key.into())
            .decrypt_in_place_detached(
                (&nonce_bytes).into(),
                ad,
                output,
                ciphertext[output.len()..].into(),
            )
            .map_err(|_| ())?;
        Ok(())
    }

    fn decrypt_in_place(
        key: &Self::Key,
        nonce: u64,
        ad: &[u8],
        output: &mut [u8],
        ciphertext_len: usize,
    ) -> Result<usize, ()> {
        if ciphertext_len < Self::tag_len() || ciphertext_len > output.len() {
            return Err(());
        }
        let mut nonce_bytes = [0; 12];
        nonce_bytes[4..].copy_from_slice(&nonce.to_le_bytes());
        let (ciphertext, tag) =
            output[..ciphertext_len].split_at_mut(ciphertext_len - Self::tag_len());
        ChaCha20Poly1305::new(key.into())
            .decrypt_in_place_detached((&nonce_bytes).into(), ad, ciphertext, tag.as_ref().into())
            .map_err(|_| ())?;
        Ok(ciphertext.len())
    }
}

#[derive(Clone, Default)]
pub struct NoiseSha256(sha2::Sha256);

impl Hash for NoiseSha256 {
    fn name() -> &'static str {
        "SHA256"
    }

    type Block = [u8; 64];
    type Output = [u8; 32];

    fn input(&mut self, data: &[u8]) {
        self.0.update(data);
    }

    fn result(&mut self) -> Self::Output {
        self.0.finalize_reset().into()
    }
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

pub struct FrameDecoder {
    bytes: [u8; MAX_FRAME_BUFFER],
    length: usize,
}

impl FrameDecoder {
    pub const fn new() -> Self {
        Self { bytes: [0; MAX_FRAME_BUFFER], length: 0 }
    }

    pub fn push(&mut self, chunk: &[u8]) -> Result<(), Error> {
        let end = self.length.checked_add(chunk.len()).ok_or(Error::Frame)?;
        if end > self.bytes.len() {
            return Err(Error::Frame);
        }
        self.bytes[self.length..end].copy_from_slice(chunk);
        self.length = end;
        if self.length >= 2 {
            let declared = usize::from(u16::from_be_bytes([self.bytes[0], self.bytes[1]]));
            if declared == 0 || declared > MAX_FRAME || declared + 2 > self.bytes.len() {
                return Err(Error::Frame);
            }
        }
        Ok(())
    }

    pub fn ready(&self) -> Result<Option<&[u8]>, Error> {
        if self.length < 2 {
            return Ok(None);
        }
        let declared = usize::from(u16::from_be_bytes([self.bytes[0], self.bytes[1]]));
        if self.length < declared + 2 {
            return Ok(None);
        }
        Ok(Some(&self.bytes[2..declared + 2]))
    }

    pub fn consume(&mut self) -> Result<(), Error> {
        let Some(frame) = self.ready()? else { return Err(Error::Frame) };
        let consumed = frame.len() + 2;
        self.bytes.copy_within(consumed..self.length, 0);
        self.length -= consumed;
        Ok(())
    }
}

impl Default for FrameDecoder {
    fn default() -> Self {
        Self::new()
    }
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
    use noise_protocol::{HandshakeStateBuilder, patterns::noise_ik};

    #[test]
    fn frame_is_exact_and_bounded() {
        let mut output = [0; MAX_FRAME + 2];
        let length = frame_encode(&mut output, b"ping").unwrap();
        assert_eq!(frame_decode(&output[..length]), Ok(&b"ping"[..]));
        assert_eq!(frame_decode(&output[..length - 1]), Err(Error::Frame));
        assert_eq!(frame_encode(&mut output, &[]), Err(Error::Frame));
    }

    #[test]
    fn typed_remote_contracts_are_exact_and_versioned() {
        let request = RemoteRequest::ping(4, [1; SESSION_ID_LEN], 9);
        let mut request_bytes = [0; REMOTE_REQUEST_BYTES];
        request.encode(&mut request_bytes).unwrap();
        assert_eq!(RemoteRequest::decode(&request_bytes), Ok(request));
        request_bytes[1] = 0;
        assert_eq!(RemoteRequest::decode(&request_bytes), Err(Error::Frame));
        let reply = RemoteReply::pong(4, 9);
        let mut reply_bytes = [0; REMOTE_REPLY_BYTES];
        reply.encode(&mut reply_bytes).unwrap();
        assert_eq!(RemoteReply::decode(&reply_bytes), Ok(reply));
        reply_bytes[18] = 0;
        assert_eq!(RemoteReply::decode(&reply_bytes), Err(Error::Frame));
    }

    #[test]
    fn protected_records_reject_tampering() {
        let (device, storage) = derive_keys(&[7; 32]).unwrap();
        assert_ne!(device, storage);
        let mut record = [0; 32];
        record[..4].copy_from_slice(b"ping");
        let length = seal(&storage, &[9; 24], b"enrollment", &mut record, 4).unwrap();
        assert_eq!(open(&storage, &[9; 24], b"enrollment", &mut record, length).unwrap(), 4);
        record[0] ^= 1;
        assert_eq!(
            open(&storage, &[9; 24], b"enrollment", &mut record, length),
            Err(Error::Crypto)
        );
    }

    #[test]
    fn records_and_partial_frames_are_exact() {
        let enrollment = Enrollment { generation: 3, active: true, client_key: [4; 32] };
        let mut enrollment_bytes = [0; ENROLLMENT_BYTES];
        enrollment.encode(&mut enrollment_bytes);
        assert_eq!(Enrollment::decode(&enrollment_bytes), Ok(enrollment));
        let record = SessionRecord {
            enrollment_generation: 3,
            session: [5; SESSION_ID_LEN],
            sequence: 7,
            pending: true,
            digest: [6; REQUEST_DIGEST_LEN],
            reply: [8; 256],
            reply_length: 4,
        };
        let mut record_bytes = [0; SESSION_RECORD_BYTES];
        record.encode(&mut record_bytes).unwrap();
        assert_eq!(SessionRecord::decode(&record_bytes), Ok(record));
        let mut encoded = [0; MAX_FRAME_BUFFER];
        let length = frame_encode(&mut encoded, b"hello").unwrap();
        let mut decoder = FrameDecoder::new();
        decoder.push(&encoded[..2]).unwrap();
        assert_eq!(decoder.ready(), Ok(None));
        decoder.push(&encoded[2..length]).unwrap();
        assert_eq!(decoder.ready(), Ok(Some(&b"hello"[..])));
        decoder.consume().unwrap();
        assert_eq!(decoder.ready(), Ok(None));
        let mut joined = [0; MAX_FRAME_BUFFER];
        let first = frame_encode(&mut joined, b"a").unwrap();
        let mut second = [0; MAX_FRAME_BUFFER];
        let second_len = frame_encode(&mut second, b"b").unwrap();
        joined[first..first + second_len].copy_from_slice(&second[..second_len]);
        let mut joined_decoder = FrameDecoder::new();
        joined_decoder.push(&joined[..first + second_len]).unwrap();
        assert_eq!(joined_decoder.ready(), Ok(Some(&b"a"[..])));
        joined_decoder.consume().unwrap();
        assert_eq!(joined_decoder.ready(), Ok(Some(&b"b"[..])));
    }

    #[test]
    fn trust_generation_and_corruption_are_fail_closed() {
        let mut state = TrustState::new([7; 32]).unwrap();
        assert!(!state.authorizes(&[8; 32], 1));
        let generation = state.enroll([8; 32]).unwrap();
        assert!(state.authorizes(&[8; 32], generation));
        assert!(!state.authorizes(&[8; 32], generation - 1));
        let storage = derive_keys(&[9; 32]).unwrap().1;
        let mut sealed = [0; ENROLLMENT_BYTES + 16];
        state.seal_enrollment(&storage, &[3; 24], &mut sealed).unwrap();
        let loaded = TrustState::open_enrollment([7; 32], &storage, &[3; 24], &mut sealed);
        assert!(loaded.available() && loaded.authorizes(&[8; 32], generation));
        sealed[0] ^= 1;
        assert!(!TrustState::open_enrollment([7; 32], &storage, &[3; 24], &mut sealed).available());
        assert!(matches!(TrustState::new([0; 32]), Err(Error::Invalid)));
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

    #[test]
    fn noise_ik_completes_with_explicit_entropy() {
        let machine = [3; 32];
        let mut initiator = HandshakeStateBuilder::<X25519>::new();
        initiator
            .set_pattern(noise_ik())
            .set_is_initiator(true)
            .set_prologue(b"LogOS/remote/1")
            .set_s([1; 32])
            .set_e([2; 32])
            .set_rs(X25519::pubkey(&machine));
        let mut responder = HandshakeStateBuilder::<X25519>::new();
        responder
            .set_pattern(noise_ik())
            .set_is_initiator(false)
            .set_prologue(b"LogOS/remote/1")
            .set_s(machine)
            .set_e([4; 32]);
        let mut initiator = initiator.build_handshake_state::<NoiseChaCha, NoiseSha256>();
        let mut responder = responder.build_handshake_state::<NoiseChaCha, NoiseSha256>();
        let mut first = [0; 96];
        initiator.write_message(&[], &mut first).unwrap();
        responder.read_message(&first, &mut []).unwrap();
        let mut second = [0; 48];
        responder.write_message(&[], &mut second).unwrap();
        initiator.read_message(&second, &mut []).unwrap();
        assert!(initiator.completed() && responder.completed());
    }
}
