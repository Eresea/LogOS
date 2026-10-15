//! Property tests for the Network boundary: untrusted frames through the
//! packet device and protocol stack, untrusted requests through the service,
//! and the checksum helpers. Skipped under Miri (smoltcp + proptest is slow).
#![cfg(not(miri))]

use logos_abi::*;
use logos_network::smoltcp::{
    iface::{PollIngressSingleResult, SocketSet},
    socket::{icmp, tcp, udp},
    time::Instant,
};
use logos_network::*;
use proptest::prelude::*;

const MAC: [u8; 6] = [2, 0, 0, 0, 0, 1];
const PEER_MAC: [u8; 6] = [2, 0, 0, 0, 0, 2];
const ADDRESS: [u8; 4] = [10, 0, 2, 15];
const PEER: [u8; 4] = [10, 0, 2, 2];

fn config() -> NetworkConfig {
    NetworkConfig {
        profile: NetworkProfile::StaticThenDhcp,
        address: ADDRESS,
        netmask: [255, 255, 255, 0],
        gateway: PEER,
        ..NetworkConfig::disabled()
    }
}

fn checksum_with_pseudo(src: [u8; 4], dst: [u8; 4], protocol: u8, segment: &[u8]) -> u16 {
    let mut data = Vec::new();
    data.extend_from_slice(&src);
    data.extend_from_slice(&dst);
    data.push(0);
    data.push(protocol);
    data.extend_from_slice(&(segment.len() as u16).to_be_bytes());
    data.extend_from_slice(segment);
    internet_checksum(&data)
}

fn ethernet(ethertype: u16, payload: &[u8]) -> Vec<u8> {
    let mut frame = Vec::new();
    frame.extend_from_slice(&MAC);
    frame.extend_from_slice(&PEER_MAC);
    frame.extend_from_slice(&ethertype.to_be_bytes());
    frame.extend_from_slice(payload);
    frame
}

fn ipv4(protocol: u8, payload: &[u8]) -> Vec<u8> {
    let mut packet = vec![0x45, 0];
    packet.extend_from_slice(&((20 + payload.len()) as u16).to_be_bytes());
    packet.extend_from_slice(&[0, 1, 0, 0, 64, protocol, 0, 0]);
    packet.extend_from_slice(&PEER);
    packet.extend_from_slice(&ADDRESS);
    let checksum = internet_checksum(&packet);
    packet[10..12].copy_from_slice(&checksum.to_be_bytes());
    packet.extend_from_slice(payload);
    packet
}

/// Well-formed frames (valid checksums) that reach the ARP, ICMP, UDP and TCP paths.
fn valid_frames(payload: &[u8]) -> Vec<Vec<u8>> {
    let mut arp = vec![0, 1, 8, 0, 6, 4, 0, 1];
    arp.extend_from_slice(&PEER_MAC);
    arp.extend_from_slice(&PEER);
    arp.extend_from_slice(&[0; 6]);
    arp.extend_from_slice(&ADDRESS);

    let mut icmp = vec![8, 0, 0, 0, 0, 1, 0, 1];
    icmp.extend_from_slice(payload);
    let checksum = internet_checksum(&icmp);
    icmp[2..4].copy_from_slice(&checksum.to_be_bytes());

    let mut udp = Vec::new();
    udp.extend_from_slice(&1234u16.to_be_bytes());
    udp.extend_from_slice(&5000u16.to_be_bytes());
    udp.extend_from_slice(&((8 + payload.len()) as u16).to_be_bytes());
    udp.extend_from_slice(&[0, 0]);
    udp.extend_from_slice(payload);
    let checksum = checksum_with_pseudo(PEER, ADDRESS, 17, &udp);
    udp[6..8].copy_from_slice(&checksum.to_be_bytes());

    let mut tcp = Vec::new();
    tcp.extend_from_slice(&1234u16.to_be_bytes());
    tcp.extend_from_slice(&80u16.to_be_bytes());
    tcp.extend_from_slice(&1u32.to_be_bytes());
    tcp.extend_from_slice(&0u32.to_be_bytes());
    tcp.extend_from_slice(&[0x50, 0x02, 0xff, 0xff, 0, 0, 0, 0]);
    tcp.extend_from_slice(payload);
    let checksum = checksum_with_pseudo(PEER, ADDRESS, 6, &tcp);
    tcp[16..18].copy_from_slice(&checksum.to_be_bytes());

    vec![
        ethernet(0x0806, &arp),
        ethernet(0x0800, &ipv4(1, &icmp)),
        ethernet(0x0800, &ipv4(17, &udp)),
        ethernet(0x0800, &ipv4(6, &tcp)),
    ]
}

/// Push frames through a fully configured interface with one socket of each
/// kind and drain whatever it transmits.
fn run_stack(frames: &[Vec<u8>]) {
    let mut device = PacketDevice::new();
    let mut iface = interface(&mut device, MAC, 0);
    assert!(configure_static_ipv4(&mut iface, config()));

    let mut udp_rx_meta = [udp::PacketMetadata::EMPTY; 4];
    let mut udp_rx = [0u8; 256];
    let mut udp_tx_meta = [udp::PacketMetadata::EMPTY; 4];
    let mut udp_tx = [0u8; 256];
    let mut icmp_rx_meta = [icmp::PacketMetadata::EMPTY; 4];
    let mut icmp_rx = [0u8; 256];
    let mut icmp_tx_meta = [icmp::PacketMetadata::EMPTY; 4];
    let mut icmp_tx = [0u8; 256];
    let mut tcp_rx = [0u8; 512];
    let mut tcp_tx = [0u8; 512];
    let mut storage: [_; 4] = Default::default();
    let mut sockets = SocketSet::new(&mut storage[..]);
    let handles = add_protocol_sockets(
        &mut sockets,
        ProtocolSocketBuffers {
            udp_rx_metadata: &mut udp_rx_meta,
            udp_rx_bytes: &mut udp_rx,
            udp_tx_metadata: &mut udp_tx_meta,
            udp_tx_bytes: &mut udp_tx,
            icmp_rx_metadata: &mut icmp_rx_meta,
            icmp_rx_bytes: &mut icmp_rx,
            icmp_tx_metadata: &mut icmp_tx_meta,
            icmp_tx_bytes: &mut icmp_tx,
            tcp_rx_bytes: &mut tcp_rx,
            tcp_tx_bytes: &mut tcp_tx,
        },
    );
    sockets.get_mut::<udp::Socket>(handles[1]).bind(5000).unwrap();
    sockets.get_mut::<tcp::Socket>(handles[2]).listen(80).unwrap();
    sockets.get_mut::<icmp::Socket>(handles[0]).bind(icmp::Endpoint::Ident(1)).unwrap();

    let mut out = [0u8; NETWORK_MAX_FRAME_BYTES];
    for (tick, frame) in frames.iter().enumerate() {
        let now = Instant::from_millis(tick as i64 * 10);
        if device.enqueue_rx(frame) {
            while iface.poll_ingress_single(now, &mut device, &mut sockets)
                != PollIngressSingleResult::None
            {}
        }
        iface.poll(now, &mut device, &mut sockets);
        let udp_socket = sockets.get_mut::<udp::Socket>(handles[1]);
        while udp_socket.recv().is_ok() {}
        let tcp_socket = sockets.get_mut::<tcp::Socket>(handles[2]);
        if tcp_socket.can_recv() {
            let _ = tcp_socket.recv(|data| (data.len(), ()));
        }
        while let Some(length) = device.take_tx(&mut out) {
            assert!(length <= NETWORK_MAX_FRAME_BYTES);
        }
    }
}

fn mutate(frame: &mut Vec<u8>, flips: &[(usize, u8)], truncate: Option<usize>) {
    for (index, value) in flips {
        if !frame.is_empty() {
            let at = index % frame.len();
            frame[at] = *value;
        }
    }
    if let Some(len) = truncate {
        frame.truncate(len % (frame.len() + 1));
    }
}

fn request_strategy() -> impl Strategy<Value = NetworkRequest> {
    (
        1u8..=12,
        any::<u8>(),
        any::<u32>(),
        any::<u32>(),
        any::<u16>(),
        any::<u64>(),
        any::<u16>(),
        (any::<[u8; 4]>(), any::<u16>(), any::<u32>()),
    )
        .prop_map(
            |(operation, flags, request_id, handle, generation, epoch, payload_len, tail)| {
                let mut request =
                    NetworkRequest::new(NetworkOperation::from_raw(operation).unwrap(), request_id);
                request.flags = flags;
                request.handle = handle;
                request.generation = generation;
                request.service_epoch = epoch;
                request.payload_len = payload_len;
                request.address = tail.0;
                request.port = tail.1;
                request.timeout_ticks = tail.2;
                request
            },
        )
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    #[test]
    fn arbitrary_frames_never_panic(
        frames in proptest::collection::vec(
            proptest::collection::vec(any::<u8>(), 0..NETWORK_MAX_FRAME_BYTES + 64), 1..8),
    ) {
        run_stack(&frames);
    }

    #[test]
    fn mutated_valid_frames_never_panic(
        payload in proptest::collection::vec(any::<u8>(), 0..64),
        which in 0usize..4,
        flips in proptest::collection::vec((any::<usize>(), any::<u8>()), 0..6),
        truncate in proptest::option::of(any::<usize>()),
        repeat in 1usize..4,
    ) {
        let mut frame = valid_frames(&payload).swap_remove(which);
        let pristine = frame.clone();
        mutate(&mut frame, &flips, truncate);
        let mut frames = vec![pristine];
        for _ in 0..repeat {
            frames.push(frame.clone());
        }
        run_stack(&frames);
    }

    #[test]
    fn unmutated_valid_frames_get_answers(payload in proptest::collection::vec(any::<u8>(), 0..64)) {
        // ARP request for our address and an ICMP echo must each produce a reply.
        let frames = valid_frames(&payload);
        for frame in [&frames[0], &frames[1]] {
            let mut device = PacketDevice::new();
            let mut iface = interface(&mut device, MAC, 0);
            prop_assert!(configure_static_ipv4(&mut iface, config()));
            let mut storage: [_; 1] = Default::default();
            let mut sockets = SocketSet::new(&mut storage[..]);
            prop_assert!(device.enqueue_rx(frame));
            let now = Instant::from_millis(1);
            iface.poll(now, &mut device, &mut sockets);
            let mut out = [0u8; NETWORK_MAX_FRAME_BYTES];
            prop_assert!(device.take_tx(&mut out).is_some());
        }
    }

    #[test]
    fn service_handles_arbitrary_requests(
        requests in proptest::collection::vec(request_strategy(), 1..40),
        ready in any::<bool>(),
        reset_at in proptest::option::of(0usize..40),
    ) {
        let mut service = NetworkService::new(config());
        if ready {
            service.set_ready();
        }
        for (index, request) in requests.iter().enumerate() {
            if reset_at == Some(index) {
                service.reset();
            }
            let response = service.handle(*request);
            prop_assert_eq!(response.request_id, request.request_id);
            prop_assert_eq!(response.operation, request.operation);
        }
    }

    #[test]
    fn service_handles_arbitrary_socket_lifecycle(
        ops in proptest::collection::vec((any::<u8>(), any::<bool>(), 0u8..8, any::<bool>()), 1..80),
    ) {
        let mut service = NetworkService::new(config());
        service.set_ready();
        let mut live = Vec::new();
        for (op, listener, slot, stale) in ops {
            match op % 4 {
                0 => if let Ok(handle) = service.allocate_socket(listener) { live.push((handle, listener)); },
                1 => if let Ok(handle) = service.allocate_udp() { live.push((handle, false)); },
                2 => if let Some((handle, listener)) = live.get(slot as usize % live.len().max(1)).copied() {
                    let mut handle = handle;
                    if stale { handle.generation = handle.generation.wrapping_add(1); }
                    let _ = service.close(handle, listener);
                },
                _ => if let Some((handle, true)) = live.get(slot as usize % live.len().max(1)).copied() {
                    let _ = service.accept(handle);
                },
            }
        }
    }

    #[test]
    fn checksum_never_panics_and_validates_its_own_output(
        data in proptest::collection::vec(any::<u8>(), 0..2048),
    ) {
        let _ = checksum_valid(&data);
        let mut even = data.clone();
        if even.len() % 2 == 1 {
            even.push(0);
        }
        let checksum = internet_checksum(&even);
        even.extend_from_slice(&checksum.to_be_bytes());
        prop_assert!(checksum_valid(&even));
    }

    #[test]
    fn packet_device_is_a_bounded_fifo(
        script in proptest::collection::vec((any::<bool>(), 0usize..NETWORK_MAX_FRAME_BYTES + 8), 0..64),
    ) {
        let mut device = PacketDevice::new();
        let mut model = std::collections::VecDeque::new();
        for (push, length) in script {
            if push {
                let frame = vec![length as u8; length];
                let accepted = device.enqueue_rx(&frame);
                prop_assert_eq!(
                    accepted,
                    length <= NETWORK_MAX_FRAME_BYTES && model.len() < NETWORK_RX_PACKET_PAGES
                );
                if accepted {
                    model.push_back(frame);
                }
            }
            prop_assert_eq!(device.pending_rx(), model.len());
        }
    }
}
