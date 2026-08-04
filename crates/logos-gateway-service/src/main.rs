#![no_main]
#![no_std]

use logos_abi::{
    MAX_TCP_PAYLOAD, NetworkEndpoint, NetworkOperation, NetworkProtocol, NetworkRequest,
    NetworkScope, NetworkStatus, REMOTE_TCP_PORT,
    service::{RemoteGateOperation, RemoteGateRequest, RemoteGateStatus},
};
use logos_remote::{
    FrameDecoder, MAX_FRAME, MAX_FRAME_BUFFER, RemoteMessage, RemoteMessageKind, frame_encode,
};
use logos_service_rt::{Header, ProtocolVersion, ServiceContext};

const DEADLINE: u64 = u64::MAX / 2;

#[used]
#[unsafe(link_section = ".logos")]
static HEADER: Header =
    Header::new(*b"gateway\0\0\0\0\0\0\0\0\0", ProtocolVersion::V1, logos_service_entry);

#[unsafe(no_mangle)]
extern "C" fn logos_service_entry(context: logos_service_rt::EntryControlPage) -> ! {
    logos_service_rt::entry(context, run)
}

fn run(context: &mut ServiceContext) -> ! {
    if !context.ready() {
        spin();
    }
    let Some(page) = context.shared_page() else { spin() };
    let mut id = 1;
    while context.acknowledged() {
        let Some(listener) = network(
            context,
            &mut id,
            NetworkOperation::Listen,
            NetworkEndpoint(0),
            0,
            logos_abi::PageHandle(0),
            0,
        ) else {
            continue;
        };
        loop {
            let Some(stream) = network(
                context,
                &mut id,
                NetworkOperation::Accept,
                listener.endpoint,
                0,
                logos_abi::PageHandle(0),
                0,
            ) else {
                break;
            };
            serve(context, page.handle, page.address, listener.endpoint, stream, &mut id);
        }
    }
    spin()
}

fn serve(
    context: &mut ServiceContext,
    page: logos_abi::PageHandle,
    address: u64,
    _listener: NetworkEndpoint,
    stream: logos_abi::NetworkReply,
    id: &mut u32,
) {
    let mut decoder = FrameDecoder::new();
    let mut authenticated = false;
    'connection: loop {
        let Some(reply) = network(
            context,
            id,
            NetworkOperation::Read,
            stream.endpoint,
            stream.generation,
            page,
            MAX_TCP_PAYLOAD as u16,
        ) else {
            break;
        };
        if reply.length == 0 {
            break;
        }
        let input =
            unsafe { core::slice::from_raw_parts(address as *const u8, reply.length as usize) };
        for &byte in input {
            if decoder.push(core::slice::from_ref(&byte)).is_err() {
                break 'connection;
            }
            let Some(frame) = decoder.ready().ok().flatten() else { continue };
            let mut request = [0; MAX_FRAME];
            request[..frame.len()].copy_from_slice(frame);
            let operation = if authenticated {
                RemoteGateOperation::Open
            } else {
                RemoteGateOperation::Handshake
            };
            let Some(opened) = gate(context, id, page, address, operation, &request[..frame.len()])
            else {
                break 'connection;
            };
            let mut stream_events = false;
            let response_length = if authenticated {
                let plaintext =
                    unsafe { core::slice::from_raw_parts(address as *const u8, opened) };
                let Ok(message) = RemoteMessage::decode(plaintext) else {
                    break 'connection;
                };
                let operation = match message.kind {
                    RemoteMessageKind::Invoke => RemoteGateOperation::Invoke,
                    RemoteMessageKind::Subscribe => RemoteGateOperation::Subscribe,
                    RemoteMessageKind::Credit => RemoteGateOperation::Credit,
                    RemoteMessageKind::Cancel => RemoteGateOperation::Acknowledge,
                    _ => break 'connection,
                };
                stream_events = matches!(
                    message.kind,
                    RemoteMessageKind::Subscribe | RemoteMessageKind::Credit
                );
                let Some(length) = gate(context, id, page, address, operation, plaintext) else {
                    break 'connection;
                };
                if length == 0 {
                    decoder.consume().ok();
                    continue;
                }
                let plaintext =
                    unsafe { core::slice::from_raw_parts(address as *const u8, length) };
                let mut plain = [0; MAX_FRAME];
                plain[..length].copy_from_slice(plaintext);
                let Some(sealed) =
                    gate(context, id, page, address, RemoteGateOperation::Seal, &plain[..length])
                else {
                    break 'connection;
                };
                sealed
            } else {
                authenticated = true;
                opened
            };
            let response =
                unsafe { core::slice::from_raw_parts(address as *const u8, response_length) };
            let mut framed = [0; MAX_FRAME_BUFFER];
            let Ok(framed_length) = frame_encode(&mut framed, response) else {
                break 'connection;
            };
            if !write_all(context, id, page, address, stream, &framed[..framed_length]) {
                break 'connection;
            }
            if decoder.consume().is_err() {
                break 'connection;
            }
            if authenticated && stream_events {
                loop {
                    let Some(length) =
                        gate(context, id, page, address, RemoteGateOperation::Acknowledge, &[])
                    else {
                        break 'connection;
                    };
                    if length == 0 {
                        break;
                    }
                    let plaintext =
                        unsafe { core::slice::from_raw_parts(address as *const u8, length) };
                    let mut plain = [0; MAX_FRAME];
                    plain[..length].copy_from_slice(plaintext);
                    let Some(sealed) = gate(
                        context,
                        id,
                        page,
                        address,
                        RemoteGateOperation::Seal,
                        &plain[..length],
                    ) else {
                        break 'connection;
                    };
                    let response =
                        unsafe { core::slice::from_raw_parts(address as *const u8, sealed) };
                    let mut event_frame = [0; MAX_FRAME_BUFFER];
                    let Ok(event_length) = frame_encode(&mut event_frame, response) else {
                        break 'connection;
                    };
                    if !write_all(context, id, page, address, stream, &event_frame[..event_length])
                    {
                        break 'connection;
                    }
                }
            }
        }
    }
    let _ = gate(context, id, page, address, RemoteGateOperation::Reset, &[]);
    let _ = network(
        context,
        id,
        NetworkOperation::Close,
        stream.endpoint,
        0,
        logos_abi::PageHandle(0),
        0,
    );
}

fn gate(
    context: &mut ServiceContext,
    id: &mut u32,
    page: logos_abi::PageHandle,
    address: u64,
    operation: RemoteGateOperation,
    input: &[u8],
) -> Option<usize> {
    if input.len() > logos_abi::PAGE_SIZE {
        return None;
    }
    unsafe { core::ptr::copy_nonoverlapping(input.as_ptr(), address as *mut u8, input.len()) };
    let request_id = *id;
    *id = id.wrapping_add(1).max(1);
    if !context.request_remote_gate(RemoteGateRequest {
        id: request_id,
        operation,
        page,
        length: input.len() as u16,
        deadline: DEADLINE,
    }) {
        return None;
    }
    let reply = context.remote_gate_reply(request_id)?;
    (reply.status == RemoteGateStatus::Complete).then_some(reply.length as usize)
}

fn write_all(
    context: &mut ServiceContext,
    id: &mut u32,
    page: logos_abi::PageHandle,
    address: u64,
    stream: logos_abi::NetworkReply,
    bytes: &[u8],
) -> bool {
    for chunk in bytes.chunks(MAX_TCP_PAYLOAD) {
        unsafe { core::ptr::copy_nonoverlapping(chunk.as_ptr(), address as *mut u8, chunk.len()) };
        if network(
            context,
            id,
            NetworkOperation::Write,
            stream.endpoint,
            stream.generation,
            page,
            chunk.len() as u16,
        )
        .is_none()
        {
            return false;
        }
    }
    true
}

fn network(
    context: &mut ServiceContext,
    id: &mut u32,
    operation: NetworkOperation,
    endpoint: NetworkEndpoint,
    generation: u16,
    page: logos_abi::PageHandle,
    length: u16,
) -> Option<logos_abi::NetworkReply> {
    let request_id = *id;
    *id = id.wrapping_add(1).max(1);
    let request = NetworkRequest {
        id: request_id,
        operation,
        endpoint,
        peer: if matches!(operation, NetworkOperation::Close | NetworkOperation::Cancel) {
            NetworkScope(0)
        } else {
            NetworkScope::new(NetworkProtocol::Tcp, 0, REMOTE_TCP_PORT)
        },
        page,
        length,
        generation,
        deadline: DEADLINE,
    };
    if !context.request_network(request) {
        return None;
    }
    context.network_response(request_id).filter(|reply| reply.status == NetworkStatus::Complete)
}

fn spin() -> ! {
    loop {
        core::hint::spin_loop();
    }
}
