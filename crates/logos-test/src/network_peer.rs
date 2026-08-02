use std::net::{IpAddr, Ipv4Addr, SocketAddr, UdpSocket};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use std::thread::{self, JoinHandle};
use std::time::Duration;

const GUEST: Ipv4Addr = Ipv4Addr::new(10, 0, 2, 15);
const SERVER: Ipv4Addr = Ipv4Addr::new(10, 0, 2, 2);

pub struct NetworkPeer {
    stop: Arc<AtomicBool>,
    worker: Option<JoinHandle<()>>,
    pub qemu_port: u16,
    pub peer_port: u16,
}

impl NetworkPeer {
    pub fn start() -> Result<Self, String> {
        let socket =
            UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).map_err(|error| error.to_string())?;
        socket
            .set_read_timeout(Some(Duration::from_millis(100)))
            .map_err(|error| error.to_string())?;
        let peer_port = socket.local_addr().map_err(|error| error.to_string())?.port();
        let qemu_port = UdpSocket::bind((Ipv4Addr::LOCALHOST, 0))
            .map_err(|error| error.to_string())?
            .local_addr()
            .map_err(|error| error.to_string())?
            .port();
        let qemu = SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), qemu_port);
        let stop = Arc::new(AtomicBool::new(false));
        let worker_stop = Arc::clone(&stop);
        let worker = thread::spawn(move || {
            let mut input = [0; 2048];
            let mut drop_first_discover = true;
            while !worker_stop.load(Ordering::Relaxed) {
                let Ok((length, _)) = socket.recv_from(&mut input) else { continue };
                if drop_first_discover && dhcp_message_type(&input[..length]) == Some(1) {
                    drop_first_discover = false;
                    continue;
                }
                if let Some(frame) = dhcp_reply(&input[..length]) {
                    let _ = socket.send_to(&frame, qemu);
                }
            }
        });
        Ok(Self { stop, worker: Some(worker), qemu_port, peer_port })
    }
}

impl Drop for NetworkPeer {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

fn dhcp_message_type(frame: &[u8]) -> Option<u8> {
    if frame.len() < 14 + 20 + 8 + 240 || u16::from_be_bytes([frame[12], frame[13]]) != 0x0800 {
        return None;
    }
    let ip = &frame[14..];
    let ihl = usize::from(ip[0] & 0x0f) * 4;
    if ip[0] >> 4 != 4 || ihl < 20 || ip.len() < ihl + 8 || ip[9] != 17 {
        return None;
    }
    let udp = &ip[ihl..];
    let length = usize::from(u16::from_be_bytes([udp[4], udp[5]]));
    if length < 8 || length > udp.len() || u16::from_be_bytes([udp[0], udp[1]]) != 68 {
        return None;
    }
    option(&udp[8..length], 53).and_then(|value| value.first().copied())
}

fn dhcp_reply(frame: &[u8]) -> Option<Vec<u8>> {
    if frame.len() < 14 + 20 + 8 + 240 || u16::from_be_bytes([frame[12], frame[13]]) != 0x0800 {
        return None;
    }
    let ip = &frame[14..];
    let ihl = usize::from(ip[0] & 0x0f) * 4;
    if ip[0] >> 4 != 4 || ihl < 20 || ip.len() < ihl + 8 || ip[9] != 17 {
        return None;
    }
    let udp = &ip[ihl..];
    let udp_length = usize::from(u16::from_be_bytes([udp[4], udp[5]]));
    if udp_length < 8 || udp_length > udp.len() || u16::from_be_bytes([udp[0], udp[1]]) != 68 {
        return None;
    }
    let dhcp = &udp[8..udp_length];
    if dhcp.len() < 240 || dhcp[0] != 1 || dhcp[28..34] == [0; 6] {
        return None;
    }
    let message = option(dhcp, 53)?;
    let (kind, address) = match message.first().copied()? {
        1 => (2, GUEST),
        3 => (
            5,
            option(dhcp, 50)
                .and_then(|value| <[u8; 4]>::try_from(value).ok())
                .map(Ipv4Addr::from)
                .unwrap_or(GUEST),
        ),
        _ => return None,
    };
    let mut payload = vec![0; 240 + 64];
    payload[0] = 2;
    payload[1] = 1;
    payload[2] = 6;
    payload[4..8].copy_from_slice(&dhcp[4..8]);
    payload[16..20].copy_from_slice(&address.octets());
    payload[20..24].copy_from_slice(&SERVER.octets());
    payload[28..34].copy_from_slice(&dhcp[28..34]);
    payload[236..240].copy_from_slice(&[99, 130, 83, 99]);
    let mut offset = 240;
    write_option(&mut payload, &mut offset, 53, &[kind]);
    write_option(&mut payload, &mut offset, 54, &SERVER.octets());
    write_option(&mut payload, &mut offset, 1, &[255, 255, 255, 0]);
    write_option(&mut payload, &mut offset, 3, &SERVER.octets());
    write_option(&mut payload, &mut offset, 51, &600u32.to_be_bytes());
    write_option(&mut payload, &mut offset, 58, &300u32.to_be_bytes());
    write_option(&mut payload, &mut offset, 59, &525u32.to_be_bytes());
    payload[offset] = 255;
    offset += 1;
    payload.truncate(offset);

    let mut udp = vec![0; 8 + payload.len()];
    udp[0..2].copy_from_slice(&67u16.to_be_bytes());
    udp[2..4].copy_from_slice(&68u16.to_be_bytes());
    let udp_length = udp.len() as u16;
    udp[4..6].copy_from_slice(&udp_length.to_be_bytes());
    udp[8..].copy_from_slice(&payload);
    let checksum = pseudo_checksum(SERVER, Ipv4Addr::BROADCAST, &udp);
    udp[6..8].copy_from_slice(&checksum.to_be_bytes());

    let mut ip_packet = vec![0; 20 + udp.len()];
    ip_packet[0] = 0x45;
    let ip_length = ip_packet.len() as u16;
    ip_packet[2..4].copy_from_slice(&ip_length.to_be_bytes());
    ip_packet[8] = 64;
    ip_packet[9] = 17;
    ip_packet[12..16].copy_from_slice(&SERVER.octets());
    ip_packet[16..20].copy_from_slice(&Ipv4Addr::BROADCAST.octets());
    let ip_checksum = checksum_bytes(&ip_packet[..20]);
    ip_packet[10..12].copy_from_slice(&ip_checksum.to_be_bytes());
    ip_packet[20..].copy_from_slice(&udp);

    let mut output = vec![0; 14 + ip_packet.len().max(46)];
    output[0..6].fill(0xff);
    output[6..12].copy_from_slice(&frame[6..12]);
    output[12..14].copy_from_slice(&0x0800u16.to_be_bytes());
    output[14..14 + ip_packet.len()].copy_from_slice(&ip_packet);
    Some(output)
}

fn option(packet: &[u8], wanted: u8) -> Option<&[u8]> {
    let mut index = 240;
    while index < packet.len() {
        let code = packet[index];
        index += 1;
        if code == 255 {
            break;
        }
        if code == 0 {
            continue;
        }
        let length = usize::from(*packet.get(index)?);
        index += 1;
        let end = index.checked_add(length)?;
        let value = packet.get(index..end)?;
        if code == wanted {
            return Some(value);
        }
        index = end;
    }
    None
}

fn write_option(output: &mut [u8], offset: &mut usize, code: u8, value: &[u8]) {
    output[*offset] = code;
    output[*offset + 1] = value.len() as u8;
    output[*offset + 2..*offset + 2 + value.len()].copy_from_slice(value);
    *offset += 2 + value.len();
}

fn pseudo_checksum(source: Ipv4Addr, destination: Ipv4Addr, payload: &[u8]) -> u16 {
    let mut bytes = Vec::with_capacity(12 + payload.len());
    bytes.extend_from_slice(&source.octets());
    bytes.extend_from_slice(&destination.octets());
    bytes.extend_from_slice(&[0, 17]);
    bytes.extend_from_slice(&(payload.len() as u16).to_be_bytes());
    bytes.extend_from_slice(payload);
    checksum_bytes(&bytes)
}

fn checksum_bytes(bytes: &[u8]) -> u16 {
    let mut sum = 0u32;
    for chunk in bytes.chunks(2) {
        sum += u32::from(u16::from_be_bytes([chunk[0], *chunk.get(1).unwrap_or(&0)]));
    }
    while sum >> 16 != 0 {
        sum = (sum & 0xffff) + (sum >> 16);
    }
    !(sum as u16)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn checksum_is_stable() {
        assert_eq!(checksum_bytes(&[0, 1, 0, 2]), 0xfffc);
    }
}
