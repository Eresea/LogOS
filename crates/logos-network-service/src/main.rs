#![cfg_attr(not(test), no_main)]
#![cfg_attr(not(test), no_std)]

use logos_abi::{
    NetworkEndpoint, NetworkInfo, NetworkOperation, NetworkReply, NetworkRequest, NetworkStatus,
};
use logos_net::{Mac, NetworkState, StateError, parse_arp, parse_ethernet};
use logos_service_rt::{Context, Header};

#[used]
#[unsafe(link_section = ".logos")]
static HEADER: Header = Header::new(*b"network\0\0\0\0\0\0\0\0\0", logos_service_entry);

#[unsafe(no_mangle)]
extern "C" fn logos_service_entry(context: logos_service_rt::EntryContext) -> ! {
    logos_service_rt::entry(context, run)
}

fn run(context: &mut Context) -> ! {
    if !context.ready() {
        spin();
    }
    let mut state = NetworkState::new();
    state.dhcp_start(1, 1);
    let mut deadline: u64 = 1_000_000;
    while context.acknowledged() {
        if let Some(request) = context.network_request() {
            let reply = handle_request(&mut state, request);
            if !context.network_reply(reply) {
                spin();
            }
            continue;
        }
        if let Some(event) = context.network_event() {
            if event.kind == logos_abi::NetworkEventKind::Frame {
                let _ = validate_frame(context, event.length);
            } else if event.kind == logos_abi::NetworkEventKind::Timer {
                let _ = state.dhcp_tick(deadline);
            }
            deadline = deadline.saturating_add(1_000_000).max(1);
        }
        if !context.network_wait(deadline) {
            spin();
        }
    }
    spin()
}

fn handle_request(state: &mut NetworkState, request: NetworkRequest) -> NetworkReply {
    let (status, endpoint) = match request.operation {
        NetworkOperation::Status => (NetworkStatus::Offline, NetworkEndpoint(0)),
        NetworkOperation::Bind => match state.bind(1, request.peer.port()) {
            Ok(endpoint) => (NetworkStatus::Complete, NetworkEndpoint(endpoint.wire())),
            Err(StateError::AddressInUse) => (NetworkStatus::AddressInUse, NetworkEndpoint(0)),
            Err(StateError::Full) => (NetworkStatus::Full, NetworkEndpoint(0)),
            Err(_) => (NetworkStatus::Invalid, NetworkEndpoint(0)),
        },
        NetworkOperation::Close => match logos_net::EndpointId::from_wire(request.endpoint.0)
            .and_then(|endpoint| state.close(1, endpoint).ok())
        {
            Some(()) => (NetworkStatus::Complete, NetworkEndpoint(0)),
            None => (NetworkStatus::Invalid, NetworkEndpoint(0)),
        },
        NetworkOperation::Cancel => {
            let status = if state.cancel_pending(request.id).is_ok() {
                NetworkStatus::Cancelled
            } else {
                NetworkStatus::Invalid
            };
            (status, NetworkEndpoint(0))
        }
        NetworkOperation::SendTo | NetworkOperation::ReceiveFrom | NetworkOperation::Echo => {
            (NetworkStatus::Offline, NetworkEndpoint(0))
        }
    };
    NetworkReply {
        id: request.id,
        status,
        endpoint,
        generation: state.generation(),
        source_address: 0,
        source_port: 0,
        length: 0,
        info: NetworkInfo::default(),
        counters: logos_abi::NetworkCounters::default(),
    }
}

fn validate_frame(context: &Context, length: u16) -> bool {
    let Some(pages) = context.network_pages() else {
        return false;
    };
    let length = usize::from(length);
    if length > logos_net::ETHERNET_MAX_FRAME {
        return false;
    }
    let frame = unsafe { core::slice::from_raw_parts(pages.rx_address as *const u8, length) };
    let Ok(ethernet) = parse_ethernet(frame, Mac::BROADCAST) else {
        return false;
    };
    ethernet.ether_type != 0x0806 || parse_arp(ethernet.payload).is_ok()
}

fn spin() -> ! {
    loop {
        core::hint::spin_loop();
    }
}
