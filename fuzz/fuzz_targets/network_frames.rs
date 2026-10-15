#![no_main]

//! Feed the input as a sequence of Ethernet frames (u16 length prefix) to a
//! configured interface with ICMP, UDP and TCP sockets, and as NetworkRequests
//! to the service.

use libfuzzer_sys::fuzz_target;
use logos_abi::*;
use logos_network::smoltcp::{
    iface::{PollIngressSingleResult, SocketSet},
    socket::{icmp, tcp, udp},
    time::Instant,
};
use logos_network::*;

fuzz_target!(|data: &[u8]| {
    let mut device = PacketDevice::new();
    let mut iface = interface(&mut device, [2, 0, 0, 0, 0, 1], 0);
    let config = NetworkConfig {
        profile: NetworkProfile::StaticThenDhcp,
        address: [10, 0, 2, 15],
        netmask: [255, 255, 255, 0],
        gateway: [10, 0, 2, 2],
        ..NetworkConfig::disabled()
    };
    assert!(configure_static_ipv4(&mut iface, config));

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

    let mut rest = data;
    let mut out = [0u8; NETWORK_MAX_FRAME_BYTES];
    let mut tick = 0i64;
    while rest.len() >= 2 {
        let length =
            u16::from_le_bytes([rest[0], rest[1]]) as usize % (NETWORK_MAX_FRAME_BYTES + 32);
        let length = length.min(rest.len() - 2);
        let frame = &rest[2..2 + length];
        rest = &rest[2 + length..];
        tick += 10;
        let now = Instant::from_millis(tick);
        if device.enqueue_rx(frame) {
            while iface.poll_ingress_single(now, &mut device, &mut sockets)
                != PollIngressSingleResult::None
            {}
        }
        iface.poll(now, &mut device, &mut sockets);
        let socket = sockets.get_mut::<udp::Socket>(handles[1]);
        while socket.recv().is_ok() {}
        while device.take_tx(&mut out).is_some() {}
    }

    let mut service = NetworkService::new(config);
    service.set_ready();
    for chunk in data.chunks_exact(24) {
        let Some(operation) = NetworkOperation::from_raw(chunk[0] % 13) else { continue };
        let mut request =
            NetworkRequest::new(operation, u32::from_le_bytes(chunk[1..5].try_into().unwrap()));
        request.flags = chunk[5];
        request.handle = u32::from(chunk[6]);
        request.generation = u16::from_le_bytes([chunk[7], chunk[8]]);
        request.service_epoch = u64::from_le_bytes(chunk[9..17].try_into().unwrap());
        request.payload_len = u16::from_le_bytes([chunk[17], chunk[18]]);
        let _ = service.handle(request);
    }
});
