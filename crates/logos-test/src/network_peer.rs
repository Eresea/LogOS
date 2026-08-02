use std::fs::{File, OpenOptions};
use std::io::Write;
use std::net::{IpAddr, Ipv4Addr, SocketAddr, UdpSocket};
use std::path::Path;
use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicU8, Ordering},
};
use std::thread::{self, JoinHandle};
use std::time::Duration;

const GUEST: Ipv4Addr = Ipv4Addr::new(10, 0, 2, 15);
const SERVER: Ipv4Addr = Ipv4Addr::new(10, 0, 2, 2);
const SERVER_MAC: [u8; 6] = [0x52, 0x54, 0x00, 0xaa, 0xbb, 0xcc];

pub struct NetworkPeer {
    stop: Arc<AtomicBool>,
    mode: Arc<AtomicU8>,
    worker: Option<JoinHandle<()>>,
    pub qemu_port: u16,
    pub peer_port: u16,
}

impl NetworkPeer {
    pub fn start(capture_path: &Path) -> Result<Self, String> {
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
        let mode = Arc::new(AtomicU8::new(0));
        let worker_mode = Arc::clone(&mode);
        let capture = OpenOptions::new()
            .create(true)
            .append(true)
            .open(capture_path)
            .map_err(|error| error.to_string())?;
        let worker = thread::spawn(move || {
            let mut input = [0; 2048];
            let mut drop_first_discover = true;
            let mut drop_first_loss_echo = true;
            let mut capture = Some(capture);
            while !worker_stop.load(Ordering::Relaxed) {
                let Ok((length, _)) = socket.recv_from(&mut input) else { continue };
                log_frame(&mut capture, "rx", &input[..length]);
                let mut send = |frame: &[u8]| {
                    log_frame(&mut capture, "tx", frame);
                    let _ = socket.send_to(frame, qemu);
                };
                if drop_first_discover && dhcp_message_type(&input[..length]) == Some(1) {
                    drop_first_discover = false;
                    continue;
                }
                if let Some(frame) = dhcp_reply(&input[..length]) {
                    send(&frame);
                } else if let Some(frame) = arp_reply(&input[..length]) {
                    send(&frame);
                } else if let Some(frame) = icmp_reply(&input[..length]) {
                    let current_mode = worker_mode.load(Ordering::Relaxed);
                    if current_mode == 2 {
                        continue;
                    }
                    if current_mode == 1
                        && drop_first_loss_echo
                        && icmp_identifier(&input[..length]).is_some_and(|id| id >= 2)
                    {
                        drop_first_loss_echo = false;
                        send(&[0; 60]);
                    }
                    send(&frame);
                    if current_mode == 1 {
                        send(&frame);
                    }
                } else if let Some(frame) = udp_reply(&input[..length]) {
                    if worker_mode.load(Ordering::Relaxed) == 2 {
                        continue;
                    }
                    send(&frame);
                }
            }
        });
        Ok(Self { stop, mode, worker: Some(worker), qemu_port, peer_port })
    }

    pub fn set_scenario(&self, scenario: &str) {
        self.mode.store(
            match scenario {
                "network/packet-loss" => 1,
                "network/timeout" => 2,
                _ => 0,
            },
            Ordering::Relaxed,
        );
    }
}

fn log_frame(file: &mut Option<File>, direction: &str, frame: &[u8]) {
    let ether_type = frame.get(12..14).map_or(0, |bytes| u16::from_be_bytes([bytes[0], bytes[1]]));
    if let Some(file) = file {
        let _ = writeln!(file, "{direction} len={} ether={ether_type:04x}", frame.len());
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

fn ipv4_payload(frame: &[u8], protocol: u8) -> Option<(&[u8], [u8; 6], [u8; 6])> {
    if frame.len() < 14 + 20 || u16::from_be_bytes([frame[12], frame[13]]) != 0x0800 {
        return None;
    }
    let ip = &frame[14..];
    let ihl = usize::from(ip[0] & 0x0f) * 4;
    if ip[0] >> 4 != 4 || ihl < 20 || ip.len() < ihl || ip[9] != protocol {
        return None;
    }
    let total = usize::from(u16::from_be_bytes([ip[2], ip[3]]));
    if total < ihl || total > ip.len() {
        return None;
    }
    let mut destination = [0; 6];
    destination.copy_from_slice(&frame[..6]);
    let mut source = [0; 6];
    source.copy_from_slice(&frame[6..12]);
    Some((&ip[ihl..total], destination, source))
}

fn arp_reply(frame: &[u8]) -> Option<Vec<u8>> {
    if frame.len() < 14 + 28 || u16::from_be_bytes([frame[12], frame[13]]) != 0x0806 {
        return None;
    }
    let arp = &frame[14..42];
    if u16::from_be_bytes([arp[0], arp[1]]) != 1
        || u16::from_be_bytes([arp[2], arp[3]]) != 0x0800
        || arp[4] != 6
        || arp[5] != 4
        || u16::from_be_bytes([arp[6], arp[7]]) != 1
        || arp[24..28] != SERVER.octets()
    {
        return None;
    }
    let mut output = vec![0; 60];
    output[..6].copy_from_slice(&frame[6..12]);
    output[6..12].copy_from_slice(&SERVER_MAC);
    output[12..14].copy_from_slice(&0x0806u16.to_be_bytes());
    output[14..16].copy_from_slice(&1u16.to_be_bytes());
    output[16..18].copy_from_slice(&0x0800u16.to_be_bytes());
    output[18] = 6;
    output[19] = 4;
    output[20..22].copy_from_slice(&2u16.to_be_bytes());
    output[22..28].copy_from_slice(&SERVER_MAC);
    output[28..32].copy_from_slice(&SERVER.octets());
    output[32..38].copy_from_slice(&arp[8..14]);
    output[38..42].copy_from_slice(&arp[14..18]);
    Some(output)
}

fn icmp_reply(frame: &[u8]) -> Option<Vec<u8>> {
    let (payload, _destination, source) = ipv4_payload(frame, 1)?;
    if payload.len() < 8 || payload[0] != 8 || checksum_bytes(payload) != 0 {
        return None;
    }
    let mut ip = vec![0; 20 + payload.len()];
    ip[0] = 0x45;
    let ip_length = ip.len() as u16;
    ip[2..4].copy_from_slice(&ip_length.to_be_bytes());
    ip[8] = 64;
    ip[9] = 1;
    ip[12..16].copy_from_slice(&SERVER.octets());
    ip[16..20].copy_from_slice(&GUEST.octets());
    let mut icmp = payload.to_vec();
    icmp[0] = 0;
    icmp[2..4].fill(0);
    let icmp_checksum = checksum_bytes(&icmp);
    icmp[2..4].copy_from_slice(&icmp_checksum.to_be_bytes());
    let ip_checksum = checksum_bytes(&ip[..20]);
    ip[10..12].copy_from_slice(&ip_checksum.to_be_bytes());
    ip[20..].copy_from_slice(&icmp);
    ethernet_frame(source, SERVER_MAC, 0x0800, &ip)
}

fn icmp_identifier(frame: &[u8]) -> Option<u16> {
    let (payload, _, _) = ipv4_payload(frame, 1)?;
    (payload.len() >= 8 && payload[0] == 8).then(|| u16::from_be_bytes([payload[4], payload[5]]))
}

fn udp_reply(frame: &[u8]) -> Option<Vec<u8>> {
    let (payload, _destination, source) = ipv4_payload(frame, 17)?;
    if payload.len() < 8
        || u16::from_be_bytes([payload[0], payload[1]]) == 0
        || u16::from_be_bytes([payload[2], payload[3]]) != 4001
    {
        return None;
    }
    let length = usize::from(u16::from_be_bytes([payload[4], payload[5]]));
    if length < 8 || length > payload.len() {
        return None;
    }
    let body = &payload[8..length];
    let mut udp = vec![0; 8 + body.len()];
    udp[0..2].copy_from_slice(&4001u16.to_be_bytes());
    udp[2..4].copy_from_slice(&4000u16.to_be_bytes());
    let udp_length = udp.len() as u16;
    udp[4..6].copy_from_slice(&udp_length.to_be_bytes());
    udp[8..].copy_from_slice(body);
    let udp_checksum = pseudo_checksum(SERVER, GUEST, &udp);
    udp[6..8].copy_from_slice(&udp_checksum.to_be_bytes());
    let mut ip = vec![0; 20 + udp.len()];
    ip[0] = 0x45;
    let ip_length = ip.len() as u16;
    ip[2..4].copy_from_slice(&ip_length.to_be_bytes());
    ip[8] = 64;
    ip[9] = 17;
    ip[12..16].copy_from_slice(&SERVER.octets());
    ip[16..20].copy_from_slice(&GUEST.octets());
    let ip_checksum = checksum_bytes(&ip[..20]);
    ip[10..12].copy_from_slice(&ip_checksum.to_be_bytes());
    ip[20..].copy_from_slice(&udp);
    ethernet_frame(source, SERVER_MAC, 0x0800, &ip)
}

fn ethernet_frame(
    destination: [u8; 6],
    source: [u8; 6],
    ether_type: u16,
    payload: &[u8],
) -> Option<Vec<u8>> {
    let length = 14usize.checked_add(payload.len())?;
    let mut output = vec![0; length.max(60)];
    output[..6].copy_from_slice(&destination);
    output[6..12].copy_from_slice(&source);
    output[12..14].copy_from_slice(&ether_type.to_be_bytes());
    output[14..length].copy_from_slice(payload);
    Some(output)
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
