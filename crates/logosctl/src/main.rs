use std::{
    env, fs,
    io::{self, BufRead, Read, Write},
    net::{TcpStream, ToSocketAddrs},
    path::Path,
    sync::mpsc,
    time::Duration,
};

use logos_remote::{
    MAX_FRAME, MAX_FRAME_BUFFER, REMOTE_EVENT_CREDIT, REMOTE_MESSAGE_PAYLOAD, REMOTE_PROLOGUE,
    REMOTE_SUBSCRIBE_LOG, REMOTE_SUBSCRIBE_TRACE, RemoteCommand, RemoteInvocation, RemoteMessage,
    RemoteMessageKind, X25519, frame_decode, frame_encode,
};
use noise_protocol::{CipherState, DH, HandshakeStateBuilder, patterns::noise_ik};
use rand_core::{OsRng, RngCore};
use x25519_dalek::StaticSecret;

fn main() {
    let mut args = env::args().skip(1);
    match args.next().as_deref() {
        Some("keygen") => keygen(args.next()),
        Some("session") => {
            let address = args.next().unwrap_or_else(|| "127.0.0.1:7443".into());
            let key = args.next().unwrap_or_else(|| "logosctl.key".into());
            let enrollment = args.next().unwrap_or_default();
            session(&address, Path::new(&key), &enrollment);
        }
        _ => eprintln!(
            "usage: logosctl keygen [file] | session [address] [key-file] [machine-key-hex:generation]"
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

fn session(address: &str, key_path: &Path, enrollment: &str) {
    let secret = parse_hex::<32>(&fs::read_to_string(key_path).expect("read key"));
    let (machine_hex, generation) =
        enrollment.split_once(':').expect("expected machine-key:generation");
    let machine = parse_hex::<32>(machine_hex);
    assert!(generation.parse::<u64>().expect("invalid enrollment generation") != 0);
    let attachment = OsRng.next_u64().max(1);
    let mut sequence = 1u64;
    let mut cursor = 0u64;
    let (commands, input) = mpsc::channel();
    std::thread::spawn(move || {
        for line in io::stdin().lock().lines() {
            if commands.send(line).is_err() {
                break;
            }
        }
    });
    let mut transport: Option<Transport> = None;
    let mut pending: Option<Pending> = None;
    let mut following = false;
    let mut input_closed = false;
    loop {
        if pending.is_none() && !input_closed {
            match input.recv_timeout(Duration::from_millis(20)) {
                Ok(Ok(line)) => {
                    let line = line.trim();
                    if line.is_empty() {
                        continue;
                    }
                    if line == "quit" {
                        return;
                    }
                    let message = if let Some(source) = line.strip_prefix("follow ") {
                        let source = match source {
                            "trace" => REMOTE_SUBSCRIBE_TRACE,
                            "log" => REMOTE_SUBSCRIBE_LOG,
                            _ => panic!("expected follow trace|log"),
                        };
                        following = true;
                        wire_message(RemoteMessageKind::Subscribe, attachment, 0, cursor, source)
                    } else if line == "unfollow" {
                        following = false;
                        wire_message(RemoteMessageKind::Cancel, attachment, 0, 0, &[])
                    } else {
                        invocation(attachment, sequence, line)
                    };
                    pending = Some(Pending::new(message));
                }
                Ok(Err(error)) => panic!("read command: {error}"),
                Err(mpsc::RecvTimeoutError::Disconnected) => input_closed = true,
                Err(mpsc::RecvTimeoutError::Timeout) => {}
            }
        }
        let Some(current) = pending.as_mut() else {
            if input_closed {
                return;
            }
            continue;
        };
        if transport.is_none() {
            match Transport::connect(address, secret, machine) {
                Ok(value) => transport = Some(value),
                Err(error) => {
                    current.attempts = current.attempts.saturating_add(1);
                    if current.attempts >= 3 {
                        panic!("session reconnect failed: {error}");
                    }
                    continue;
                }
            }
            current.sent = false;
        }
        if !current.sent {
            if let Err(error) =
                transport.as_mut().expect("transport").send(&current.bytes[..current.length])
            {
                transport = None;
                current.attempts = current.attempts.saturating_add(1);
                if current.attempts >= 3 {
                    panic!("session send failed: {error}");
                }
                continue;
            }
            current.sent = true;
        }
        match transport.as_mut().expect("transport").try_read() {
            Ok(Some(reply)) => {
                let kind = current.message.kind;
                if reply.kind == RemoteMessageKind::Error {
                    println!(
                        "error: {}",
                        String::from_utf8_lossy(
                            &reply.payload[..usize::from(reply.payload_length)]
                        )
                    );
                } else {
                    println!(
                        "{}",
                        String::from_utf8_lossy(
                            &reply.payload[..usize::from(reply.payload_length)]
                        )
                    );
                }
                cursor = reply.cursor.max(cursor);
                if kind == RemoteMessageKind::Invoke {
                    sequence = sequence.wrapping_add(1).max(1);
                }
                pending = if following
                    && matches!(reply.kind, RemoteMessageKind::Subscribe | RemoteMessageKind::Event)
                {
                    Some(Pending::new(wire_message(
                        RemoteMessageKind::Credit,
                        attachment,
                        0,
                        REMOTE_EVENT_CREDIT,
                        &[],
                    )))
                } else {
                    None
                };
            }
            Ok(None) => {}
            Err(error) => {
                transport = None;
                current.sent = false;
                current.attempts = current.attempts.saturating_add(1);
                if current.attempts >= 3 {
                    panic!("session read failed: {error}");
                }
            }
        }
    }
}

struct Pending {
    message: RemoteMessage,
    bytes: [u8; MAX_FRAME],
    length: usize,
    sent: bool,
    attempts: u8,
}

impl Pending {
    fn new(message: RemoteMessage) -> Self {
        let mut bytes = [0; MAX_FRAME];
        let length = message.encode(&mut bytes).expect("encode message");
        Self { message, bytes, length, sent: false, attempts: 0 }
    }
}

fn invocation(attachment: u64, sequence: u64, line: &str) -> RemoteMessage {
    let (name, argument) = line.split_once(' ').map_or((line, ""), |parts| parts);
    let command = RemoteCommand::from_name(name.as_bytes()).expect("unknown remote command");
    let invocation = RemoteInvocation::new(command, argument.as_bytes()).expect("invalid argument");
    let mut payload = [0; REMOTE_MESSAGE_PAYLOAD];
    let length = invocation.encode(&mut payload).expect("encode invocation");
    RemoteMessage {
        kind: RemoteMessageKind::Invoke,
        id: attachment,
        sequence,
        cursor: 0,
        payload,
        payload_length: length as u16,
    }
}

fn wire_message(
    kind: RemoteMessageKind,
    id: u64,
    sequence: u64,
    cursor: u64,
    bytes: &[u8],
) -> RemoteMessage {
    let mut payload = [0; REMOTE_MESSAGE_PAYLOAD];
    payload[..bytes.len()].copy_from_slice(bytes);
    RemoteMessage { kind, id, sequence, cursor, payload, payload_length: bytes.len() as u16 }
}

struct Transport {
    stream: TcpStream,
    send: CipherState<logos_remote::NoiseChaCha>,
    receive: CipherState<logos_remote::NoiseChaCha>,
    input: Vec<u8>,
}

impl Transport {
    fn connect(address: &str, secret: [u8; 32], machine: [u8; 32]) -> io::Result<Self> {
        let address =
            address.to_socket_addrs()?.next().ok_or_else(|| io::Error::other("address"))?;
        let mut stream = TcpStream::connect_timeout(&address, Duration::from_secs(2))?;
        stream.set_nodelay(true)?;
        stream.set_read_timeout(Some(Duration::from_millis(500)))?;
        let mut ephemeral = [0; 32];
        OsRng.fill_bytes(&mut ephemeral);
        let mut builder = HandshakeStateBuilder::<X25519>::new();
        builder
            .set_pattern(noise_ik())
            .set_is_initiator(true)
            .set_prologue(REMOTE_PROLOGUE)
            .set_s(secret)
            .set_e(ephemeral)
            .set_rs(X25519::pubkey(&machine));
        let mut handshake =
            builder.build_handshake_state::<logos_remote::NoiseChaCha, logos_remote::NoiseSha256>();
        let first = handshake.write_message_vec(&[]).map_err(|_| io::Error::other("noise"))?;
        write_frame(&mut stream, &first)?;
        let second = read_frame(&mut stream)?;
        handshake.read_message(&second, &mut []).map_err(|_| io::Error::other("noise"))?;
        if !handshake.completed() {
            return Err(io::Error::other("noise incomplete"));
        }
        let (send, receive) = handshake.get_ciphers();
        stream.set_read_timeout(Some(Duration::from_millis(50)))?;
        Ok(Self { stream, send, receive, input: Vec::new() })
    }

    fn send(&mut self, plaintext: &[u8]) -> io::Result<()> {
        let mut ciphertext = vec![0; plaintext.len() + 16];
        self.send.encrypt(plaintext, &mut ciphertext);
        write_frame(&mut self.stream, &ciphertext)
    }

    fn try_read(&mut self) -> io::Result<Option<RemoteMessage>> {
        let mut bytes = [0; 1024];
        match self.stream.read(&mut bytes) {
            Ok(0) => return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "closed")),
            Ok(length) => self.input.extend_from_slice(&bytes[..length]),
            Err(error)
                if error.kind() == io::ErrorKind::WouldBlock
                    || error.kind() == io::ErrorKind::TimedOut =>
            {
                return Ok(None);
            }
            Err(error) => return Err(error),
        }
        if self.input.len() < 2 {
            return Ok(None);
        }
        let length = usize::from(u16::from_be_bytes([self.input[0], self.input[1]]));
        if length == 0 || length > MAX_FRAME {
            return Err(io::Error::other("frame bound"));
        }
        if self.input.len() < length + 2 {
            return Ok(None);
        }
        let frame = frame_decode(&self.input[..length + 2])
            .map_err(|_| io::Error::other("frame"))?
            .to_vec();
        self.input.drain(..length + 2);
        if frame.len() < 16 {
            return Err(io::Error::other("short reply"));
        }
        let mut plaintext = vec![0; frame.len() - 16];
        self.receive.decrypt(&frame, &mut plaintext).map_err(|_| io::Error::other("reply auth"))?;
        RemoteMessage::decode(&plaintext).map(Some).map_err(|_| io::Error::other("reply"))
    }
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
