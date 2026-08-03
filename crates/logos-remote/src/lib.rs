#![no_std]

use chacha20poly1305::{AeadInPlace, ChaCha20Poly1305, KeyInit};
use noise_protocol::{Cipher, DH, Hash};
use sha2::Digest;
use x25519_dalek::{PublicKey, StaticSecret};

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
    Crypto,
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
        [0; 32]
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
