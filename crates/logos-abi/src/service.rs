use crate as logos_abi;
use core::mem::align_of;

pub const MAGIC: [u8; 4] = *b"LGSV";
pub const ABI: u16 = 3;
pub const MAX_TEXT: usize = 256;
pub const READY: u32 = 1;
pub const READ_INPUT: u32 = 2;
pub const PRESENT_PIXEL: u32 = 3;
pub const PRESENT_TEXT: u32 = 4;
pub const CLEAR_DISPLAY: u32 = 5;
pub const COMPLETE: u32 = 6;
pub const SYSCALL: u32 = 7;
pub const SESSION_REPLY: u32 = 8;
pub const SESSION_EFFECT: u32 = 9;
pub const STORE_REQUEST: u32 = 10;
pub const STORE_REPLY: u32 = 11;
pub const BLOCK_REQUEST: u32 = 12;
pub const BLOCK_REPLY: u32 = 13;
pub const NETWORK_REQUEST: u32 = 14;
pub const NETWORK_REPLY: u32 = 15;
pub const NETWORK_WAIT: u32 = 16;
pub const NETWORK_EVENT: u32 = 17;
pub const NETWORK_DEVICE_REQUEST: u32 = 18;
pub const NETWORK_DEVICE_REPLY: u32 = 19;
pub const PANIC: u32 = 20;
pub const ACKNOWLEDGED: u32 = 1;
pub const STORAGE_FORMATTED: u32 = 1;
pub const STORAGE_RECOVERED: u32 = 2;
pub const STORAGE_RECOVERED_INCOMPLETE: u32 = 3;
pub const STORAGE_CORRUPT: u32 = 4;
pub const STORAGE_IO_FAILED: u32 = 5;
pub const STORAGE_UNAVAILABLE: u32 = 6;

#[derive(Clone, Copy)]
#[repr(C)]
pub struct Context {
    pub abi: u16,
    pub reserved: u16,
    pub operation: u32,
    pub status: u32,
    pub input: u32,
    pub x: u32,
    pub y: u32,
    pub color: u32,
    pub text_length: u32,
    pub text: [u8; MAX_TEXT],
    pub shared_page: u32,
    pub network_rx_page: u32,
    pub network_tx_page: u32,
}

#[derive(Clone, Copy)]
pub struct TextRequest {
    pub x: u32,
    pub y: u32,
    pub color: logos_abi::DisplayColor,
    pub text: [u8; MAX_TEXT],
    pub length: usize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct BlockPage {
    pub handle: logos_abi::PageHandle,
    pub address: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct NetworkPages {
    pub rx_handle: logos_abi::PageHandle,
    pub rx_address: u64,
    pub tx_handle: logos_abi::PageHandle,
    pub tx_address: u64,
}

const BLOCK_REQUEST_BYTES: usize = 32;
const STORE_REQUEST_BYTES: usize = 102;
const BLOCK_REPLY_BYTES: usize = 21;
const STORE_REPLY_BYTES: usize = 17;
const NETWORK_REQUEST_BYTES: usize = 34;
const NETWORK_REPLY_BYTES: usize = 148;
const NETWORK_DEVICE_REQUEST_BYTES: usize = 18;
const NETWORK_DEVICE_REPLY_BYTES: usize = 34;
const NETWORK_EVENT_BYTES: usize = 18;

fn read_u32(bytes: &[u8], offset: usize) -> Option<u32> {
    Some(u32::from_le_bytes(bytes.get(offset..offset + 4)?.try_into().ok()?))
}

fn read_u64(bytes: &[u8], offset: usize) -> Option<u64> {
    Some(u64::from_le_bytes(bytes.get(offset..offset + 8)?.try_into().ok()?))
}

fn write_u32(bytes: &mut [u8], offset: usize, value: u32) {
    bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
}
fn write_u16(bytes: &mut [u8], offset: usize, value: u16) {
    bytes[offset..offset + 2].copy_from_slice(&value.to_le_bytes());
}
fn write_u64(bytes: &mut [u8], offset: usize, value: u64) {
    bytes[offset..offset + 8].copy_from_slice(&value.to_le_bytes());
}

fn read_u16(bytes: &[u8], offset: usize) -> Option<u16> {
    Some(u16::from_le_bytes(bytes.get(offset..offset + 2)?.try_into().ok()?))
}

fn decode_block_request(bytes: &[u8]) -> Option<logos_abi::BlockRequest> {
    Some(logos_abi::BlockRequest {
        id: read_u32(bytes, 0)?,
        operation: logos_abi::BlockOperation::from_wire(*bytes.get(4)?)?,
        lba: read_u64(bytes, 8)?,
        blocks: read_u32(bytes, 16)?,
        page: logos_abi::PageHandle(read_u32(bytes, 20)?),
        deadline: read_u64(bytes, 24)?,
    })
}

fn encode_block_request(bytes: &mut [u8; MAX_TEXT], request: logos_abi::BlockRequest) {
    *bytes = [0; MAX_TEXT];
    write_u32(bytes, 0, request.id);
    bytes[4] = request.operation as u8;
    write_u64(bytes, 8, request.lba);
    write_u32(bytes, 16, request.blocks);
    write_u32(bytes, 20, request.page.0);
    write_u64(bytes, 24, request.deadline);
}

fn decode_store_request(bytes: &[u8]) -> Option<logos_abi::StoreRequest> {
    let mut name = [0; logos_abi::MAX_OBJECT_NAME];
    name.copy_from_slice(bytes.get(14..14 + logos_abi::MAX_OBJECT_NAME)?);
    Some(logos_abi::StoreRequest {
        id: read_u32(bytes, 0)?,
        operation: logos_abi::StoreOperation::from_wire(*bytes.get(4)?)?,
        namespace: logos_abi::NamespaceId(read_u32(bytes, 8)?),
        name,
        name_length: *bytes.get(12)?,
        version: logos_abi::VersionSelector::from_wire(*bytes.get(13)?)?,
        offset: read_u64(bytes, 78)?,
        length: read_u32(bytes, 86)?,
        page: logos_abi::PageHandle(read_u32(bytes, 90)?),
        deadline: read_u64(bytes, 94)?,
    })
}

fn encode_store_request(bytes: &mut [u8; MAX_TEXT], request: logos_abi::StoreRequest) {
    *bytes = [0; MAX_TEXT];
    write_u32(bytes, 0, request.id);
    bytes[4] = request.operation as u8;
    write_u32(bytes, 8, request.namespace.0);
    bytes[12] = request.name_length;
    bytes[13] = request.version as u8;
    bytes[14..14 + logos_abi::MAX_OBJECT_NAME].copy_from_slice(&request.name);
    write_u64(bytes, 78, request.offset);
    write_u32(bytes, 86, request.length);
    write_u32(bytes, 90, request.page.0);
    write_u64(bytes, 94, request.deadline);
}

fn decode_block_reply(bytes: &[u8]) -> Option<logos_abi::BlockReply> {
    Some(logos_abi::BlockReply {
        id: read_u32(bytes, 0)?,
        status: logos_abi::PersistenceStatus::from_wire(*bytes.get(4)?)?,
        info: logos_abi::BlockInfo {
            logical_block_size: read_u32(bytes, 5)?,
            blocks: read_u64(bytes, 9)?,
            max_transfer_blocks: read_u32(bytes, 17)?,
        },
    })
}

fn encode_block_reply(bytes: &mut [u8; MAX_TEXT], reply: logos_abi::BlockReply) {
    *bytes = [0; MAX_TEXT];
    write_u32(bytes, 0, reply.id);
    bytes[4] = reply.status as u8;
    write_u32(bytes, 5, reply.info.logical_block_size);
    write_u64(bytes, 9, reply.info.blocks);
    write_u32(bytes, 17, reply.info.max_transfer_blocks);
}

fn decode_store_reply(bytes: &[u8]) -> Option<logos_abi::StoreReply> {
    Some(logos_abi::StoreReply {
        id: read_u32(bytes, 0)?,
        status: logos_abi::PersistenceStatus::from_wire(*bytes.get(4)?)?,
        version: read_u64(bytes, 5)?,
        length: read_u32(bytes, 13)?,
    })
}

fn encode_store_reply(bytes: &mut [u8; MAX_TEXT], reply: logos_abi::StoreReply) {
    *bytes = [0; MAX_TEXT];
    write_u32(bytes, 0, reply.id);
    bytes[4] = reply.status as u8;
    write_u64(bytes, 5, reply.version);
    write_u32(bytes, 13, reply.length);
}

fn decode_network_request(bytes: &[u8]) -> Option<logos_abi::NetworkRequest> {
    if *bytes.get(5)? != 0 {
        return None;
    }
    Some(logos_abi::NetworkRequest {
        id: read_u32(bytes, 0)?,
        operation: logos_abi::NetworkOperation::from_wire(*bytes.get(4)?)?,
        endpoint: logos_abi::NetworkEndpoint(read_u32(bytes, 6)?),
        peer: logos_abi::NetworkScope(read_u64(bytes, 10)?),
        page: logos_abi::PageHandle(read_u32(bytes, 18)?),
        length: read_u16(bytes, 22)?,
        generation: read_u16(bytes, 24)?,
        deadline: read_u64(bytes, 26)?,
    })
}

fn encode_network_request(bytes: &mut [u8; MAX_TEXT], request: logos_abi::NetworkRequest) {
    *bytes = [0; MAX_TEXT];
    write_u32(bytes, 0, request.id);
    bytes[4] = request.operation as u8;
    write_u32(bytes, 6, request.endpoint.0);
    write_u64(bytes, 10, request.peer.0);
    write_u32(bytes, 18, request.page.0);
    write_u16(bytes, 22, request.length);
    write_u16(bytes, 24, request.generation);
    write_u64(bytes, 26, request.deadline);
}

fn decode_network_reply(bytes: &[u8]) -> Option<logos_abi::NetworkReply> {
    if *bytes.get(5)? != 0 {
        return None;
    }
    Some(logos_abi::NetworkReply {
        id: read_u32(bytes, 0)?,
        status: logos_abi::NetworkStatus::from_wire(*bytes.get(4)?)?,
        endpoint: logos_abi::NetworkEndpoint(read_u32(bytes, 6)?),
        generation: read_u16(bytes, 10)?,
        source_address: read_u32(bytes, 12)?,
        source_port: read_u16(bytes, 16)?,
        length: read_u16(bytes, 18)?,
        info: decode_network_info(bytes, 20)?,
        counters: decode_network_counters(bytes, 44)?,
    })
}

fn encode_network_reply(bytes: &mut [u8; MAX_TEXT], reply: logos_abi::NetworkReply) {
    *bytes = [0; MAX_TEXT];
    write_u32(bytes, 0, reply.id);
    bytes[4] = reply.status as u8;
    write_u32(bytes, 6, reply.endpoint.0);
    write_u16(bytes, 10, reply.generation);
    write_u32(bytes, 12, reply.source_address);
    write_u16(bytes, 16, reply.source_port);
    write_u16(bytes, 18, reply.length);
    encode_network_info(bytes, 20, reply.info);
    encode_network_counters(bytes, 44, reply.counters);
}

fn decode_network_device_request(bytes: &[u8]) -> Option<logos_abi::NetworkDeviceRequest> {
    if *bytes.get(5)? != 0 {
        return None;
    }
    Some(logos_abi::NetworkDeviceRequest {
        id: read_u32(bytes, 0)?,
        operation: logos_abi::NetworkDeviceOperation::from_wire(*bytes.get(4)?)?,
        length: read_u16(bytes, 6)?,
        generation: read_u16(bytes, 8)?,
        deadline: read_u64(bytes, 10)?,
    })
}

fn encode_network_device_request(
    bytes: &mut [u8; MAX_TEXT],
    request: logos_abi::NetworkDeviceRequest,
) {
    *bytes = [0; MAX_TEXT];
    write_u32(bytes, 0, request.id);
    bytes[4] = request.operation as u8;
    write_u16(bytes, 6, request.length);
    write_u16(bytes, 8, request.generation);
    write_u64(bytes, 10, request.deadline);
}

fn decode_network_device_reply(bytes: &[u8]) -> Option<logos_abi::NetworkDeviceReply> {
    if *bytes.get(5)? != 0 {
        return None;
    }
    Some(logos_abi::NetworkDeviceReply {
        id: read_u32(bytes, 0)?,
        status: logos_abi::NetworkStatus::from_wire(*bytes.get(4)?)?,
        generation: read_u16(bytes, 6)?,
        info: decode_network_info(bytes, 8)?,
    })
}

fn encode_network_device_reply(bytes: &mut [u8; MAX_TEXT], reply: logos_abi::NetworkDeviceReply) {
    *bytes = [0; MAX_TEXT];
    write_u32(bytes, 0, reply.id);
    bytes[4] = reply.status as u8;
    write_u16(bytes, 6, reply.generation);
    encode_network_info(bytes, 8, reply.info);
}

fn decode_network_info(bytes: &[u8], offset: usize) -> Option<logos_abi::NetworkInfo> {
    let mut mac = [0; 6];
    mac.copy_from_slice(bytes.get(offset..offset + 6)?);
    Some(logos_abi::NetworkInfo {
        mac,
        mtu: read_u16(bytes, offset + 6)?,
        generation: read_u16(bytes, offset + 8)?,
        link_up: *bytes.get(offset + 10)?,
        configuration: *bytes.get(offset + 11)?,
        ipv4: read_u32(bytes, offset + 12)?,
        subnet_mask: read_u32(bytes, offset + 16)?,
        router: read_u32(bytes, offset + 20)?,
    })
}

fn encode_network_info(bytes: &mut [u8; MAX_TEXT], offset: usize, info: logos_abi::NetworkInfo) {
    bytes[offset..offset + 6].copy_from_slice(&info.mac);
    write_u16(bytes, offset + 6, info.mtu);
    write_u16(bytes, offset + 8, info.generation);
    bytes[offset + 10] = info.link_up;
    bytes[offset + 11] = info.configuration;
    write_u32(bytes, offset + 12, info.ipv4);
    write_u32(bytes, offset + 16, info.subnet_mask);
    write_u32(bytes, offset + 20, info.router);
}

fn decode_network_counters(bytes: &[u8], offset: usize) -> Option<logos_abi::NetworkCounters> {
    Some(logos_abi::NetworkCounters {
        rx_frames: read_u64(bytes, offset)?,
        tx_frames: read_u64(bytes, offset + 8)?,
        rx_bytes: read_u64(bytes, offset + 16)?,
        tx_bytes: read_u64(bytes, offset + 24)?,
        malformed: read_u64(bytes, offset + 32)?,
        unsupported: read_u64(bytes, offset + 40)?,
        rx_dropped: read_u64(bytes, offset + 48)?,
        udp_no_endpoint: read_u64(bytes, offset + 56)?,
        udp_queue_dropped: read_u64(bytes, offset + 64)?,
        timeouts: read_u64(bytes, offset + 72)?,
        cancellations: read_u64(bytes, offset + 80)?,
        resets: read_u64(bytes, offset + 88)?,
        denied: read_u64(bytes, offset + 96)?,
    })
}

fn encode_network_counters(
    bytes: &mut [u8; MAX_TEXT],
    offset: usize,
    counters: logos_abi::NetworkCounters,
) {
    for (index, value) in [
        counters.rx_frames,
        counters.tx_frames,
        counters.rx_bytes,
        counters.tx_bytes,
        counters.malformed,
        counters.unsupported,
        counters.rx_dropped,
        counters.udp_no_endpoint,
        counters.udp_queue_dropped,
        counters.timeouts,
        counters.cancellations,
        counters.resets,
        counters.denied,
    ]
    .into_iter()
    .enumerate()
    {
        write_u64(bytes, offset + index * 8, value);
    }
}

fn decode_network_event(bytes: &[u8]) -> Option<logos_abi::NetworkEvent> {
    if *bytes.get(5)? != 0 {
        return None;
    }
    Some(logos_abi::NetworkEvent {
        id: read_u32(bytes, 0)?,
        kind: logos_abi::NetworkEventKind::from_wire(*bytes.get(4)?)?,
        generation: read_u16(bytes, 6)?,
        length: read_u16(bytes, 8)?,
        now: read_u64(bytes, 10)?,
    })
}

fn encode_network_event(bytes: &mut [u8; MAX_TEXT], event: logos_abi::NetworkEvent) {
    *bytes = [0; MAX_TEXT];
    write_u32(bytes, 0, event.id);
    bytes[4] = event.kind as u8;
    write_u16(bytes, 6, event.generation);
    write_u16(bytes, 8, event.length);
    write_u64(bytes, 10, event.now);
}

impl Context {
    pub const fn new() -> Self {
        Self {
            abi: ABI,
            reserved: 0,
            operation: 0,
            status: 0,
            input: 0,
            x: 0,
            y: 0,
            color: 0,
            text_length: 0,
            text: [0; MAX_TEXT],
            shared_page: 0,
            network_rx_page: 0,
            network_tx_page: 0,
        }
    }

    /// # Safety
    /// `address` must point to a live, aligned `Context` mapping.
    pub unsafe fn panicked_at(address: u64) -> bool {
        let Some(context) = (unsafe { (address as *const Self).as_ref() }) else { return false };
        context.abi == ABI && context.reserved == 0 && context.operation == PANIC
    }

    /// # Safety
    /// `address` must point to a live, aligned `Context` mapping.
    pub unsafe fn reset_at(address: u64) -> bool {
        if address == 0 || !address.is_multiple_of(align_of::<Self>() as u64) {
            return false;
        }
        unsafe { (address as *mut Self).write_volatile(Self::new()) };
        true
    }

    /// # Safety
    ///
    /// `address` must point to a live, aligned `Context` mapping.
    pub unsafe fn ready_at(address: u64) -> bool {
        let context = unsafe { (address as *const Self).read_volatile() };
        context.abi == ABI && context.reserved == 0 && context.operation == READY
    }

    /// # Safety
    ///
    /// `address` must point to a live, aligned `Context` mapping.
    pub unsafe fn acknowledge_at(address: u64) -> bool {
        let context = unsafe { (address as *mut Self).read_volatile() };
        if context.abi != ABI
            || context.reserved != 0
            || context.operation != READY
            || context.status != 0
        {
            return false;
        }
        unsafe { (address as *mut Self).cast::<u32>().add(2).write_volatile(ACKNOWLEDGED) };
        true
    }

    /// # Safety
    ///
    /// `address` must point to a live, aligned `Context` mapping.
    pub unsafe fn complete_at(address: u64) -> bool {
        let context = unsafe { (address as *const Self).read_volatile() };
        context.abi == ABI
            && context.reserved == 0
            && context.operation == COMPLETE
            && context.status == ACKNOWLEDGED
    }

    /// # Safety
    ///
    /// `address` must point to a live, aligned `Context` mapping.
    pub unsafe fn input_waiting_at(address: u64) -> bool {
        let context = unsafe { (address as *const Self).read_volatile() };
        context.abi == ABI
            && context.reserved == 0
            && context.operation == READ_INPUT
            && context.status == ACKNOWLEDGED
    }

    /// # Safety
    /// `address` must point to a live, aligned `Context` mapping.
    pub unsafe fn storage_status_at(address: u64) -> Option<u32> {
        let context = unsafe { (address as *const Self).read_volatile() };
        (context.abi == ABI
            && context.reserved == 0
            && (STORAGE_FORMATTED..=STORAGE_IO_FAILED).contains(&context.x))
        .then_some(context.x)
    }

    /// # Safety
    ///
    /// `address` must point to a live, aligned `Context` mapping.
    pub unsafe fn deliver_input_at(address: u64, input: u8) -> bool {
        let mut context = unsafe { (address as *mut Self).read_volatile() };
        if context.abi != ABI
            || context.reserved != 0
            || context.operation != READ_INPUT
            || context.status != ACKNOWLEDGED
        {
            return false;
        }
        context.input = u32::from(input);
        unsafe { (address as *mut Self).write_volatile(context) };
        true
    }

    /// # Safety
    /// `address` must point to a live, aligned `Context` mapping.
    pub unsafe fn deliver_session_at(address: u64, request: logos_abi::SessionRequest) -> bool {
        if !request.valid() {
            return false;
        }
        let mut context = unsafe { (address as *mut Self).read_volatile() };
        if context.abi != ABI
            || context.reserved != 0
            || context.operation != READ_INPUT
            || context.status != ACKNOWLEDGED
        {
            return false;
        }
        context.input = 1;
        context.x = request.syscall as u32;
        context.text = request.argument;
        context.text_length = request.length as u32;
        context.operation = SYSCALL;
        unsafe { (address as *mut Self).write_volatile(context) };
        true
    }

    /// # Safety
    /// `address` must point to a live, aligned `Context` mapping.
    pub unsafe fn syscall_at(address: u64) -> Option<logos_abi::SessionRequest> {
        let context = unsafe { (address as *const Self).read_volatile() };
        let length = usize::try_from(context.text_length).ok()?;
        let syscall = logos_abi::Syscall::from_wire(context.x)?;
        let request = logos_abi::SessionRequest::new(syscall, context.text, length);
        (context.abi == ABI
            && context.reserved == 0
            && context.operation == SYSCALL
            && context.status == ACKNOWLEDGED
            && request.valid())
        .then_some(request)
    }

    /// # Safety
    /// `address` must point to a live, aligned `Context` mapping.
    pub unsafe fn reply_at(address: u64, reply: &[u8]) -> bool {
        if reply.len() > MAX_TEXT || unsafe { Self::syscall_at(address) }.is_none() {
            return false;
        }
        let mut context = unsafe { (address as *mut Self).read_volatile() };
        context.text = [0; MAX_TEXT];
        context.text[..reply.len()].copy_from_slice(reply);
        context.text_length = reply.len() as u32;
        unsafe { (address as *mut Self).write_volatile(context) };
        true
    }

    /// # Safety
    /// `address` must point to a live, aligned `Context` mapping.
    pub unsafe fn response_at(address: u64) -> Option<TextRequest> {
        let context = unsafe { (address as *const Self).read_volatile() };
        let length = usize::try_from(context.text_length).ok()?;
        (context.abi == ABI
            && context.reserved == 0
            && context.operation == SYSCALL
            && context.status == ACKNOWLEDGED
            && length <= context.text.len())
        .then_some(TextRequest {
            x: 0,
            y: 0,
            color: logos_abi::DisplayColor::BLACK,
            text: context.text,
            length,
        })
    }

    /// # Safety
    /// `address` must point to a live, aligned `Context` mapping.
    pub unsafe fn session_reply_at(address: u64) -> Option<logos_abi::SessionReply> {
        let context = unsafe { (address as *const Self).read_volatile() };
        let length = usize::try_from(context.text_length).ok()?;
        (context.abi == ABI
            && context.reserved == 0
            && context.operation == SESSION_REPLY
            && context.status == ACKNOWLEDGED
            && length <= context.text.len())
        .then_some(logos_abi::SessionReply { text: context.text, length })
    }

    /// # Safety
    /// `address` must point to a live, aligned `Context` mapping.
    pub unsafe fn session_effect_at(address: u64) -> Option<logos_abi::EffectRequest> {
        let context = unsafe { (address as *const Self).read_volatile() };
        let length = usize::try_from(context.text_length).ok()?;
        let effect = logos_abi::Effect::from_wire(context.x)?;
        let request = logos_abi::EffectRequest::new(effect, context.text, length);
        (context.abi == ABI
            && context.reserved == 0
            && context.operation == SESSION_EFFECT
            && context.status == ACKNOWLEDGED
            && request.valid())
        .then_some(request)
    }

    /// # Safety
    /// `address` must point to a live, aligned `Context` mapping.
    pub unsafe fn store_at(address: u64) -> Option<logos_abi::StoreRequest> {
        let context = unsafe { (address as *const Self).read_volatile() };
        if context.abi != ABI
            || context.reserved != 0
            || context.operation != STORE_REQUEST
            || context.status != ACKNOWLEDGED
            || context.text_length != STORE_REQUEST_BYTES as u32
        {
            return None;
        }
        let request = decode_store_request(&context.text)?;
        request.valid().then_some(request)
    }

    /// # Safety
    /// `address` must point to a live, aligned `Context` mapping owned by the caller.
    pub unsafe fn request_store_at(address: u64, request: logos_abi::StoreRequest) -> bool {
        if !request.valid() {
            return false;
        }
        let mut context = unsafe { (address as *mut Self).read_volatile() };
        if context.abi != ABI
            || context.reserved != 0
            || context.status != ACKNOWLEDGED
            || !matches!(context.operation, READY | READ_INPUT | STORE_REPLY)
        {
            return false;
        }
        encode_store_request(&mut context.text, request);
        context.text_length = STORE_REQUEST_BYTES as u32;
        context.operation = STORE_REQUEST;
        unsafe { (address as *mut Self).write_volatile(context) };
        true
    }

    /// # Safety
    /// `address` must point to a live, aligned `Context` mapping.
    pub unsafe fn deliver_store_at(address: u64, request: logos_abi::StoreRequest) -> bool {
        if !request.valid() {
            return false;
        }
        let mut context = unsafe { (address as *mut Self).read_volatile() };
        if context.abi != ABI
            || context.reserved != 0
            || context.operation != READ_INPUT
            || context.status != ACKNOWLEDGED
        {
            return false;
        }
        context.text = [0; MAX_TEXT];
        encode_store_request(&mut context.text, request);
        context.text_length = STORE_REQUEST_BYTES as u32;
        context.operation = STORE_REQUEST;
        unsafe { (address as *mut Self).write_volatile(context) };
        true
    }

    /// # Safety
    /// `address` must point to a live, aligned `Context` mapping.
    pub unsafe fn reply_store_at(address: u64, reply: logos_abi::StoreReply) -> bool {
        let mut context = unsafe { (address as *mut Self).read_volatile() };
        let valid = match context.operation {
            STORE_REQUEST => {
                decode_store_request(&context.text).is_some_and(|request| reply.valid_for(request))
            }
            BLOCK_REPLY => {
                reply.length as usize <= logos_abi::PAGE_SIZE && context.color == reply.id
            }
            _ => false,
        };
        if !valid {
            return false;
        }
        context.operation = STORE_REPLY;
        context.color = 0;
        encode_store_reply(&mut context.text, reply);
        context.text_length = STORE_REPLY_BYTES as u32;
        unsafe { (address as *mut Self).write_volatile(context) };
        true
    }

    /// # Safety
    /// `address` must point to a live, aligned `Context` mapping.
    pub unsafe fn store_reply_at(address: u64, expected_id: u32) -> Option<logos_abi::StoreReply> {
        let reply = unsafe { Self::store_reply_pending_at(address) }?;
        (reply.id == expected_id).then_some(reply)
    }

    /// # Safety
    /// `address` must point to a live, aligned `Context` mapping.
    pub unsafe fn store_reply_pending_at(address: u64) -> Option<logos_abi::StoreReply> {
        let context = unsafe { (address as *const Self).read_volatile() };
        if context.abi != ABI
            || context.reserved != 0
            || context.operation != STORE_REPLY
            || context.status != ACKNOWLEDGED
            || context.text_length != STORE_REPLY_BYTES as u32
        {
            return None;
        }
        decode_store_reply(&context.text)
            .filter(|reply| reply.length as usize <= logos_abi::PAGE_SIZE)
    }

    /// # Safety
    /// `address` must point to a live, aligned `Context` mapping.
    pub unsafe fn network_at(address: u64) -> Option<logos_abi::NetworkRequest> {
        let context = unsafe { (address as *const Self).read_volatile() };
        if context.abi != ABI
            || context.reserved != 0
            || context.operation != NETWORK_REQUEST
            || context.status != ACKNOWLEDGED
            || context.text_length != NETWORK_REQUEST_BYTES as u32
        {
            return None;
        }
        let request = decode_network_request(&context.text)?;
        request.valid_shape().then_some(request)
    }

    /// # Safety
    /// `address` must point to a live, aligned `Context` mapping owned by the caller.
    pub unsafe fn request_network_at(address: u64, request: logos_abi::NetworkRequest) -> bool {
        if !request.valid_shape() {
            return false;
        }
        let mut context = unsafe { (address as *mut Self).read_volatile() };
        if context.abi != ABI
            || context.reserved != 0
            || context.status != ACKNOWLEDGED
            || !matches!(
                context.operation,
                READY
                    | READ_INPUT
                    | NETWORK_REPLY
                    | NETWORK_EVENT
                    | NETWORK_DEVICE_REPLY
                    | NETWORK_WAIT
                    | NETWORK_REQUEST
            )
        {
            return false;
        }
        encode_network_request(&mut context.text, request);
        context.text_length = NETWORK_REQUEST_BYTES as u32;
        context.operation = NETWORK_REQUEST;
        unsafe { (address as *mut Self).write_volatile(context) };
        true
    }

    /// # Safety
    /// `address` must point to a live, aligned `Context` mapping owned by Core.
    pub unsafe fn deliver_network_at(address: u64, request: logos_abi::NetworkRequest) -> bool {
        if !request.valid_shape() {
            return false;
        }
        let mut context = unsafe { (address as *mut Self).read_volatile() };
        if context.abi != ABI
            || context.reserved != 0
            || !matches!(context.operation, READ_INPUT | NETWORK_WAIT)
            || context.status != ACKNOWLEDGED
        {
            return false;
        }
        encode_network_request(&mut context.text, request);
        context.text_length = NETWORK_REQUEST_BYTES as u32;
        context.operation = NETWORK_REQUEST;
        unsafe { (address as *mut Self).write_volatile(context) };
        true
    }

    /// # Safety
    /// `address` must point to a live, aligned `Context` mapping owned by Core.
    pub unsafe fn reply_network_at(address: u64, reply: logos_abi::NetworkReply) -> bool {
        let mut context = unsafe { (address as *mut Self).read_volatile() };
        let valid = context.operation == NETWORK_REQUEST
            && decode_network_request(&context.text)
                .is_some_and(|request| reply.valid_for(request));
        if !valid {
            return false;
        }
        encode_network_reply(&mut context.text, reply);
        context.text_length = NETWORK_REPLY_BYTES as u32;
        context.operation = NETWORK_REPLY;
        unsafe { (address as *mut Self).write_volatile(context) };
        true
    }

    /// # Safety
    /// `address` must point to a live, aligned `Context` mapping owned by the service.
    pub unsafe fn reply_network_after_device_at(
        address: u64,
        request: logos_abi::NetworkRequest,
        reply: logos_abi::NetworkReply,
    ) -> bool {
        let mut context = unsafe { (address as *mut Self).read_volatile() };
        if context.operation != NETWORK_DEVICE_REPLY || !reply.valid_for(request) {
            return false;
        }
        encode_network_reply(&mut context.text, reply);
        context.text_length = NETWORK_REPLY_BYTES as u32;
        context.operation = NETWORK_REPLY;
        unsafe { (address as *mut Self).write_volatile(context) };
        true
    }

    /// # Safety
    /// `address` must point to a live, aligned `Context` mapping owned by the service.
    pub unsafe fn reply_network_after_event_at(
        address: u64,
        request: logos_abi::NetworkRequest,
        reply: logos_abi::NetworkReply,
    ) -> bool {
        let mut context = unsafe { (address as *mut Self).read_volatile() };
        if context.operation != NETWORK_EVENT || !reply.valid_for(request) {
            return false;
        }
        encode_network_reply(&mut context.text, reply);
        context.text_length = NETWORK_REPLY_BYTES as u32;
        context.operation = NETWORK_REPLY;
        unsafe { (address as *mut Self).write_volatile(context) };
        true
    }

    /// # Safety
    /// `address` must point to a live, aligned `Context` mapping.
    pub unsafe fn network_reply_at(
        address: u64,
        expected_id: u32,
    ) -> Option<logos_abi::NetworkReply> {
        let context = unsafe { (address as *const Self).read_volatile() };
        if context.abi != ABI
            || context.reserved != 0
            || context.operation != NETWORK_REPLY
            || context.status != ACKNOWLEDGED
            || context.text_length != NETWORK_REPLY_BYTES as u32
        {
            return None;
        }
        decode_network_reply(&context.text).filter(|reply| reply.id == expected_id)
    }

    /// # Safety
    /// `address` must point to a live, aligned `Context` mapping.
    pub unsafe fn network_reply_pending_at(address: u64) -> bool {
        let context = unsafe { (address as *const Self).read_volatile() };
        context.abi == ABI
            && context.reserved == 0
            && context.operation == NETWORK_REPLY
            && context.status == ACKNOWLEDGED
            && context.text_length == NETWORK_REPLY_BYTES as u32
    }

    /// # Safety
    /// `address` must point to a live, aligned `Context` mapping owned by the caller.
    pub unsafe fn network_device_at(address: u64) -> Option<logos_abi::NetworkDeviceRequest> {
        let context = unsafe { (address as *const Self).read_volatile() };
        if context.abi != ABI
            || context.reserved != 0
            || context.operation != NETWORK_DEVICE_REQUEST
            || context.status != ACKNOWLEDGED
            || context.text_length != NETWORK_DEVICE_REQUEST_BYTES as u32
        {
            return None;
        }
        let request = decode_network_device_request(&context.text)?;
        request.valid_shape().then_some(request)
    }

    /// # Safety
    /// `address` must point to a live, aligned `Context` mapping.
    pub unsafe fn network_device_pending_at(address: u64) -> bool {
        let context = unsafe { (address as *const Self).read_volatile() };
        context.abi == ABI
            && context.reserved == 0
            && context.operation == NETWORK_DEVICE_REQUEST
            && context.status == ACKNOWLEDGED
            && context.text_length == NETWORK_DEVICE_REQUEST_BYTES as u32
    }

    /// # Safety
    /// `address` must point to a live, aligned `Context` mapping owned by the caller.
    pub unsafe fn request_network_device_at(
        address: u64,
        request: logos_abi::NetworkDeviceRequest,
    ) -> bool {
        if !request.valid_shape() {
            return false;
        }
        let mut context = unsafe { (address as *mut Self).read_volatile() };
        if context.abi != ABI
            || context.reserved != 0
            || context.status != ACKNOWLEDGED
            || !matches!(
                context.operation,
                READY
                    | READ_INPUT
                    | NETWORK_REQUEST
                    | NETWORK_REPLY
                    | NETWORK_EVENT
                    | NETWORK_DEVICE_REPLY
            )
        {
            return false;
        }
        encode_network_device_request(&mut context.text, request);
        context.text_length = NETWORK_DEVICE_REQUEST_BYTES as u32;
        context.operation = NETWORK_DEVICE_REQUEST;
        unsafe { (address as *mut Self).write_volatile(context) };
        true
    }

    /// # Safety
    /// `address` must point to a live, aligned `Context` mapping owned by Core.
    pub unsafe fn reply_network_device_at(
        address: u64,
        reply: logos_abi::NetworkDeviceReply,
    ) -> bool {
        let mut context = unsafe { (address as *mut Self).read_volatile() };
        let valid = context.operation == NETWORK_DEVICE_REQUEST
            && decode_network_device_request(&context.text)
                .is_some_and(|request| reply.valid_for(request));
        if !valid {
            return false;
        }
        encode_network_device_reply(&mut context.text, reply);
        context.text_length = NETWORK_DEVICE_REPLY_BYTES as u32;
        context.operation = NETWORK_DEVICE_REPLY;
        unsafe { (address as *mut Self).write_volatile(context) };
        true
    }

    /// # Safety
    /// `address` must point to a live, aligned `Context` mapping.
    pub unsafe fn network_device_reply_at(
        address: u64,
        expected_id: u32,
    ) -> Option<logos_abi::NetworkDeviceReply> {
        let context = unsafe { (address as *const Self).read_volatile() };
        if context.abi != ABI
            || context.reserved != 0
            || context.operation != NETWORK_DEVICE_REPLY
            || context.status != ACKNOWLEDGED
            || context.text_length != NETWORK_DEVICE_REPLY_BYTES as u32
        {
            return None;
        }
        decode_network_device_reply(&context.text).filter(|reply| reply.id == expected_id)
    }

    /// # Safety
    /// `address` must point to a live, aligned `Context` mapping owned by Core.
    pub unsafe fn deliver_network_device_reply_at(
        address: u64,
        reply: logos_abi::NetworkDeviceReply,
    ) -> bool {
        let mut context = unsafe { (address as *mut Self).read_volatile() };
        if context.abi != ABI
            || context.reserved != 0
            || context.operation != NETWORK_WAIT
            || context.status != ACKNOWLEDGED
        {
            return false;
        }
        encode_network_device_reply(&mut context.text, reply);
        context.text_length = NETWORK_DEVICE_REPLY_BYTES as u32;
        context.operation = NETWORK_DEVICE_REPLY;
        unsafe { (address as *mut Self).write_volatile(context) };
        true
    }

    /// # Safety
    /// `address` must point to a live, aligned `Context` mapping owned by the caller.
    pub unsafe fn network_wait_at(address: u64, deadline: u64) -> bool {
        if deadline == 0 {
            return false;
        }
        let mut context = unsafe { (address as *mut Self).read_volatile() };
        if context.abi != ABI
            || context.reserved != 0
            || context.status != ACKNOWLEDGED
            || !matches!(
                context.operation,
                READY
                    | READ_INPUT
                    | NETWORK_REQUEST
                    | NETWORK_REPLY
                    | NETWORK_EVENT
                    | NETWORK_DEVICE_REQUEST
                    | NETWORK_DEVICE_REPLY
            )
        {
            return false;
        }
        context.x = deadline as u32;
        context.y = (deadline >> 32) as u32;
        context.text = [0; MAX_TEXT];
        context.text_length = 0;
        context.operation = NETWORK_WAIT;
        unsafe { (address as *mut Self).write_volatile(context) };
        true
    }

    /// # Safety
    /// `address` must point to a live, aligned `Context` mapping.
    pub unsafe fn network_waiting_at(address: u64) -> bool {
        let context = unsafe { (address as *const Self).read_volatile() };
        context.abi == ABI
            && context.reserved == 0
            && context.operation == NETWORK_WAIT
            && context.status == ACKNOWLEDGED
            && (u64::from(context.x) | (u64::from(context.y) << 32)) != 0
    }

    /// # Safety
    /// `address` must point to a live, aligned `Context` mapping.
    pub unsafe fn network_deadline_at(address: u64) -> Option<u64> {
        let context = unsafe { (address as *const Self).read_volatile() };
        (context.abi == ABI
            && context.reserved == 0
            && context.operation == NETWORK_WAIT
            && context.status == ACKNOWLEDGED)
            .then_some(u64::from(context.x) | (u64::from(context.y) << 32))
            .filter(|deadline| *deadline != 0)
    }

    /// # Safety
    /// `address` must point to a live, aligned `Context` mapping owned by Core.
    pub unsafe fn deliver_network_event_at(address: u64, event: logos_abi::NetworkEvent) -> bool {
        if !event.valid() {
            return false;
        }
        let mut context = unsafe { (address as *mut Self).read_volatile() };
        if context.abi != ABI
            || context.reserved != 0
            || context.operation != NETWORK_WAIT
            || context.status != ACKNOWLEDGED
        {
            return false;
        }
        encode_network_event(&mut context.text, event);
        context.text_length = NETWORK_EVENT_BYTES as u32;
        context.operation = NETWORK_EVENT;
        unsafe { (address as *mut Self).write_volatile(context) };
        true
    }

    /// # Safety
    /// `address` must point to a live, aligned `Context` mapping.
    pub unsafe fn network_event_at(address: u64) -> Option<logos_abi::NetworkEvent> {
        let context = unsafe { (address as *const Self).read_volatile() };
        if context.abi != ABI
            || context.reserved != 0
            || context.operation != NETWORK_EVENT
            || context.status != ACKNOWLEDGED
            || context.text_length != NETWORK_EVENT_BYTES as u32
        {
            return None;
        }
        let event = decode_network_event(&context.text)?;
        event.valid().then_some(event)
    }

    /// # Safety
    /// `address` must point to a live, aligned `Context` mapping.
    pub unsafe fn block_at(address: u64) -> Option<logos_abi::BlockRequest> {
        let context = unsafe { (address as *const Self).read_volatile() };
        if context.abi != ABI
            || context.reserved != 0
            || context.operation != BLOCK_REQUEST
            || context.status != ACKNOWLEDGED
            || context.text_length != BLOCK_REQUEST_BYTES as u32
        {
            return None;
        }
        let request = decode_block_request(&context.text)?;
        request.valid_shape().then_some(request)
    }

    /// # Safety
    /// `address` must point to a live, aligned `Context` mapping before service startup.
    pub unsafe fn configure_block_page_at(address: u64, page: BlockPage) -> bool {
        if page.handle.0 == 0
            || page.address == 0
            || !page.address.is_multiple_of(logos_abi::PAGE_SIZE as u64)
        {
            return false;
        }
        let mut context = unsafe { (address as *mut Self).read_volatile() };
        if context.abi != ABI
            || context.reserved != 0
            || context.operation != 0
            || context.status != 0
        {
            return false;
        }
        context.input = page.handle.0;
        context.x = page.address as u32;
        context.y = (page.address >> 32) as u32;
        unsafe { (address as *mut Self).write_volatile(context) };
        true
    }

    /// # Safety
    /// `address` must point to a live, aligned `Context` mapping before service startup.
    pub unsafe fn configure_shared_page_at(address: u64, page: logos_abi::PageHandle) -> bool {
        if page.0 == 0 {
            return false;
        }
        let mut context = unsafe { (address as *mut Self).read_volatile() };
        if context.abi != ABI
            || context.reserved != 0
            || context.operation != 0
            || context.status != 0
        {
            return false;
        }
        context.shared_page = page.0;
        unsafe { (address as *mut Self).write_volatile(context) };
        true
    }

    /// # Safety
    /// `address` must point to a live, aligned `Context` mapping owned by Core.
    pub unsafe fn remap_shared_page_at(address: u64, page: logos_abi::PageHandle) -> bool {
        if page.0 == 0 {
            return false;
        }
        let mut context = unsafe { (address as *mut Self).read_volatile() };
        if context.abi != ABI || context.reserved != 0 || context.shared_page == 0 {
            return false;
        }
        context.shared_page = page.0;
        unsafe { (address as *mut Self).write_volatile(context) };
        true
    }

    /// # Safety
    /// `address` must point to a live, aligned `Context` mapping before service startup.
    pub unsafe fn configure_network_pages_at(
        address: u64,
        rx: logos_abi::PageHandle,
        tx: logos_abi::PageHandle,
    ) -> bool {
        if rx.0 == 0 || tx.0 == 0 || rx == tx {
            return false;
        }
        let mut context = unsafe { (address as *mut Self).read_volatile() };
        if context.abi != ABI
            || context.reserved != 0
            || context.operation != 0
            || context.status != 0
        {
            return false;
        }
        context.network_rx_page = rx.0;
        context.network_tx_page = tx.0;
        unsafe { (address as *mut Self).write_volatile(context) };
        true
    }

    /// # Safety
    /// `address` must point to a live, aligned `Context` mapping.
    pub unsafe fn shared_page_at(address: u64) -> Option<logos_abi::PageHandle> {
        let context = unsafe { (address as *const Self).read_volatile() };
        (context.abi == ABI && context.reserved == 0 && context.shared_page != 0)
            .then_some(logos_abi::PageHandle(context.shared_page))
    }

    /// # Safety
    /// `address` must point to a live, aligned `Context` mapping.
    pub unsafe fn network_pages_at(address: u64) -> Option<NetworkPages> {
        let context = unsafe { (address as *const Self).read_volatile() };
        if context.abi != ABI
            || context.reserved != 0
            || context.network_rx_page == 0
            || context.network_tx_page == 0
        {
            return None;
        }
        let rx_address = address.checked_sub(logos_abi::PAGE_SIZE as u64 * 19)?;
        Some(NetworkPages {
            rx_handle: logos_abi::PageHandle(context.network_rx_page),
            rx_address,
            tx_handle: logos_abi::PageHandle(context.network_tx_page),
            tx_address: rx_address.checked_sub(logos_abi::PAGE_SIZE as u64)?,
        })
    }

    /// # Safety
    /// `address` must point to a live, aligned `Context` mapping.
    pub unsafe fn block_page_at(address: u64) -> Option<BlockPage> {
        let context = unsafe { (address as *const Self).read_volatile() };
        let page = BlockPage {
            handle: logos_abi::PageHandle(context.input),
            address: u64::from(context.x) | (u64::from(context.y) << 32),
        };
        (context.abi == ABI
            && context.reserved == 0
            && page.handle.0 != 0
            && page.address != 0
            && page.address.is_multiple_of(logos_abi::PAGE_SIZE as u64))
        .then_some(page)
    }

    /// # Safety
    /// `address` must point to a live, aligned `Context` mapping owned by the caller.
    pub unsafe fn request_block_at(address: u64, request: logos_abi::BlockRequest) -> bool {
        if !request.valid_shape() {
            return false;
        }
        let mut context = unsafe { (address as *mut Self).read_volatile() };
        if context.abi != ABI
            || context.reserved != 0
            || context.status != ACKNOWLEDGED
            || !matches!(context.operation, READY | READ_INPUT | STORE_REQUEST | BLOCK_REPLY)
        {
            return false;
        }
        let parent_store_id = if context.operation == STORE_REQUEST {
            let Some(parent) = decode_store_request(&context.text) else {
                return false;
            };
            Some(parent.id)
        } else {
            None
        };
        if let Some(id) = parent_store_id {
            // `color` is free while a Block request is active and preserves the
            // Store request ID across the nested Block round trip.
            context.color = id;
        } else if context.operation != BLOCK_REPLY {
            context.color = 0;
        }
        encode_block_request(&mut context.text, request);
        context.text_length = BLOCK_REQUEST_BYTES as u32;
        context.operation = BLOCK_REQUEST;
        unsafe { (address as *mut Self).write_volatile(context) };
        true
    }

    /// # Safety
    /// `address` must point to a live, aligned `Context` mapping.
    pub unsafe fn reply_block_at(address: u64, reply: logos_abi::BlockReply) -> bool {
        let Some(request) = (unsafe { Self::block_at(address) }) else {
            return false;
        };
        if !reply.valid_for(request) {
            return false;
        }
        let mut context = unsafe { (address as *mut Self).read_volatile() };
        context.operation = BLOCK_REPLY;
        encode_block_reply(&mut context.text, reply);
        context.text_length = BLOCK_REPLY_BYTES as u32;
        unsafe { (address as *mut Self).write_volatile(context) };
        true
    }

    /// # Safety
    /// `address` must point to a live, aligned `Context` mapping.
    pub unsafe fn block_reply_at(address: u64, expected_id: u32) -> Option<logos_abi::BlockReply> {
        let context = unsafe { (address as *const Self).read_volatile() };
        if context.abi != ABI
            || context.reserved != 0
            || context.operation != BLOCK_REPLY
            || context.status != ACKNOWLEDGED
            || context.text_length != BLOCK_REPLY_BYTES as u32
        {
            return None;
        }
        let reply = decode_block_reply(&context.text)?;
        (reply.id == expected_id).then_some(reply)
    }

    /// # Safety
    /// `address` must point to a live, aligned `Context` mapping.
    pub unsafe fn reply_effect_at(address: u64, reply: logos_abi::EffectResult) -> bool {
        unsafe { Self::reply_effect_with_text_at(address, logos_abi::EffectReply::new(reply, &[])) }
    }

    /// # Safety
    /// `address` must point to a live, aligned `Context` mapping.
    pub unsafe fn reply_effect_with_text_at(address: u64, reply: logos_abi::EffectReply) -> bool {
        if !reply.valid() {
            return false;
        }
        if unsafe { Self::session_effect_at(address) }.is_none() {
            return false;
        }
        let mut context = unsafe { (address as *mut Self).read_volatile() };
        context.x = reply.result as u32;
        context.text = [0; MAX_TEXT];
        context.text[..reply.length as usize].copy_from_slice(&reply.text[..reply.length as usize]);
        context.text_length = u32::from(reply.length);
        unsafe { (address as *mut Self).write_volatile(context) };
        true
    }

    /// # Safety
    ///
    /// `address` must point to a live, aligned `Context` mapping.
    pub unsafe fn pixel_at(address: u64) -> Option<(u32, u32, logos_abi::DisplayColor)> {
        let context = unsafe { (address as *const Self).read_volatile() };
        let color = logos_abi::DisplayColor::from_wire(context.color)?;
        (context.abi == ABI
            && context.reserved == 0
            && context.operation == PRESENT_PIXEL
            && context.status == ACKNOWLEDGED)
            .then_some((context.x, context.y, color))
    }

    /// # Safety
    ///
    /// `address` must point to a live, aligned `Context` mapping.
    pub unsafe fn text_at(address: u64) -> Option<TextRequest> {
        let context = unsafe { (address as *const Self).read_volatile() };
        let length = usize::try_from(context.text_length).ok()?;
        let color = logos_abi::DisplayColor::from_wire(context.color)?;
        (context.abi == ABI
            && context.reserved == 0
            && context.operation == PRESENT_TEXT
            && context.status == ACKNOWLEDGED
            && length <= context.text.len())
        .then_some(TextRequest { x: context.x, y: context.y, color, text: context.text, length })
    }

    /// # Safety
    ///
    /// `address` must point to a live, aligned `Context` mapping.
    pub unsafe fn clear_at(address: u64) -> bool {
        let context = unsafe { (address as *const Self).read_volatile() };
        context.abi == ABI
            && context.reserved == 0
            && context.operation == CLEAR_DISPLAY
            && context.status == ACKNOWLEDGED
    }

    /// # Safety
    /// `address` must point to a live, aligned `Context` mapping.
    pub unsafe fn display_waiting_at(address: u64) -> bool {
        unsafe { Self::pixel_at(address) }.is_some()
            || unsafe { Self::text_at(address) }.is_some()
            || unsafe { Self::clear_at(address) }
    }
}

impl Default for Context {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Clone, Copy)]
#[repr(C)]
pub struct Header {
    pub magic: [u8; 4],
    pub abi: u16,
    pub reserved: u16,
    pub name: [u8; 16],
    pub protocol: ProtocolVersion,
    pub entry: extern "C" fn(*mut Context) -> !,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(C)]
pub struct ProtocolVersion {
    pub major: u16,
    pub minor: u16,
}

impl ProtocolVersion {
    pub const V1: Self = Self { major: 1, minor: 0 };

    pub const fn supports(self, required: Self) -> bool {
        self.major == required.major && self.minor >= required.minor
    }
}

impl Header {
    pub const fn new(
        name: [u8; 16],
        protocol: ProtocolVersion,
        entry: extern "C" fn(*mut Context) -> !,
    ) -> Self {
        Self { magic: MAGIC, abi: ABI, reserved: 0, name, protocol, entry }
    }

    pub fn entry_address(&self) -> usize {
        self.entry as usize
    }

    pub fn valid_for(&self, name: &[u8], protocol: ProtocolVersion) -> bool {
        self.magic == MAGIC
            && self.abi == ABI
            && self.reserved == 0
            && self.protocol.supports(protocol)
            && self.name_starts_with(name)
    }

    fn name_starts_with(&self, name: &[u8]) -> bool {
        if name.len() > self.name.len() {
            return false;
        }
        let mut index = 0;
        while index < name.len() {
            if self.name[index] != name[index] {
                return false;
            }
            index += 1;
        }
        index == self.name.len() || self.name[index] == 0
    }
}

pub fn self_check() -> bool {
    let mut syscall = Context::new();
    syscall.operation = SYSCALL;
    syscall.status = ACKNOWLEDGED;
    syscall.x = logos_abi::Syscall::Inspect as u32;
    syscall.text[..4].copy_from_slice(b"name");
    syscall.text_length = 4;
    let valid = unsafe { Context::syscall_at((&syscall as *const Context) as u64) }.is_some_and(
        |request| request.syscall == logos_abi::Syscall::Inspect && request.length == 4,
    );
    syscall.x = 0;
    let unknown = unsafe { Context::syscall_at((&syscall as *const Context) as u64) }.is_none();
    syscall.x = logos_abi::Syscall::Reboot as u32;
    let malformed = unsafe { Context::syscall_at((&syscall as *const Context) as u64) }.is_none();
    let request = logos_abi::SessionRequest::new(logos_abi::Syscall::Tasks, [0; MAX_TEXT], 0);
    syscall.operation = READ_INPUT;
    let delivered =
        unsafe { Context::deliver_session_at((&mut syscall as *mut Context) as u64, request) }
            && syscall.operation == SYSCALL;
    syscall.operation = SESSION_EFFECT;
    syscall.x = logos_abi::Effect::ReadTasks as u32;
    let effect = unsafe {
        Context::reply_effect_at(
            (&mut syscall as *mut Context) as u64,
            logos_abi::EffectResult::TasksActive,
        )
    } && syscall.x == logos_abi::EffectResult::TasksActive as u32;
    syscall.operation = SESSION_EFFECT;
    let effect_text = unsafe {
        Context::reply_effect_with_text_at(
            (&mut syscall as *mut Context) as u64,
            logos_abi::EffectReply::new(logos_abi::EffectResult::Completed, b"ok"),
        )
    } && syscall.x == logos_abi::EffectResult::Completed as u32
        && syscall.text_length == 2
        && syscall.text[..2] == *b"ok";
    syscall.operation = SESSION_REPLY;
    syscall.text[..2].copy_from_slice(b"ok");
    syscall.text_length = 2;
    let reply = unsafe { Context::session_reply_at((&syscall as *const Context) as u64) }
        .is_some_and(|reply| reply.length == 2 && reply.text[..2] == *b"ok");
    let reset = unsafe { Context::reset_at((&mut syscall as *mut Context) as u64) }
        && syscall.abi == ABI
        && syscall.operation == 0;
    Header::new(*b"terminal\0\0\0\0\0\0\0\0", ProtocolVersion::V1, self_check_entry)
        .valid_for(b"terminal", ProtocolVersion::V1)
        && !Header::new(*b"terminal\0\0\0\0\0\0\0\0", ProtocolVersion::V1, self_check_entry)
            .valid_for(b"other", ProtocolVersion::V1)
        && valid
        && unknown
        && malformed
        && delivered
        && effect
        && effect_text
        && reply
        && reset
}

extern "C" fn self_check_entry(_: *mut Context) -> ! {
    loop {
        core::hint::spin_loop();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn persistence_replies_round_trip_and_match_ids() {
        let mut context = Context::new();
        context.operation = READ_INPUT;
        context.status = ACKNOWLEDGED;
        let store = logos_abi::StoreRequest {
            id: 7,
            operation: logos_abi::StoreOperation::Commit,
            namespace: logos_abi::NamespaceId(0),
            name: [0; logos_abi::MAX_OBJECT_NAME],
            name_length: 0,
            version: logos_abi::VersionSelector::None,
            offset: 0,
            length: 0,
            page: logos_abi::PageHandle(0),
            deadline: 0,
        };
        let address = (&mut context as *mut Context) as u64;
        assert!(unsafe { Context::request_store_at(address, store) });
        assert!(unsafe {
            Context::store_at(address).is_some_and(|request| {
                request.id == store.id && request.operation == store.operation
            })
        });
        let block = logos_abi::BlockRequest {
            id: 9,
            operation: logos_abi::BlockOperation::Flush,
            lba: 0,
            blocks: 0,
            page: logos_abi::PageHandle(0),
            deadline: 0,
        };
        assert!(unsafe { Context::request_block_at(address, block) });
        assert_eq!(unsafe { Context::block_at(address) }, Some(block));
        let block_reply = logos_abi::BlockReply {
            id: 9,
            status: logos_abi::PersistenceStatus::Complete,
            info: logos_abi::BlockInfo::default(),
        };
        assert!(unsafe { Context::reply_block_at(address, block_reply) });
        assert!(unsafe { Context::block_reply_at(address, 9) }.is_some());
        assert!(unsafe {
            !Context::reply_store_at(
                address,
                logos_abi::StoreReply {
                    id: 8,
                    status: logos_abi::PersistenceStatus::Complete,
                    version: 3,
                    length: 0,
                },
            )
        });
        let store_reply = logos_abi::StoreReply {
            id: 7,
            status: logos_abi::PersistenceStatus::Complete,
            version: 3,
            length: 0,
        };
        assert!(unsafe { Context::reply_store_at(address, store_reply) });
        assert!(unsafe { Context::store_reply_at(address, 8) }.is_none());
        assert_eq!(unsafe { Context::store_reply_at(address, 7) }, Some(store_reply));

        context.operation = 0;
        context.status = 0;
        unsafe { (address as *mut Context).write_volatile(context) };
        assert!(unsafe {
            Context::configure_shared_page_at(address, logos_abi::PageHandle(0x10001))
        });
        assert_eq!(
            unsafe { Context::shared_page_at(address) },
            Some(logos_abi::PageHandle(0x10001))
        );
        context.operation = READY;
        context.status = ACKNOWLEDGED;
        unsafe { (address as *mut Context).write_volatile(context) };
        assert!(unsafe { Context::request_store_at(address, store) });
        assert!(
            unsafe { Context::store_at(address) }.is_some_and(
                |request| request.id == store.id && request.operation == store.operation
            )
        );

        context.operation = READ_INPUT;
        context.status = ACKNOWLEDGED;
        unsafe { (address as *mut Context).write_volatile(context) };
        let block = logos_abi::BlockRequest {
            id: 9,
            operation: logos_abi::BlockOperation::Flush,
            lba: 0,
            blocks: 0,
            page: logos_abi::PageHandle(0),
            deadline: 0,
        };
        assert!(unsafe { Context::request_block_at(address, block) });
        assert_eq!(unsafe { Context::block_at(address) }, Some(block));
        let block_reply = logos_abi::BlockReply {
            id: 9,
            status: logos_abi::PersistenceStatus::Complete,
            info: logos_abi::BlockInfo::default(),
        };
        assert!(unsafe { Context::reply_block_at(address, block_reply) });
        assert!(unsafe { Context::block_reply_at(address, 10) }.is_none());
        assert_eq!(unsafe { Context::block_reply_at(address, 9) }, Some(block_reply));
    }

    #[test]
    fn block_page_is_configured_and_reply_ids_are_checked() {
        let mut context = Context::new();
        let address = (&mut context as *mut Context) as u64;
        let page = BlockPage { handle: logos_abi::PageHandle(7), address: 0x2000 };
        assert!(unsafe { Context::configure_block_page_at(address, page) });
        assert_eq!(unsafe { Context::block_page_at(address) }, Some(page));
        context.operation = READY;
        context.status = ACKNOWLEDGED;
        unsafe { (address as *mut Context).write_volatile(context) };
        let request = logos_abi::BlockRequest {
            id: 3,
            operation: logos_abi::BlockOperation::Info,
            lba: 0,
            blocks: 0,
            page: logos_abi::PageHandle(0),
            deadline: 1,
        };
        assert!(unsafe { Context::request_block_at(address, request) });
        assert!(!unsafe {
            Context::reply_block_at(
                address,
                logos_abi::BlockReply {
                    id: 4,
                    status: logos_abi::PersistenceStatus::Complete,
                    info: logos_abi::BlockInfo {
                        logical_block_size: 512,
                        blocks: 1,
                        max_transfer_blocks: 1,
                    },
                },
            )
        });
    }

    #[test]
    fn network_request_reply_and_deadline_event_are_bounded() {
        let mut context = Context::new();
        context.operation = READ_INPUT;
        context.status = ACKNOWLEDGED;
        let request = logos_abi::NetworkRequest {
            id: 11,
            operation: logos_abi::NetworkOperation::Bind,
            endpoint: logos_abi::NetworkEndpoint(0),
            peer: logos_abi::NetworkScope::new(logos_abi::NetworkProtocol::Udp, 0, 4000),
            page: logos_abi::PageHandle(0),
            length: 0,
            generation: 0,
            deadline: 100,
        };
        let address = (&mut context as *mut Context) as u64;
        assert!(unsafe { Context::request_network_at(address, request) });
        assert_eq!(unsafe { Context::network_at(address) }, Some(request));
        let reply = logos_abi::NetworkReply {
            id: request.id,
            status: logos_abi::NetworkStatus::Complete,
            endpoint: logos_abi::NetworkEndpoint::new(1, 1).unwrap(),
            generation: 1,
            source_address: 0,
            source_port: 0,
            length: 0,
            info: logos_abi::NetworkInfo { generation: 1, ..Default::default() },
            counters: logos_abi::NetworkCounters::default(),
        };
        assert!(unsafe { Context::reply_network_at(address, reply) });
        assert_eq!(unsafe { Context::network_reply_at(address, request.id) }, Some(reply));
        assert!(unsafe { Context::network_wait_at(address, 101) });
        assert!(unsafe { Context::network_waiting_at(address) });
        let event = logos_abi::NetworkEvent {
            id: 12,
            kind: logos_abi::NetworkEventKind::Timer,
            generation: 1,
            length: 0,
            now: 101,
        };
        assert!(unsafe { Context::deliver_network_event_at(address, event) });
        assert_eq!(unsafe { Context::network_event_at(address) }, Some(event));
        assert!(unsafe { Context::reply_network_after_event_at(address, request, reply) });
        assert_eq!(unsafe { Context::network_reply_at(address, request.id) }, Some(reply));
        assert!(unsafe { !Context::network_wait_at(address, 0) });
        assert!(unsafe { Context::network_reply_at(address, request.id + 1) }.is_none());

        let mut pages_context = Context::new();
        let pages_address = (&mut pages_context as *mut Context) as u64;
        assert!(unsafe {
            Context::configure_network_pages_at(
                pages_address,
                logos_abi::PageHandle(1),
                logos_abi::PageHandle(2),
            )
        });
        let pages = unsafe { Context::network_pages_at(pages_address) }.unwrap();
        assert_eq!(pages.tx_address, pages.rx_address - 4096);
    }

    #[test]
    fn network_device_gate_rejects_mismatch_and_delivers_async_completion() {
        let mut context = Context::new();
        context.operation = READ_INPUT;
        context.status = ACKNOWLEDGED;
        let address = (&mut context as *mut Context) as u64;
        let request = logos_abi::NetworkDeviceRequest {
            id: 9,
            operation: logos_abi::NetworkDeviceOperation::Info,
            length: 0,
            generation: 0,
            deadline: 1,
        };
        assert!(unsafe { Context::request_network_device_at(address, request) });
        let reply = logos_abi::NetworkDeviceReply {
            id: request.id,
            status: logos_abi::NetworkStatus::Complete,
            generation: 1,
            info: logos_abi::NetworkInfo { generation: 1, ..Default::default() },
        };
        assert!(!unsafe {
            Context::reply_network_device_at(
                address,
                logos_abi::NetworkDeviceReply { id: 8, ..reply },
            )
        });
        assert!(unsafe { Context::reply_network_device_at(address, reply) });
        assert!(unsafe { Context::network_wait_at(address, 2) });
        assert!(unsafe { Context::deliver_network_device_reply_at(address, reply) });
        assert_eq!(unsafe { Context::network_device_reply_at(address, request.id) }, Some(reply));
        assert!(unsafe { Context::network_device_reply_at(address, request.id + 1) }.is_none());
    }
}
