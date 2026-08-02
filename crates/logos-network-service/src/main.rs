#![cfg_attr(not(test), no_main)]
#![cfg_attr(not(test), no_std)]

use logos_abi::{NetworkEndpoint, NetworkReply, NetworkRequest, NetworkStatus};
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
    let mut deadline: u64 = 1_000_000;
    while context.acknowledged() {
        if let Some(request) = context.network_request() {
            let reply = offline_reply(request);
            if !context.network_reply(reply) {
                spin();
            }
            continue;
        }
        if context.network_event().is_some() {
            deadline = deadline.saturating_add(1_000_000);
        }
        if !context.network_wait(deadline) {
            spin();
        }
    }
    spin()
}

fn offline_reply(request: NetworkRequest) -> NetworkReply {
    NetworkReply {
        id: request.id,
        status: NetworkStatus::Offline,
        endpoint: NetworkEndpoint(0),
        generation: request.generation,
        source_address: 0,
        source_port: 0,
        length: 0,
        info: logos_abi::NetworkInfo::default(),
        counters: logos_abi::NetworkCounters::default(),
    }
}

fn spin() -> ! {
    loop {
        core::hint::spin_loop();
    }
}
