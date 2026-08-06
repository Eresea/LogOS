use crate as logos_abi;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u32)]
pub enum RemotePageState {
    Ready = 1,
    Request = 2,
    Processing = 3,
    Reply = 4,
    Denied = 5,
    Failed = 6,
}

impl RemotePageState {
    pub const fn from_wire(value: u32) -> Option<Self> {
        Some(match value {
            1 => Self::Ready,
            2 => Self::Request,
            3 => Self::Processing,
            4 => Self::Reply,
            5 => Self::Denied,
            6 => Self::Failed,
            _ => return None,
        })
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u32)]
pub enum RemoteGateOperation {
    Handshake = 1,
    Open,
    Invoke,
    Seal,
    Subscribe,
    Credit,
    Acknowledge,
    Reset,
}

impl RemoteGateOperation {
    pub(super) fn from_wire(value: u32) -> Option<Self> {
        Some(match value {
            1 => Self::Handshake,
            2 => Self::Open,
            3 => Self::Invoke,
            4 => Self::Seal,
            5 => Self::Subscribe,
            6 => Self::Credit,
            7 => Self::Acknowledge,
            8 => Self::Reset,
            _ => return None,
        })
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u32)]
pub enum RemoteGateStatus {
    Complete = 1,
    Busy,
    Denied,
    Invalid,
    Unavailable,
    Indeterminate,
}

impl RemoteGateStatus {
    pub(super) fn from_wire(value: u32) -> Option<Self> {
        Some(match value {
            1 => Self::Complete,
            2 => Self::Busy,
            3 => Self::Denied,
            4 => Self::Invalid,
            5 => Self::Unavailable,
            6 => Self::Indeterminate,
            _ => return None,
        })
    }
}

#[derive(Clone, Copy)]
#[repr(C)]
pub struct RemotePage {
    pub service_generation: u32,
    pub endpoint_generation: u32,
    pub state: u32,
    pub request_id: u32,
    pub operation: u32,
    pub page: u32,
    pub length: u16,
    pub reserved0: u16,
    pub deadline: u64,
    pub reply_status: u32,
    pub reply_length: u16,
    pub reserved1: u16,
    pub reply_cursor: u64,
    pub reserved: [u8; logos_abi::PAGE_SIZE - 56],
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RemotePageRequest {
    pub id: u32,
    pub operation: RemoteGateOperation,
    pub page: logos_abi::PageHandle,
    pub length: u16,
    pub deadline: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RemotePageReply {
    pub id: u32,
    pub status: RemoteGateStatus,
    pub length: u16,
    pub cursor: u64,
}

fn identity(page: &RemotePage, service_generation: u32, endpoint_generation: u32) -> bool {
    page.service_generation == service_generation && page.endpoint_generation == endpoint_generation
}

#[allow(clippy::missing_safety_doc)]
impl RemotePage {
    pub const fn new(service_generation: u32, endpoint_generation: u32) -> Self {
        Self {
            service_generation,
            endpoint_generation,
            state: RemotePageState::Ready as u32,
            request_id: 0,
            operation: 0,
            page: 0,
            length: 0,
            reserved0: 0,
            deadline: 0,
            reply_status: 0,
            reply_length: 0,
            reserved1: 0,
            reply_cursor: 0,
            reserved: [0; logos_abi::PAGE_SIZE - 56],
        }
    }

    pub unsafe fn reset_at(
        address: u64,
        service_generation: u32,
        endpoint_generation: u32,
    ) -> bool {
        if !super::valid_page_identity::<Self>(address, service_generation, endpoint_generation) {
            return false;
        }
        unsafe {
            (address as *mut Self)
                .write_volatile(Self::new(service_generation, endpoint_generation))
        };
        true
    }

    pub unsafe fn request_at(
        address: u64,
        service_generation: u32,
        endpoint_generation: u32,
        request: RemotePageRequest,
    ) -> bool {
        if request.id == 0 || request.page.0 == 0 || request.length as usize > logos_abi::PAGE_SIZE
        {
            return false;
        }
        let mut page = unsafe { (address as *mut Self).read_volatile() };
        if !identity(&page, service_generation, endpoint_generation)
            || RemotePageState::from_wire(page.state) != Some(RemotePageState::Ready)
        {
            return false;
        }
        page.request_id = request.id;
        page.operation = request.operation as u32;
        page.page = request.page.0;
        page.length = request.length;
        page.deadline = request.deadline;
        page.reply_status = 0;
        page.reply_length = 0;
        page.reply_cursor = 0;
        page.state = RemotePageState::Request as u32;
        unsafe { (address as *mut Self).write_volatile(page) };
        true
    }

    pub unsafe fn take_at(
        address: u64,
        service_generation: u32,
        endpoint_generation: u32,
    ) -> Option<RemotePageRequest> {
        let mut page = unsafe { (address as *mut Self).read_volatile() };
        if !identity(&page, service_generation, endpoint_generation)
            || RemotePageState::from_wire(page.state) != Some(RemotePageState::Request)
        {
            return None;
        }
        let request = RemotePageRequest {
            id: page.request_id,
            operation: RemoteGateOperation::from_wire(page.operation)?,
            page: logos_abi::PageHandle(page.page),
            length: page.length,
            deadline: page.deadline,
        };
        page.state = RemotePageState::Processing as u32;
        unsafe { (address as *mut Self).write_volatile(page) };
        Some(request)
    }

    pub unsafe fn pending_at(
        address: u64,
        service_generation: u32,
        endpoint_generation: u32,
    ) -> bool {
        let page = unsafe { (address as *const Self).read_volatile() };
        identity(&page, service_generation, endpoint_generation)
            && RemotePageState::from_wire(page.state) == Some(RemotePageState::Request)
    }

    pub unsafe fn reply_at(
        address: u64,
        service_generation: u32,
        endpoint_generation: u32,
        reply: RemotePageReply,
    ) -> bool {
        if reply.length as usize > logos_abi::PAGE_SIZE {
            return false;
        }
        let mut page = unsafe { (address as *mut Self).read_volatile() };
        if !identity(&page, service_generation, endpoint_generation)
            || page.request_id != reply.id
            || RemotePageState::from_wire(page.state) != Some(RemotePageState::Processing)
        {
            return false;
        }
        page.reply_status = reply.status as u32;
        page.reply_length = reply.length;
        page.reply_cursor = reply.cursor;
        page.state = match reply.status {
            RemoteGateStatus::Complete => RemotePageState::Reply,
            RemoteGateStatus::Denied => RemotePageState::Denied,
            _ => RemotePageState::Failed,
        } as u32;
        unsafe { (address as *mut Self).write_volatile(page) };
        true
    }

    pub unsafe fn finish_at(
        address: u64,
        service_generation: u32,
        endpoint_generation: u32,
        expected_id: u32,
    ) -> Option<RemotePageReply> {
        let page = unsafe { (address as *const Self).read_volatile() };
        if !identity(&page, service_generation, endpoint_generation)
            || page.request_id != expected_id
            || !matches!(
                RemotePageState::from_wire(page.state),
                Some(RemotePageState::Reply | RemotePageState::Denied | RemotePageState::Failed)
            )
        {
            return None;
        }
        let reply = RemotePageReply {
            id: page.request_id,
            status: RemoteGateStatus::from_wire(page.reply_status)?,
            length: page.reply_length,
            cursor: page.reply_cursor,
        };
        unsafe { Self::reset_at(address, service_generation, endpoint_generation) };
        Some(reply)
    }
}

const _: () = assert!(core::mem::size_of::<RemotePage>() == logos_abi::PAGE_SIZE);
