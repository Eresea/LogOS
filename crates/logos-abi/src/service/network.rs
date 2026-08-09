use super::*;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u32)]
pub enum NetworkDevicePageState {
    Ready = 1,
    Request = 2,
    Submitted = 3,
    Reply = 4,
}

impl NetworkDevicePageState {
    pub const fn from_wire(value: u32) -> Option<Self> {
        Some(match value {
            1 => Self::Ready,
            2 => Self::Request,
            3 => Self::Submitted,
            4 => Self::Reply,
            _ => return None,
        })
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u32)]
pub enum NetworkEventPageState {
    Ready = 1,
    Waiting = 2,
    Event = 3,
    Consumed = 4,
}

impl NetworkEventPageState {
    pub const fn from_wire(value: u32) -> Option<Self> {
        Some(match value {
            1 => Self::Ready,
            2 => Self::Waiting,
            3 => Self::Event,
            4 => Self::Consumed,
            _ => return None,
        })
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct NetworkDeviceMessage {
    pub request: logos_abi::NetworkDeviceRequest,
    pub rx_page: logos_abi::PageHandle,
    pub tx_page: logos_abi::PageHandle,
}

/// Core-owned Network device endpoint. Only the Network service maps it.
#[derive(Clone, Copy)]
#[repr(C)]
pub struct NetworkDevicePage {
    pub service_generation: u32,
    pub endpoint_generation: u32,
    pub device_generation: u32,
    pub state: u32,
    pub request_id: u32,
    pub operation: u32,
    pub rx_page: u32,
    pub tx_page: u32,
    pub length: u32,
    pub deadline: u64,
    pub reply_status: u32,
    pub reset_generation: u32,
    pub info: logos_abi::NetworkInfo,
    pub metadata: [u8; 32],
    pub reserved: [u8; logos_abi::PAGE_SIZE - 112],
}

/// Core/Foundation-produced Network event endpoint. It holds one event only.
#[derive(Clone, Copy)]
#[repr(C)]
pub struct NetworkEventPage {
    pub service_generation: u32,
    pub endpoint_generation: u32,
    pub device_generation: u32,
    pub state: u32,
    pub sequence: u32,
    pub kind: u32,
    pub transfer_page: u32,
    pub length: u32,
    pub deadline: u64,
    pub now: u64,
    pub generation: u16,
    pub reserved0: u16,
    pub metadata: [u8; 32],
    pub configured_rx_page: u32,
    pub reserved: [u8; logos_abi::PAGE_SIZE - 88],
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u32)]
pub enum NetworkPageState {
    Ready = 1,
    Request = 2,
    Processing = 3,
    Reply = 4,
    Denied = 5,
    Failed = 6,
    Cancelled = 7,
    TimedOut = 8,
}

impl NetworkPageState {
    pub const fn from_wire(value: u32) -> Option<Self> {
        Some(match value {
            1 => Self::Ready,
            2 => Self::Request,
            3 => Self::Processing,
            4 => Self::Reply,
            5 => Self::Denied,
            6 => Self::Failed,
            7 => Self::Cancelled,
            8 => Self::TimedOut,
            _ => return None,
        })
    }
}

/// Auxiliary, generation-bound stream readiness/completion page.
///
/// The page is owned by Core and shared with the Network service. Records are
/// coalesced per endpoint; the page is not a second request/reply transport.
#[derive(Clone, Copy)]
#[repr(C)]
pub struct StreamPage {
    pub service_generation: u32,
    pub endpoint_generation: u32,
    pub state: u32,
    pub overflow: u32,
    pub sequence: u64,
    pub records: [logos_abi::NetworkStreamRecord; logos_abi::NETWORK_MAX_STREAM_RECORDS],
    pub reserved: [u8; logos_abi::PAGE_SIZE - 408],
}

#[allow(clippy::missing_safety_doc)]
impl StreamPage {
    pub const fn new(service_generation: u32, endpoint_generation: u32) -> Self {
        Self {
            service_generation,
            endpoint_generation,
            state: NetworkPageState::Ready as u32,
            overflow: 0,
            sequence: 0,
            records: [logos_abi::NetworkStreamRecord::EMPTY; logos_abi::NETWORK_MAX_STREAM_RECORDS],
            reserved: [0; logos_abi::PAGE_SIZE - 408],
        }
    }

    pub unsafe fn reset_at(
        address: u64,
        service_generation: u32,
        endpoint_generation: u32,
    ) -> bool {
        if address == 0 || !address.is_multiple_of(core::mem::align_of::<Self>() as u64) {
            return false;
        }
        let current = unsafe { (address as *const Self).read_volatile() };
        if current.service_generation != service_generation
            || current.endpoint_generation != endpoint_generation
            || service_generation == 0
            || endpoint_generation == 0
        {
            return false;
        }
        unsafe {
            (address as *mut Self)
                .write_volatile(Self::new(service_generation, endpoint_generation))
        };
        true
    }

    pub unsafe fn publish_at(
        address: u64,
        service_generation: u32,
        endpoint_generation: u32,
        mut record: logos_abi::NetworkStreamRecord,
    ) -> bool {
        if !record.endpoint.valid() || record.generation == 0 {
            return false;
        }
        let mut page = unsafe { (address as *mut Self).read_volatile() };
        if page.service_generation != service_generation
            || page.endpoint_generation != endpoint_generation
            || service_generation == 0
            || endpoint_generation == 0
        {
            return false;
        }
        page.sequence = page.sequence.wrapping_add(1).max(1);
        record.sequence = page.sequence;
        if let Some(existing) = page.records.iter_mut().find(|item| {
            item.owner == record.owner
                && item.endpoint == record.endpoint
                && item.generation == record.generation
        }) {
            *existing = record;
        } else if let Some(empty) = page.records.iter_mut().find(|item| item.endpoint.0 == 0) {
            *empty = record;
        } else {
            page.overflow = 1;
            unsafe { (address as *mut Self).write_volatile(page) };
            return false;
        }
        unsafe { (address as *mut Self).write_volatile(page) };
        true
    }

    pub unsafe fn take_at(
        address: u64,
        service_generation: u32,
        endpoint_generation: u32,
        endpoint: logos_abi::NetworkEndpoint,
    ) -> Option<logos_abi::NetworkStreamRecord> {
        let mut page = unsafe { (address as *mut Self).read_volatile() };
        if page.service_generation != service_generation
            || page.endpoint_generation != endpoint_generation
            || !endpoint.valid()
        {
            return None;
        }
        let record = page.records.iter_mut().find(|item| item.endpoint == endpoint)?;
        let value = *record;
        *record = logos_abi::NetworkStreamRecord::EMPTY;
        unsafe { (address as *mut Self).write_volatile(page) };
        Some(value)
    }

    pub unsafe fn take_next_at(
        address: u64,
        service_generation: u32,
        endpoint_generation: u32,
    ) -> Option<logos_abi::NetworkStreamRecord> {
        let mut page = unsafe { (address as *mut Self).read_volatile() };
        if page.service_generation != service_generation
            || page.endpoint_generation != endpoint_generation
        {
            return None;
        }
        let record = page.records.iter_mut().find(|item| item.endpoint.valid())?;
        let value = *record;
        *record = logos_abi::NetworkStreamRecord::EMPTY;
        unsafe { (address as *mut Self).write_volatile(page) };
        Some(value)
    }

    pub unsafe fn overflow_at(
        address: u64,
        service_generation: u32,
        endpoint_generation: u32,
    ) -> bool {
        let page = unsafe { (address as *const Self).read_volatile() };
        page.service_generation == service_generation
            && page.endpoint_generation == endpoint_generation
            && page.overflow != 0
    }

    pub unsafe fn clear_overflow_at(
        address: u64,
        service_generation: u32,
        endpoint_generation: u32,
    ) -> bool {
        let mut page = unsafe { (address as *mut Self).read_volatile() };
        if page.service_generation != service_generation
            || page.endpoint_generation != endpoint_generation
        {
            return false;
        }
        page.overflow = 0;
        unsafe { (address as *mut Self).write_volatile(page) };
        true
    }
}

/// Client-owned Network request/reply page.
#[derive(Clone, Copy)]
#[repr(C)]
pub struct NetworkClientPage {
    pub service_generation: u32,
    pub endpoint_generation: u32,
    pub state: u32,
    pub request_id: u32,
    pub operation: u32,
    pub endpoint: u32,
    pub peer: u64,
    pub page: u32,
    pub length: u16,
    pub generation: u16,
    pub deadline: u64,
    pub transfer_page: u32,
    pub reply_status: u32,
    pub reply_endpoint: u32,
    pub reply_generation: u16,
    pub reply_source_port: u16,
    pub reply_source_address: u32,
    pub reply_length: u16,
    pub reserved0: u16,
    pub reply_stream_readiness: u16,
    pub reply_stream_reserved: u16,
    pub reply_stream_accepted_bytes: u64,
    pub reply_stream_acknowledged_bytes: u64,
    pub reply_info: logos_abi::NetworkInfo,
    pub reply_counters: logos_abi::NetworkCounters,
}

/// Network-owned server request/reply page.
#[derive(Clone, Copy)]
#[repr(C)]
pub struct NetworkServerPage {
    pub service_generation: u32,
    pub endpoint_generation: u32,
    pub state: u32,
    pub request_id: u32,
    pub caller_low: u32,
    pub caller_high: u32,
    pub operation: u32,
    pub endpoint: u32,
    pub peer: u64,
    pub page: u32,
    pub length: u16,
    pub generation: u16,
    pub deadline: u64,
    pub transfer_page: u32,
    pub reply_status: u32,
    pub reply_endpoint: u32,
    pub reply_generation: u16,
    pub reply_source_port: u16,
    pub reply_source_address: u32,
    pub reply_length: u16,
    pub reserved0: u16,
    pub reply_stream_readiness: u16,
    pub reply_stream_reserved: u16,
    pub reply_stream_accepted_bytes: u64,
    pub reply_stream_acknowledged_bytes: u64,
    pub reply_info: logos_abi::NetworkInfo,
    pub reply_counters: logos_abi::NetworkCounters,
}

#[derive(Clone, Copy)]
pub struct NetworkServerRequest {
    pub id: u32,
    pub caller: u64,
    pub request: logos_abi::NetworkRequest,
}

#[allow(clippy::too_many_arguments)]
fn network_request_from_fields(
    id: u32,
    operation: u32,
    endpoint: u32,
    peer: u64,
    page: u32,
    length: u16,
    generation: u16,
    deadline: u64,
) -> Option<logos_abi::NetworkRequest> {
    let request = logos_abi::NetworkRequest {
        id,
        operation: logos_abi::NetworkOperation::from_wire(u8::try_from(operation).ok()?)?,
        endpoint: logos_abi::NetworkEndpoint(endpoint),
        peer: logos_abi::NetworkScope(peer),
        page: logos_abi::PageHandle(page),
        length,
        generation,
        deadline,
    };
    request.valid_shape().then_some(request)
}

#[allow(clippy::too_many_arguments)]
fn network_reply_from_page(
    id: u32,
    status: u32,
    endpoint: u32,
    generation: u16,
    source_address: u32,
    source_port: u16,
    length: u16,
    stream_readiness: u16,
    stream_reserved: u16,
    stream_accepted_bytes: u64,
    stream_acknowledged_bytes: u64,
    info: logos_abi::NetworkInfo,
    counters: logos_abi::NetworkCounters,
) -> Option<logos_abi::NetworkReply> {
    Some(logos_abi::NetworkReply {
        id,
        status: logos_abi::NetworkStatus::from_wire(u8::try_from(status).ok()?)?,
        endpoint: logos_abi::NetworkEndpoint(endpoint),
        generation,
        source_address,
        source_port,
        length,
        stream_readiness,
        stream_reserved,
        stream_accepted_bytes,
        stream_acknowledged_bytes,
        info,
        counters,
    })
}

fn network_reply_state(status: logos_abi::NetworkStatus) -> NetworkPageState {
    match status {
        logos_abi::NetworkStatus::Complete => NetworkPageState::Reply,
        logos_abi::NetworkStatus::Denied => NetworkPageState::Denied,
        logos_abi::NetworkStatus::Cancelled => NetworkPageState::Cancelled,
        logos_abi::NetworkStatus::TimedOut => NetworkPageState::TimedOut,
        _ => NetworkPageState::Failed,
    }
}

#[allow(clippy::too_many_arguments)]
fn set_network_reply(
    state: &mut u32,
    reply_status: &mut u32,
    reply_endpoint: &mut u32,
    reply_generation: &mut u16,
    reply_source_address: &mut u32,
    reply_source_port: &mut u16,
    reply_length: &mut u16,
    reply_stream_readiness: &mut u16,
    reply_stream_reserved: &mut u16,
    reply_stream_accepted_bytes: &mut u64,
    reply_stream_acknowledged_bytes: &mut u64,
    reply_info: &mut logos_abi::NetworkInfo,
    reply_counters: &mut logos_abi::NetworkCounters,
    request: logos_abi::NetworkRequest,
    reply: logos_abi::NetworkReply,
) -> bool {
    if !reply.valid_for(request) {
        return false;
    }
    *reply_status = reply.status as u32;
    *reply_endpoint = reply.endpoint.0;
    *reply_generation = reply.generation;
    *reply_source_address = reply.source_address;
    *reply_source_port = reply.source_port;
    *reply_length = reply.length;
    *reply_stream_readiness = reply.stream_readiness;
    *reply_stream_reserved = reply.stream_reserved;
    *reply_stream_accepted_bytes = reply.stream_accepted_bytes;
    *reply_stream_acknowledged_bytes = reply.stream_acknowledged_bytes;
    *reply_info = reply.info;
    *reply_counters = reply.counters;
    *state = network_reply_state(reply.status) as u32;
    true
}

#[allow(clippy::missing_safety_doc)]
impl NetworkClientPage {
    pub const fn new(service_generation: u32, endpoint_generation: u32) -> Self {
        Self {
            service_generation,
            endpoint_generation,
            state: NetworkPageState::Ready as u32,
            request_id: 0,
            operation: 0,
            endpoint: 0,
            peer: 0,
            page: 0,
            length: 0,
            generation: 0,
            deadline: 0,
            transfer_page: 0,
            reply_status: 0,
            reply_endpoint: 0,
            reply_generation: 0,
            reply_source_port: 0,
            reply_source_address: 0,
            reply_length: 0,
            reserved0: 0,
            reply_stream_readiness: 0,
            reply_stream_reserved: 0,
            reply_stream_accepted_bytes: 0,
            reply_stream_acknowledged_bytes: 0,
            reply_info: logos_abi::NetworkInfo {
                mac: [0; 6],
                mtu: 0,
                generation: 0,
                link_up: 0,
                configuration: 0,
                ipv4: 0,
                subnet_mask: 0,
                router: 0,
            },
            reply_counters: logos_abi::NetworkCounters {
                rx_frames: 0,
                tx_frames: 0,
                rx_bytes: 0,
                tx_bytes: 0,
                malformed: 0,
                unsupported: 0,
                rx_dropped: 0,
                udp_no_endpoint: 0,
                udp_queue_dropped: 0,
                timeouts: 0,
                cancellations: 0,
                resets: 0,
                denied: 0,
            },
        }
    }

    pub unsafe fn reset_at(
        address: u64,
        service_generation: u32,
        endpoint_generation: u32,
    ) -> bool {
        if !valid_page_identity::<Self>(address, service_generation, endpoint_generation) {
            return false;
        }
        let old = unsafe { (address as *const Self).read_volatile() };
        let mut page = Self::new(service_generation, endpoint_generation);
        page.transfer_page = old.transfer_page;
        unsafe { (address as *mut Self).write_volatile(page) };
        true
    }

    pub unsafe fn configure_transfer_at(
        address: u64,
        service_generation: u32,
        endpoint_generation: u32,
        handle: logos_abi::PageHandle,
    ) -> bool {
        if handle.0 == 0 {
            return false;
        }
        let mut page = unsafe { (address as *mut Self).read_volatile() };
        if !client_identity(&page, service_generation, endpoint_generation)
            || NetworkPageState::from_wire(page.state) != Some(NetworkPageState::Ready)
        {
            return false;
        }
        page.transfer_page = handle.0;
        unsafe { (address as *mut Self).write_volatile(page) };
        true
    }

    pub unsafe fn transfer_page_at(
        address: u64,
        service_generation: u32,
        endpoint_generation: u32,
    ) -> Option<logos_abi::PageHandle> {
        let page = unsafe { (address as *const Self).read_volatile() };
        (client_identity(&page, service_generation, endpoint_generation) && page.transfer_page != 0)
            .then_some(logos_abi::PageHandle(page.transfer_page))
    }

    pub unsafe fn request_at(
        address: u64,
        service_generation: u32,
        endpoint_generation: u32,
        request: logos_abi::NetworkRequest,
    ) -> bool {
        if !request.valid_shape() {
            return false;
        }
        let mut page = unsafe { (address as *mut Self).read_volatile() };
        if !client_identity(&page, service_generation, endpoint_generation)
            || NetworkPageState::from_wire(page.state) != Some(NetworkPageState::Ready)
        {
            return false;
        }
        if matches!(
            request.operation,
            logos_abi::NetworkOperation::SendTo
                | logos_abi::NetworkOperation::ReceiveFrom
                | logos_abi::NetworkOperation::Read
                | logos_abi::NetworkOperation::Write
        ) && page.transfer_page != request.page.0
        {
            return false;
        }
        page.request_id = request.id;
        page.operation = request.operation as u32;
        page.endpoint = request.endpoint.0;
        page.peer = request.peer.0;
        page.page = request.page.0;
        page.length = request.length;
        page.generation = request.generation;
        page.deadline = request.deadline;
        page.reply_status = 0;
        page.state = NetworkPageState::Request as u32;
        unsafe { (address as *mut Self).write_volatile(page) };
        true
    }

    pub unsafe fn pending_at(
        address: u64,
        service_generation: u32,
        endpoint_generation: u32,
    ) -> bool {
        let page = unsafe { (address as *const Self).read_volatile() };
        client_identity(&page, service_generation, endpoint_generation)
            && NetworkPageState::from_wire(page.state) == Some(NetworkPageState::Request)
    }

    pub unsafe fn request_at_page(
        address: u64,
        service_generation: u32,
        endpoint_generation: u32,
    ) -> Option<logos_abi::NetworkRequest> {
        let page = unsafe { (address as *const Self).read_volatile() };
        if !client_identity(&page, service_generation, endpoint_generation)
            || NetworkPageState::from_wire(page.state) != Some(NetworkPageState::Request)
        {
            return None;
        }
        network_request_from_fields(
            page.request_id,
            page.operation,
            page.endpoint,
            page.peer,
            page.page,
            page.length,
            page.generation,
            page.deadline,
        )
    }

    pub unsafe fn mark_processing_at(
        address: u64,
        service_generation: u32,
        endpoint_generation: u32,
    ) -> bool {
        let mut page = unsafe { (address as *mut Self).read_volatile() };
        if !client_identity(&page, service_generation, endpoint_generation)
            || NetworkPageState::from_wire(page.state) != Some(NetworkPageState::Request)
        {
            return false;
        }
        page.state = NetworkPageState::Processing as u32;
        unsafe { (address as *mut Self).write_volatile(page) };
        true
    }

    pub unsafe fn reply_at(
        address: u64,
        service_generation: u32,
        endpoint_generation: u32,
        reply: logos_abi::NetworkReply,
    ) -> bool {
        let mut page = unsafe { (address as *mut Self).read_volatile() };
        let Some(request) =
            unsafe { Self::request_at_page(address, service_generation, endpoint_generation) }
                .or_else(|| {
                    network_request_from_fields(
                        page.request_id,
                        page.operation,
                        page.endpoint,
                        page.peer,
                        page.page,
                        page.length,
                        page.generation,
                        page.deadline,
                    )
                })
        else {
            return false;
        };
        if !client_identity(&page, service_generation, endpoint_generation)
            || !matches!(
                NetworkPageState::from_wire(page.state),
                Some(NetworkPageState::Processing)
            )
        {
            return false;
        }
        set_network_reply(
            &mut page.state,
            &mut page.reply_status,
            &mut page.reply_endpoint,
            &mut page.reply_generation,
            &mut page.reply_source_address,
            &mut page.reply_source_port,
            &mut page.reply_length,
            &mut page.reply_stream_readiness,
            &mut page.reply_stream_reserved,
            &mut page.reply_stream_accepted_bytes,
            &mut page.reply_stream_acknowledged_bytes,
            &mut page.reply_info,
            &mut page.reply_counters,
            request,
            reply,
        ) && unsafe {
            (address as *mut Self).write_volatile(page);
            true
        }
    }

    pub unsafe fn reply_request_at(
        address: u64,
        service_generation: u32,
        endpoint_generation: u32,
        reply: logos_abi::NetworkReply,
    ) -> bool {
        let mut page = unsafe { (address as *mut Self).read_volatile() };
        let Some(request) =
            (unsafe { Self::request_at_page(address, service_generation, endpoint_generation) })
        else {
            return false;
        };
        if !client_identity(&page, service_generation, endpoint_generation)
            || NetworkPageState::from_wire(page.state) != Some(NetworkPageState::Request)
        {
            return false;
        }
        set_network_reply(
            &mut page.state,
            &mut page.reply_status,
            &mut page.reply_endpoint,
            &mut page.reply_generation,
            &mut page.reply_source_address,
            &mut page.reply_source_port,
            &mut page.reply_length,
            &mut page.reply_stream_readiness,
            &mut page.reply_stream_reserved,
            &mut page.reply_stream_accepted_bytes,
            &mut page.reply_stream_acknowledged_bytes,
            &mut page.reply_info,
            &mut page.reply_counters,
            request,
            reply,
        ) && unsafe {
            (address as *mut Self).write_volatile(page);
            true
        }
    }

    pub unsafe fn finish_at(
        address: u64,
        service_generation: u32,
        endpoint_generation: u32,
        expected_id: u32,
    ) -> Option<logos_abi::NetworkReply> {
        let page = unsafe { (address as *const Self).read_volatile() };
        if !client_identity(&page, service_generation, endpoint_generation)
            || page.request_id != expected_id
            || !matches!(
                NetworkPageState::from_wire(page.state),
                Some(
                    NetworkPageState::Reply
                        | NetworkPageState::Denied
                        | NetworkPageState::Failed
                        | NetworkPageState::Cancelled
                        | NetworkPageState::TimedOut
                )
            )
        {
            return None;
        }
        let reply = network_reply_from_page(
            page.request_id,
            page.reply_status,
            page.reply_endpoint,
            page.reply_generation,
            page.reply_source_address,
            page.reply_source_port,
            page.reply_length,
            page.reply_stream_readiness,
            page.reply_stream_reserved,
            page.reply_stream_accepted_bytes,
            page.reply_stream_acknowledged_bytes,
            page.reply_info,
            page.reply_counters,
        )?;
        unsafe { Self::reset_at(address, service_generation, endpoint_generation) };
        Some(reply)
    }
}

#[allow(clippy::missing_safety_doc)]
impl NetworkServerPage {
    pub const fn new(service_generation: u32, endpoint_generation: u32) -> Self {
        Self {
            service_generation,
            endpoint_generation,
            state: NetworkPageState::Ready as u32,
            request_id: 0,
            caller_low: 0,
            caller_high: 0,
            operation: 0,
            endpoint: 0,
            peer: 0,
            page: 0,
            length: 0,
            generation: 0,
            deadline: 0,
            transfer_page: 0,
            reply_status: 0,
            reply_endpoint: 0,
            reply_generation: 0,
            reply_source_port: 0,
            reply_source_address: 0,
            reply_length: 0,
            reserved0: 0,
            reply_stream_readiness: 0,
            reply_stream_reserved: 0,
            reply_stream_accepted_bytes: 0,
            reply_stream_acknowledged_bytes: 0,
            reply_info: logos_abi::NetworkInfo {
                mac: [0; 6],
                mtu: 0,
                generation: 0,
                link_up: 0,
                configuration: 0,
                ipv4: 0,
                subnet_mask: 0,
                router: 0,
            },
            reply_counters: logos_abi::NetworkCounters {
                rx_frames: 0,
                tx_frames: 0,
                rx_bytes: 0,
                tx_bytes: 0,
                malformed: 0,
                unsupported: 0,
                rx_dropped: 0,
                udp_no_endpoint: 0,
                udp_queue_dropped: 0,
                timeouts: 0,
                cancellations: 0,
                resets: 0,
                denied: 0,
            },
        }
    }

    pub unsafe fn reset_at(
        address: u64,
        service_generation: u32,
        endpoint_generation: u32,
    ) -> bool {
        if !valid_page_identity::<Self>(address, service_generation, endpoint_generation) {
            return false;
        }
        unsafe {
            (address as *mut Self)
                .write_volatile(Self::new(service_generation, endpoint_generation))
        };
        true
    }

    pub unsafe fn deliver_at(
        address: u64,
        service_generation: u32,
        endpoint_generation: u32,
        caller: u64,
        request: logos_abi::NetworkRequest,
    ) -> bool {
        if !request.valid_shape() {
            return false;
        }
        let mut page = unsafe { (address as *mut Self).read_volatile() };
        if !server_identity(&page, service_generation, endpoint_generation)
            || NetworkPageState::from_wire(page.state) != Some(NetworkPageState::Ready)
        {
            return false;
        }
        page.request_id = request.id;
        page.caller_low = caller as u32;
        page.caller_high = (caller >> 32) as u32;
        page.operation = request.operation as u32;
        page.endpoint = request.endpoint.0;
        page.peer = request.peer.0;
        page.page = request.page.0;
        page.length = request.length;
        page.generation = request.generation;
        page.deadline = request.deadline;
        page.state = NetworkPageState::Request as u32;
        unsafe { (address as *mut Self).write_volatile(page) };
        true
    }

    pub unsafe fn take_at(
        address: u64,
        service_generation: u32,
        endpoint_generation: u32,
    ) -> Option<NetworkServerRequest> {
        let mut page = unsafe { (address as *mut Self).read_volatile() };
        if !server_identity(&page, service_generation, endpoint_generation)
            || NetworkPageState::from_wire(page.state) != Some(NetworkPageState::Request)
        {
            return None;
        }
        let request = network_request_from_fields(
            page.request_id,
            page.operation,
            page.endpoint,
            page.peer,
            page.page,
            page.length,
            page.generation,
            page.deadline,
        )?;
        page.state = NetworkPageState::Processing as u32;
        unsafe { (address as *mut Self).write_volatile(page) };
        Some(NetworkServerRequest {
            id: request.id,
            caller: u64::from(page.caller_low) | (u64::from(page.caller_high) << 32),
            request,
        })
    }

    pub unsafe fn pending_at(
        address: u64,
        service_generation: u32,
        endpoint_generation: u32,
    ) -> bool {
        let page = unsafe { (address as *const Self).read_volatile() };
        server_identity(&page, service_generation, endpoint_generation)
            && NetworkPageState::from_wire(page.state) == Some(NetworkPageState::Request)
    }

    pub unsafe fn reply_pending_at(
        address: u64,
        service_generation: u32,
        endpoint_generation: u32,
    ) -> bool {
        let page = unsafe { (address as *const Self).read_volatile() };
        server_identity(&page, service_generation, endpoint_generation)
            && matches!(
                NetworkPageState::from_wire(page.state),
                Some(
                    NetworkPageState::Reply
                        | NetworkPageState::Denied
                        | NetworkPageState::Failed
                        | NetworkPageState::Cancelled
                        | NetworkPageState::TimedOut
                )
            )
    }

    pub unsafe fn reply_at(
        address: u64,
        service_generation: u32,
        endpoint_generation: u32,
        reply: logos_abi::NetworkReply,
    ) -> bool {
        let mut page = unsafe { (address as *mut Self).read_volatile() };
        let Some(request) = network_request_from_fields(
            page.request_id,
            page.operation,
            page.endpoint,
            page.peer,
            page.page,
            page.length,
            page.generation,
            page.deadline,
        ) else {
            return false;
        };
        if !server_identity(&page, service_generation, endpoint_generation)
            || NetworkPageState::from_wire(page.state) != Some(NetworkPageState::Processing)
        {
            return false;
        }
        set_network_reply(
            &mut page.state,
            &mut page.reply_status,
            &mut page.reply_endpoint,
            &mut page.reply_generation,
            &mut page.reply_source_address,
            &mut page.reply_source_port,
            &mut page.reply_length,
            &mut page.reply_stream_readiness,
            &mut page.reply_stream_reserved,
            &mut page.reply_stream_accepted_bytes,
            &mut page.reply_stream_acknowledged_bytes,
            &mut page.reply_info,
            &mut page.reply_counters,
            request,
            reply,
        ) && unsafe {
            (address as *mut Self).write_volatile(page);
            true
        }
    }

    pub unsafe fn finish_at(
        address: u64,
        service_generation: u32,
        endpoint_generation: u32,
        expected_id: u32,
    ) -> Option<logos_abi::NetworkReply> {
        let page = unsafe { (address as *const Self).read_volatile() };
        if !server_identity(&page, service_generation, endpoint_generation)
            || page.request_id != expected_id
            || !matches!(
                NetworkPageState::from_wire(page.state),
                Some(
                    NetworkPageState::Reply
                        | NetworkPageState::Denied
                        | NetworkPageState::Failed
                        | NetworkPageState::Cancelled
                        | NetworkPageState::TimedOut
                )
            )
        {
            return None;
        }
        let reply = network_reply_from_page(
            page.request_id,
            page.reply_status,
            page.reply_endpoint,
            page.reply_generation,
            page.reply_source_address,
            page.reply_source_port,
            page.reply_length,
            page.reply_stream_readiness,
            page.reply_stream_reserved,
            page.reply_stream_accepted_bytes,
            page.reply_stream_acknowledged_bytes,
            page.reply_info,
            page.reply_counters,
        )?;
        unsafe { Self::reset_at(address, service_generation, endpoint_generation) };
        Some(reply)
    }
}

fn valid_network_page_address<T>(address: u64) -> bool {
    address != 0 && address.is_multiple_of(align_of::<T>() as u64)
}

fn valid_network_device_identity(
    page: &NetworkDevicePage,
    service_generation: u32,
    endpoint_generation: u32,
    device_generation: u32,
) -> bool {
    page.service_generation == service_generation
        && page.endpoint_generation == endpoint_generation
        && page.device_generation == device_generation
        && service_generation != 0
        && endpoint_generation != 0
        && device_generation != 0
}

fn valid_network_event_identity(
    page: &NetworkEventPage,
    service_generation: u32,
    endpoint_generation: u32,
    device_generation: u32,
) -> bool {
    page.service_generation == service_generation
        && page.endpoint_generation == endpoint_generation
        && page.device_generation == device_generation
        && service_generation != 0
        && endpoint_generation != 0
        && device_generation != 0
}

impl NetworkDevicePage {
    pub const fn new(
        service_generation: u32,
        endpoint_generation: u32,
        device_generation: u32,
    ) -> Self {
        Self {
            service_generation,
            endpoint_generation,
            device_generation,
            state: NetworkDevicePageState::Ready as u32,
            request_id: 0,
            operation: 0,
            rx_page: 0,
            tx_page: 0,
            length: 0,
            deadline: 0,
            reply_status: 0,
            reset_generation: 0,
            info: logos_abi::NetworkInfo {
                mac: [0; 6],
                mtu: 0,
                generation: 0,
                link_up: 0,
                configuration: 0,
                ipv4: 0,
                subnet_mask: 0,
                router: 0,
            },
            metadata: [0; 32],
            reserved: [0; logos_abi::PAGE_SIZE - 112],
        }
    }

    /// # Safety
    /// `address` must point to a live, aligned NetworkDevicePage mapping.
    pub unsafe fn reset_at(
        address: u64,
        service_generation: u32,
        endpoint_generation: u32,
        device_generation: u32,
        rx_page: logos_abi::PageHandle,
        tx_page: logos_abi::PageHandle,
    ) -> bool {
        if !valid_network_page_address::<Self>(address)
            || rx_page.0 == 0
            || tx_page.0 == 0
            || rx_page == tx_page
        {
            return false;
        }
        unsafe {
            (address as *mut Self).write_volatile(Self {
                rx_page: rx_page.0,
                tx_page: tx_page.0,
                ..Self::new(service_generation, endpoint_generation, device_generation)
            })
        };
        true
    }

    /// # Safety
    /// Core replaces the device generation while publishing the matching reset result.
    pub unsafe fn reset_with_reply_at(
        address: u64,
        service_generation: u32,
        endpoint_generation: u32,
        device_generation: u32,
        rx_page: logos_abi::PageHandle,
        tx_page: logos_abi::PageHandle,
        reply: logos_abi::NetworkDeviceReply,
    ) -> bool {
        if !valid_network_page_address::<Self>(address)
            || device_generation == 0
            || rx_page.0 == 0
            || tx_page.0 == 0
            || rx_page == tx_page
        {
            return false;
        }
        let old = unsafe { (address as *const Self).read_volatile() };
        if old.service_generation == 0
            || old.endpoint_generation == 0
            || old.device_generation == 0
            || NetworkDevicePageState::from_wire(old.state)
                != Some(NetworkDevicePageState::Submitted)
            || old.request_id == 0
            || reply.id != old.request_id
        {
            return false;
        }
        let mut page = Self::new(service_generation, endpoint_generation, device_generation);
        page.request_id = old.request_id;
        page.operation = old.operation;
        page.tx_page = tx_page.0;
        page.rx_page = rx_page.0;
        page.length = old.length;
        page.deadline = old.deadline;
        page.reply_status = reply.status as u32;
        page.reset_generation = u32::from(reply.generation);
        page.info = reply.info;
        page.state = NetworkDevicePageState::Reply as u32;
        unsafe { (address as *mut Self).write_volatile(page) };
        true
    }

    /// # Safety
    /// Core replaces the service generation while retaining Core-owned DMA identities.
    pub unsafe fn reset_generation_at(
        address: u64,
        service_generation: u32,
        endpoint_generation: u32,
    ) -> bool {
        if !valid_network_page_address::<Self>(address) {
            return false;
        }
        let old = unsafe { (address as *const Self).read_volatile() };
        if old.service_generation == 0 || old.endpoint_generation == 0 || old.device_generation == 0
        {
            return false;
        }
        unsafe {
            (address as *mut Self).write_volatile(Self {
                rx_page: old.rx_page,
                tx_page: old.tx_page,
                ..Self::new(service_generation, endpoint_generation, old.device_generation)
            })
        };
        true
    }

    /// # Safety
    /// Core configures a newly mapped page before the service starts.
    pub unsafe fn configure_at(
        address: u64,
        service_generation: u32,
        endpoint_generation: u32,
        device_generation: u32,
        rx_page: logos_abi::PageHandle,
        tx_page: logos_abi::PageHandle,
    ) -> bool {
        if !valid_network_page_address::<Self>(address)
            || rx_page.0 == 0
            || tx_page.0 == 0
            || rx_page == tx_page
        {
            return false;
        }
        let mut page = unsafe { (address as *mut Self).read_volatile() };
        if !valid_network_device_identity(
            &page,
            service_generation,
            endpoint_generation,
            device_generation,
        ) || NetworkDevicePageState::from_wire(page.state) != Some(NetworkDevicePageState::Ready)
            || page.rx_page != 0
            || page.tx_page != 0
        {
            return false;
        }
        page.rx_page = rx_page.0;
        page.tx_page = tx_page.0;
        unsafe { (address as *mut Self).write_volatile(page) };
        true
    }

    /// # Safety
    /// The Network service owns request creation.
    pub unsafe fn request_at(
        address: u64,
        service_generation: u32,
        endpoint_generation: u32,
        device_generation: u32,
        request: logos_abi::NetworkDeviceRequest,
    ) -> bool {
        if !request.valid_shape()
            || !valid_network_page_address::<Self>(address)
            || request.generation != 0 && u32::from(request.generation) != device_generation
        {
            return false;
        }
        let mut page = unsafe { (address as *mut Self).read_volatile() };
        if !valid_network_device_identity(
            &page,
            service_generation,
            endpoint_generation,
            device_generation,
        ) || NetworkDevicePageState::from_wire(page.state) != Some(NetworkDevicePageState::Ready)
            || page.rx_page == 0
            || page.tx_page == 0
        {
            return false;
        }
        page.request_id = request.id;
        page.operation = request.operation as u32;
        page.length = u32::from(request.length);
        page.deadline = request.deadline;
        page.reply_status = 0;
        page.reset_generation = 0;
        page.info = logos_abi::NetworkInfo::default();
        page.state = NetworkDevicePageState::Request as u32;
        unsafe { (address as *mut Self).write_volatile(page) };
        true
    }

    /// # Safety
    /// Core consumes a service-created request.
    pub unsafe fn take_request_at(
        address: u64,
        service_generation: u32,
        endpoint_generation: u32,
        device_generation: u32,
    ) -> Option<NetworkDeviceMessage> {
        if !valid_network_page_address::<Self>(address) {
            return None;
        }
        let mut page = unsafe { (address as *mut Self).read_volatile() };
        if !valid_network_device_identity(
            &page,
            service_generation,
            endpoint_generation,
            device_generation,
        ) || NetworkDevicePageState::from_wire(page.state)
            != Some(NetworkDevicePageState::Request)
        {
            return None;
        }
        let operation =
            logos_abi::NetworkDeviceOperation::from_wire(u8::try_from(page.operation).ok()?)?;
        let length = u16::try_from(page.length).ok()?;
        let generation = match operation {
            logos_abi::NetworkDeviceOperation::Info => 0,
            logos_abi::NetworkDeviceOperation::Transmit
            | logos_abi::NetworkDeviceOperation::Reset => u16::try_from(device_generation).ok()?,
        };
        let request = logos_abi::NetworkDeviceRequest {
            id: page.request_id,
            operation,
            length,
            generation,
            deadline: page.deadline,
        };
        if !request.valid_shape()
            || page.request_id == 0
            || (operation == logos_abi::NetworkDeviceOperation::Transmit && page.tx_page == 0)
        {
            return None;
        }
        page.state = NetworkDevicePageState::Submitted as u32;
        unsafe { (address as *mut Self).write_volatile(page) };
        Some(NetworkDeviceMessage {
            request,
            rx_page: logos_abi::PageHandle(page.rx_page),
            tx_page: logos_abi::PageHandle(page.tx_page),
        })
    }

    /// # Safety
    /// Core completes the current request with a validated driver result.
    pub unsafe fn complete_at(
        address: u64,
        service_generation: u32,
        endpoint_generation: u32,
        device_generation: u32,
        reply: logos_abi::NetworkDeviceReply,
    ) -> bool {
        if !valid_network_page_address::<Self>(address) {
            return false;
        }
        let mut page = unsafe { (address as *mut Self).read_volatile() };
        if !valid_network_device_identity(
            &page,
            service_generation,
            endpoint_generation,
            device_generation,
        ) || NetworkDevicePageState::from_wire(page.state)
            != Some(NetworkDevicePageState::Submitted)
        {
            return false;
        }
        let operation = match logos_abi::NetworkDeviceOperation::from_wire(
            u8::try_from(page.operation).ok().unwrap_or(0),
        ) {
            Some(operation) => operation,
            None => return false,
        };
        let request = logos_abi::NetworkDeviceRequest {
            id: page.request_id,
            operation,
            length: u16::try_from(page.length).ok().unwrap_or(0),
            generation: if operation == logos_abi::NetworkDeviceOperation::Info {
                0
            } else {
                u16::try_from(device_generation).ok().unwrap_or(0)
            },
            deadline: page.deadline,
        };
        if !request.valid_shape() || !reply.valid_for(request) {
            return false;
        }
        page.reply_status = reply.status as u32;
        page.reset_generation = u32::from(reply.generation);
        page.info = reply.info;
        page.state = NetworkDevicePageState::Reply as u32;
        unsafe { (address as *mut Self).write_volatile(page) };
        true
    }

    /// # Safety
    /// The Network service consumes only its matching completion.
    pub unsafe fn take_reply_at(
        address: u64,
        service_generation: u32,
        endpoint_generation: u32,
        device_generation: u32,
        expected_id: u32,
    ) -> Option<logos_abi::NetworkDeviceReply> {
        if !valid_network_page_address::<Self>(address) {
            return None;
        }
        let mut page = unsafe { (address as *mut Self).read_volatile() };
        if !valid_network_device_identity(
            &page,
            service_generation,
            endpoint_generation,
            device_generation,
        ) || NetworkDevicePageState::from_wire(page.state) != Some(NetworkDevicePageState::Reply)
            || page.request_id != expected_id
        {
            return None;
        }
        let status = logos_abi::NetworkStatus::from_wire(u8::try_from(page.reply_status).ok()?)?;
        let reply = logos_abi::NetworkDeviceReply {
            id: page.request_id,
            status,
            generation: u16::try_from(page.reset_generation).ok()?,
            info: page.info,
        };
        page.request_id = 0;
        page.operation = 0;
        page.length = 0;
        page.deadline = 0;
        page.reply_status = 0;
        page.reset_generation = 0;
        page.info = logos_abi::NetworkInfo::default();
        page.state = NetworkDevicePageState::Ready as u32;
        unsafe { (address as *mut Self).write_volatile(page) };
        Some(reply)
    }

    /// # Safety
    /// Core reads only scalar state.
    pub unsafe fn pending_at(
        address: u64,
        service_generation: u32,
        endpoint_generation: u32,
        device_generation: u32,
    ) -> bool {
        if !valid_network_page_address::<Self>(address) {
            return false;
        }
        let page = unsafe { (address as *const Self).read_volatile() };
        valid_network_device_identity(
            &page,
            service_generation,
            endpoint_generation,
            device_generation,
        ) && matches!(
            NetworkDevicePageState::from_wire(page.state),
            Some(NetworkDevicePageState::Request | NetworkDevicePageState::Submitted)
        )
    }

    /// # Safety
    /// Core reads only the generation and state fields from the endpoint mapping.
    pub unsafe fn active_for_core_at(
        address: u64,
        service_generation: u32,
        endpoint_generation: u32,
    ) -> bool {
        if !valid_network_page_address::<Self>(address) {
            return false;
        }
        let page = unsafe { (address as *const Self).read_volatile() };
        page.service_generation == service_generation
            && page.endpoint_generation == endpoint_generation
            && service_generation != 0
            && endpoint_generation != 0
            && page.device_generation != 0
            && matches!(
                NetworkDevicePageState::from_wire(page.state),
                Some(
                    NetworkDevicePageState::Request
                        | NetworkDevicePageState::Submitted
                        | NetworkDevicePageState::Reply
                )
            )
    }

    /// # Safety
    /// Core reads only scalar state.
    pub unsafe fn reply_pending_at(
        address: u64,
        service_generation: u32,
        endpoint_generation: u32,
        device_generation: u32,
    ) -> bool {
        if !valid_network_page_address::<Self>(address) {
            return false;
        }
        let page = unsafe { (address as *const Self).read_volatile() };
        valid_network_device_identity(
            &page,
            service_generation,
            endpoint_generation,
            device_generation,
        ) && NetworkDevicePageState::from_wire(page.state) == Some(NetworkDevicePageState::Reply)
    }

    /// # Safety
    /// The Network service reads configured DMA identities.
    pub unsafe fn dma_at(
        address: u64,
        service_generation: u32,
        endpoint_generation: u32,
        device_generation: u32,
    ) -> Option<(logos_abi::PageHandle, logos_abi::PageHandle)> {
        if !valid_network_page_address::<Self>(address) {
            return None;
        }
        let page = unsafe { (address as *const Self).read_volatile() };
        if !valid_network_device_identity(
            &page,
            service_generation,
            endpoint_generation,
            device_generation,
        ) || page.rx_page == 0
            || page.tx_page == 0
            || page.rx_page == page.tx_page
        {
            return None;
        }
        Some((logos_abi::PageHandle(page.rx_page), logos_abi::PageHandle(page.tx_page)))
    }
}

impl NetworkEventPage {
    pub const fn new(
        service_generation: u32,
        endpoint_generation: u32,
        device_generation: u32,
    ) -> Self {
        Self {
            service_generation,
            endpoint_generation,
            device_generation,
            state: NetworkEventPageState::Ready as u32,
            sequence: 0,
            kind: 0,
            transfer_page: 0,
            length: 0,
            deadline: 0,
            now: 0,
            generation: 0,
            reserved0: 0,
            metadata: [0; 32],
            configured_rx_page: 0,
            reserved: [0; logos_abi::PAGE_SIZE - 88],
        }
    }

    /// # Safety
    /// `address` must point to a live, aligned NetworkEventPage mapping.
    pub unsafe fn reset_at(
        address: u64,
        service_generation: u32,
        endpoint_generation: u32,
        device_generation: u32,
        rx_page: logos_abi::PageHandle,
    ) -> bool {
        if !valid_network_page_address::<Self>(address) || rx_page.0 == 0 {
            return false;
        }
        unsafe {
            (address as *mut Self).write_volatile(Self {
                configured_rx_page: rx_page.0,
                ..Self::new(service_generation, endpoint_generation, device_generation)
            })
        };
        true
    }

    /// # Safety
    /// Core replaces the service generation while retaining the RX identity.
    pub unsafe fn reset_generation_at(
        address: u64,
        service_generation: u32,
        endpoint_generation: u32,
    ) -> bool {
        if !valid_network_page_address::<Self>(address) {
            return false;
        }
        let old = unsafe { (address as *const Self).read_volatile() };
        if old.service_generation == 0 || old.endpoint_generation == 0 || old.device_generation == 0
        {
            return false;
        }
        unsafe {
            (address as *mut Self).write_volatile(Self {
                configured_rx_page: old.configured_rx_page,
                ..Self::new(service_generation, endpoint_generation, old.device_generation)
            })
        };
        true
    }

    /// # Safety
    /// Core configures a newly mapped page before the service starts.
    pub unsafe fn configure_at(
        address: u64,
        service_generation: u32,
        endpoint_generation: u32,
        device_generation: u32,
        rx_page: logos_abi::PageHandle,
    ) -> bool {
        if !valid_network_page_address::<Self>(address) || rx_page.0 == 0 {
            return false;
        }
        let mut page = unsafe { (address as *mut Self).read_volatile() };
        if !valid_network_event_identity(
            &page,
            service_generation,
            endpoint_generation,
            device_generation,
        ) || NetworkEventPageState::from_wire(page.state) != Some(NetworkEventPageState::Ready)
            || page.configured_rx_page != 0
        {
            return false;
        }
        page.configured_rx_page = rx_page.0;
        unsafe { (address as *mut Self).write_volatile(page) };
        true
    }

    /// # Safety
    /// The Network service owns wait creation.
    pub unsafe fn wait_at(
        address: u64,
        service_generation: u32,
        endpoint_generation: u32,
        device_generation: u32,
        deadline: u64,
    ) -> bool {
        if !valid_network_page_address::<Self>(address) || deadline == 0 {
            return false;
        }
        let mut page = unsafe { (address as *mut Self).read_volatile() };
        if !valid_network_event_identity(
            &page,
            service_generation,
            endpoint_generation,
            device_generation,
        ) || page.configured_rx_page == 0
            || NetworkEventPageState::from_wire(page.state) != Some(NetworkEventPageState::Ready)
        {
            return false;
        }
        page.deadline = deadline;
        page.state = NetworkEventPageState::Waiting as u32;
        unsafe { (address as *mut Self).write_volatile(page) };
        true
    }

    /// # Safety
    /// Core reads only scalar state.
    pub unsafe fn waiting_at(
        address: u64,
        service_generation: u32,
        endpoint_generation: u32,
        device_generation: u32,
    ) -> bool {
        if !valid_network_page_address::<Self>(address) {
            return false;
        }
        let page = unsafe { (address as *const Self).read_volatile() };
        valid_network_event_identity(
            &page,
            service_generation,
            endpoint_generation,
            device_generation,
        ) && NetworkEventPageState::from_wire(page.state) == Some(NetworkEventPageState::Waiting)
    }

    /// # Safety
    /// Core reads only scalar state.
    pub unsafe fn deadline_at(
        address: u64,
        service_generation: u32,
        endpoint_generation: u32,
        device_generation: u32,
    ) -> Option<u64> {
        if !valid_network_page_address::<Self>(address) {
            return None;
        }
        let page = unsafe { (address as *const Self).read_volatile() };
        (valid_network_event_identity(
            &page,
            service_generation,
            endpoint_generation,
            device_generation,
        ) && NetworkEventPageState::from_wire(page.state) == Some(NetworkEventPageState::Waiting))
        .then_some(page.deadline)
        .filter(|deadline| *deadline != 0)
    }

    /// # Safety
    /// Core delivers one event only while the service is waiting.
    pub unsafe fn deliver_at(
        address: u64,
        service_generation: u32,
        endpoint_generation: u32,
        device_generation: u32,
        event: logos_abi::NetworkEvent,
    ) -> bool {
        if !event.valid() || !valid_network_page_address::<Self>(address) {
            return false;
        }
        let mut page = unsafe { (address as *mut Self).read_volatile() };
        if !valid_network_event_identity(
            &page,
            service_generation,
            endpoint_generation,
            device_generation,
        ) || NetworkEventPageState::from_wire(page.state) != Some(NetworkEventPageState::Waiting)
            || event.device_generation != device_generation
            || event.generation != u16::try_from(device_generation).ok().unwrap_or(0)
        {
            return false;
        }
        if event.kind == logos_abi::NetworkEventKind::Frame {
            if event.page.0 != page.configured_rx_page {
                return false;
            }
        } else if event.page.0 != 0 || event.length != 0 {
            return false;
        }
        page.sequence = event.id;
        page.kind = event.kind as u32;
        page.transfer_page = event.page.0;
        page.length = u32::from(event.length);
        page.now = event.now;
        page.generation = event.generation;
        page.metadata = [0; 32];
        page.metadata[..event.metadata.len()].copy_from_slice(&event.metadata);
        page.state = NetworkEventPageState::Event as u32;
        unsafe { (address as *mut Self).write_volatile(page) };
        true
    }

    /// # Safety
    /// The Network service consumes the single delivered event.
    pub unsafe fn take_at(
        address: u64,
        service_generation: u32,
        endpoint_generation: u32,
        device_generation: u32,
    ) -> Option<logos_abi::NetworkEvent> {
        if !valid_network_page_address::<Self>(address) {
            return None;
        }
        let mut page = unsafe { (address as *mut Self).read_volatile() };
        if !valid_network_event_identity(
            &page,
            service_generation,
            endpoint_generation,
            device_generation,
        ) || NetworkEventPageState::from_wire(page.state) != Some(NetworkEventPageState::Event)
        {
            return None;
        }
        let kind = logos_abi::NetworkEventKind::from_wire(u8::try_from(page.kind).ok()?)?;
        let event = logos_abi::NetworkEvent {
            id: page.sequence,
            kind,
            generation: page.generation,
            device_generation,
            page: logos_abi::PageHandle(page.transfer_page),
            length: u16::try_from(page.length).ok()?,
            now: page.now,
            metadata: page.metadata[..16].try_into().ok()?,
        };
        if !event.valid() {
            return None;
        }
        page.state = NetworkEventPageState::Consumed as u32;
        unsafe { (address as *mut Self).write_volatile(page) };
        Some(event)
    }

    /// # Safety
    /// The Network service acknowledges and releases the event slot.
    pub unsafe fn acknowledge_at(
        address: u64,
        service_generation: u32,
        endpoint_generation: u32,
        device_generation: u32,
    ) -> bool {
        if !valid_network_page_address::<Self>(address) {
            return false;
        }
        let mut page = unsafe { (address as *mut Self).read_volatile() };
        if !valid_network_event_identity(
            &page,
            service_generation,
            endpoint_generation,
            device_generation,
        ) || NetworkEventPageState::from_wire(page.state)
            != Some(NetworkEventPageState::Consumed)
        {
            return false;
        }
        page.sequence = 0;
        page.kind = 0;
        page.transfer_page = 0;
        page.length = 0;
        page.deadline = 0;
        page.now = 0;
        page.generation = 0;
        page.metadata = [0; 32];
        page.state = NetworkEventPageState::Ready as u32;
        unsafe { (address as *mut Self).write_volatile(page) };
        true
    }

    /// # Safety
    /// Core reads only scalar state.
    pub unsafe fn pending_at(
        address: u64,
        service_generation: u32,
        endpoint_generation: u32,
        device_generation: u32,
    ) -> bool {
        if !valid_network_page_address::<Self>(address) {
            return false;
        }
        let page = unsafe { (address as *const Self).read_volatile() };
        valid_network_event_identity(
            &page,
            service_generation,
            endpoint_generation,
            device_generation,
        ) && NetworkEventPageState::from_wire(page.state) == Some(NetworkEventPageState::Event)
    }

    /// # Safety
    /// Core reads only the generation and state fields from the endpoint mapping.
    pub unsafe fn active_for_core_at(
        address: u64,
        service_generation: u32,
        endpoint_generation: u32,
    ) -> bool {
        if !valid_network_page_address::<Self>(address) {
            return false;
        }
        let page = unsafe { (address as *const Self).read_volatile() };
        page.service_generation == service_generation
            && page.endpoint_generation == endpoint_generation
            && service_generation != 0
            && endpoint_generation != 0
            && page.device_generation != 0
            && matches!(
                NetworkEventPageState::from_wire(page.state),
                Some(
                    NetworkEventPageState::Waiting
                        | NetworkEventPageState::Event
                        | NetworkEventPageState::Consumed
                )
            )
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct NetworkDmaResources {
    pub rx_handle: logos_abi::PageHandle,
    pub rx_address: u64,
    pub tx_handle: logos_abi::PageHandle,
    pub tx_address: u64,
}
