use std::{
    env, fs,
    io::{self, Read, Write},
    net::{TcpStream, ToSocketAddrs},
    path::Path,
};

use logos_remote::{
    MAX_FRAME, MAX_FRAME_BUFFER, REMOTE_REPLY_BYTES, REMOTE_REQUEST_BYTES, RemoteReply,
    RemoteRequest, SESSION_ID_LEN, X25519, frame_decode, frame_encode,
};
use noise_protocol::{DH, HandshakeStateBuilder, patterns::noise_ik};
use rand_core::OsRng;
use x25519_dalek::StaticSecret;

const PROLOGUE: &[u8] = b"LogOS/remote/1";

fn main() {
    let mut args = env::args().skip(1);
    match args.next().as_deref() {
        Some("keygen") => keygen(args.next()),
        Some("invoke") => {
            let address = args.next().unwrap_or_else(|| "127.0.0.1:7443".into());
            let key = args.next().unwrap_or_else(|| "logosctl.key".into());
            let enrollment = args.next().unwrap_or_default();
            invoke(&address, Path::new(&key), &enrollment);
        }
        _ => eprintln!(
            "usage: logosctl keygen [file] | invoke [address] [key-file] [machine-key-hex:generation]"
        ),
    }
}

fn keygen(path: Option<String>) {
    let secret = StaticSecret::random_from_rng(OsRng);
    let mut text = hex(secret.to_bytes());
    text.push('\n');
    if let Some(path) = path {
        fs::write(path, text).expect("write key");
    } else {
        print!("{text}");
    }
}

fn invoke(address: &str, key_path: &Path, enrollment: &str) {
    let secret = parse_hex::<32>(&fs::read_to_string(key_path).expect("read key"));
    let (machine_hex, generation) =
        enrollment.split_once(':').expect("expected machine-key:generation");
    let machine = parse_hex::<32>(machine_hex);
    let generation = generation.parse::<u64>().expect("invalid enrollment generation");
    assert!(generation != 0, "invalid enrollment generation");
    let session = [1; SESSION_ID_LEN];
    for attempt in 0..3 {
        match connect_once(address, secret, machine, session, generation) {
            Ok(reply) => {
                println!(
                    "{}",
                    String::from_utf8_lossy(&reply.payload[..usize::from(reply.payload_length)])
                );
                return;
            }
            Err(error) if attempt < 2 => eprintln!("reconnect {}/3: {error}", attempt + 1),
            Err(error) => panic!("invoke failed: {error}"),
        }
    }
}

fn connect_once(
    address: &str,
    secret: [u8; 32],
    machine: [u8; 32],
    session: [u8; SESSION_ID_LEN],
    generation: u64,
) -> io::Result<RemoteReply> {
    let address = address.to_socket_addrs()?.next().ok_or_else(|| io::Error::other("address"))?;
    let mut stream = TcpStream::connect(address)?;
    stream.set_nodelay(true)?;
    let ephemeral = StaticSecret::random_from_rng(OsRng).to_bytes();
    let mut builder = HandshakeStateBuilder::<X25519>::new();
    builder
        .set_pattern(noise_ik())
        .set_is_initiator(true)
        .set_prologue(PROLOGUE)
        .set_s(secret)
        .set_e(ephemeral)
        .set_rs(X25519::pubkey(&machine));
    let mut handshake =
        builder.build_handshake_state::<logos_remote::NoiseChaCha, logos_remote::NoiseSha256>();
    let first = handshake.write_message_vec(&[]).map_err(|_| io::Error::other("noise"))?;
    write_frame(&mut stream, &first)?;
    let second = read_frame(&mut stream)?;
    let mut payload = [0; MAX_FRAME];
    handshake.read_message(&second, &mut payload).map_err(|_| io::Error::other("noise"))?;
    if !handshake.completed() {
        return Err(io::Error::other("noise incomplete"));
    }
    let (mut send, mut recv) = handshake.get_ciphers();
    let request = RemoteRequest::ping(generation, session, 1);
    let mut plaintext = [0; REMOTE_REQUEST_BYTES];
    request.encode(&mut plaintext).map_err(|_| io::Error::other("request"))?;
    let mut ciphertext = [0; REMOTE_REQUEST_BYTES + 16];
    send.encrypt(&plaintext, &mut ciphertext);
    write_frame(&mut stream, &ciphertext)?;
    let encrypted = read_frame(&mut stream)?;
    let mut reply_bytes = [0; REMOTE_REPLY_BYTES];
    recv.decrypt(&encrypted, &mut reply_bytes).map_err(|_| io::Error::other("reply auth"))?;
    RemoteReply::decode(&reply_bytes).map_err(|_| io::Error::other("reply"))
}

fn write_frame(stream: &mut TcpStream, payload: &[u8]) -> io::Result<()> {
    let mut frame = [0; MAX_FRAME_BUFFER];
    let length = frame_encode(&mut frame, payload).map_err(|_| io::Error::other("frame"))?;
    stream.write_all(&frame[..length])
}

fn read_frame(stream: &mut TcpStream) -> io::Result<Vec<u8>> {
    let mut header = [0; 2];
    stream.read_exact(&mut header)?;
    let length = usize::from(u16::from_be_bytes(header));
    if length == 0 || length > MAX_FRAME {
        return Err(io::Error::other("frame bound"));
    }
    let mut bytes = vec![0; length + 2];
    bytes[..2].copy_from_slice(&header);
    stream.read_exact(&mut bytes[2..])?;
    Ok(frame_decode(&bytes).map_err(|_| io::Error::other("frame"))?.to_vec())
}

fn parse_hex<const N: usize>(text: &str) -> [u8; N] {
    let bytes = text.trim().as_bytes();
    assert_eq!(bytes.len(), N * 2, "expected {} hex characters", N * 2);
    let mut output = [0; N];
    for (index, pair) in bytes.chunks_exact(2).enumerate() {
        output[index] = (hex_value(pair[0]) << 4) | hex_value(pair[1]);
    }
    output
}

fn hex_value(byte: u8) -> u8 {
    match byte {
        b'0'..=b'9' => byte - b'0',
        b'a'..=b'f' => byte - b'a' + 10,
        b'A'..=b'F' => byte - b'A' + 10,
        _ => panic!("invalid hex"),
    }
}

fn hex(bytes: [u8; 32]) -> String {
    const TABLE: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(64);
    for byte in bytes {
        output.push(TABLE[(byte >> 4) as usize] as char);
        output.push(TABLE[(byte & 0xf) as usize] as char);
    }
    output
}
