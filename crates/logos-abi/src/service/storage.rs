use super::*;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u32)]
pub enum PersistencePageState {
    Ready = 1,
    Waiting = 2,
    Request = 3,
    Processing = 4,
    Submitted = 5,
    Reply = 6,
    Denied = 7,
    Failed = 8,
    Cancelled = 9,
    TimedOut = 10,
}

impl PersistencePageState {
    pub const fn wire(self) -> u32 {
        self as u32
    }

    pub const fn from_wire(value: u32) -> Option<Self> {
        Some(match value {
            1 => Self::Ready,
            2 => Self::Waiting,
            3 => Self::Request,
            4 => Self::Processing,
            5 => Self::Submitted,
            6 => Self::Reply,
            7 => Self::Denied,
            8 => Self::Failed,
            9 => Self::Cancelled,
            10 => Self::TimedOut,
            _ => return None,
        })
    }
}

/// Core-mediated Store client page. Only the owning client service maps it.
#[derive(Clone, Copy)]
#[repr(C)]
pub struct StoreClientPage {
    pub service_generation: u32,
    pub endpoint_generation: u32,
    pub state: u32,
    pub request_id: u32,
    pub operation: u32,
    pub namespace: u32,
    pub name_length: u32,
    pub name: [u8; logos_abi::MAX_OBJECT_NAME],
    pub version: u32,
    pub offset: u64,
    pub length: u32,
    pub page: u32,
    pub deadline: u64,
    pub transfer_page: u32,
    pub reply_status: u32,
    pub reply_version: u64,
    pub reply_length: u32,
}

/// Core-mediated Store server page. Only Storage maps it.
#[derive(Clone, Copy)]
#[repr(C)]
pub struct StoreServerPage {
    pub service_generation: u32,
    pub endpoint_generation: u32,
    pub state: u32,
    pub request_id: u32,
    pub caller_low: u32,
    pub caller_high: u32,
    pub operation: u32,
    pub namespace: u32,
    pub name_length: u32,
    pub name: [u8; logos_abi::MAX_OBJECT_NAME],
    pub version: u32,
    pub offset: u64,
    pub length: u32,
    pub page: u32,
    pub deadline: u64,
    pub transfer_page: u32,
    pub reply_status: u32,
    pub reply_version: u64,
    pub reply_length: u32,
    pub service_status: u32,
}

#[derive(Clone, Copy)]
pub struct StoreServerRequest {
    pub id: u32,
    pub caller: u64,
    pub request: logos_abi::StoreRequest,
}

pub(super) fn persistence_state(status: logos_abi::PersistenceStatus) -> PersistencePageState {
    match status {
        logos_abi::PersistenceStatus::Complete | logos_abi::PersistenceStatus::Recovered => {
            PersistencePageState::Reply
        }
        logos_abi::PersistenceStatus::Denied => PersistencePageState::Denied,
        logos_abi::PersistenceStatus::Cancelled => PersistencePageState::Cancelled,
        logos_abi::PersistenceStatus::TimedOut => PersistencePageState::TimedOut,
        _ => PersistencePageState::Failed,
    }
}

#[allow(clippy::too_many_arguments)]
fn store_request_from_fields(
    id: u32,
    operation: u32,
    namespace: u32,
    name_length: u32,
    name: [u8; logos_abi::MAX_OBJECT_NAME],
    version: u32,
    offset: u64,
    length: u32,
    page: u32,
    deadline: u64,
) -> Option<logos_abi::StoreRequest> {
    let operation = logos_abi::StoreOperation::from_wire(u8::try_from(operation).ok()?)?;
    let version = logos_abi::VersionSelector::from_wire(u8::try_from(version).ok()?)?;
    let request = logos_abi::StoreRequest {
        id,
        operation,
        namespace: logos_abi::NamespaceId(namespace),
        name,
        name_length: u8::try_from(name_length).ok()?,
        version,
        offset,
        length,
        page: logos_abi::PageHandle(page),
        deadline,
    };
    request.valid().then_some(request)
}

fn store_reply_from_fields(
    id: u32,
    status: u32,
    version: u64,
    length: u32,
) -> Option<logos_abi::StoreReply> {
    Some(logos_abi::StoreReply {
        id,
        status: logos_abi::PersistenceStatus::from_wire(u8::try_from(status).ok()?)?,
        version,
        length,
    })
}

#[allow(clippy::missing_safety_doc)]
impl StoreClientPage {
    pub const fn new(service_generation: u32, endpoint_generation: u32) -> Self {
        Self {
            service_generation,
            endpoint_generation,
            state: PersistencePageState::Ready.wire(),
            request_id: 0,
            operation: 0,
            namespace: 0,
            name_length: 0,
            name: [0; logos_abi::MAX_OBJECT_NAME],
            version: 0,
            offset: 0,
            length: 0,
            page: 0,
            deadline: 0,
            transfer_page: 0,
            reply_status: 0,
            reply_version: 0,
            reply_length: 0,
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
            || PersistencePageState::from_wire(page.state) != Some(PersistencePageState::Ready)
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
        request: logos_abi::StoreRequest,
    ) -> bool {
        if request.id == 0 || !request.valid() {
            return false;
        }
        let mut page = unsafe { (address as *mut Self).read_volatile() };
        if !client_identity(&page, service_generation, endpoint_generation)
            || PersistencePageState::from_wire(page.state) != Some(PersistencePageState::Ready)
        {
            return false;
        }
        page.request_id = request.id;
        page.operation = request.operation as u32;
        page.namespace = request.namespace.0;
        page.name_length = u32::from(request.name_length);
        page.name = request.name;
        page.version = request.version as u32;
        page.offset = request.offset;
        page.length = request.length;
        page.page = request.page.0;
        page.deadline = request.deadline;
        page.reply_status = 0;
        page.reply_version = 0;
        page.reply_length = 0;
        page.state = PersistencePageState::Request.wire();
        unsafe { (address as *mut Self).write_volatile(page) };
        true
    }

    pub unsafe fn current_request_at(
        address: u64,
        service_generation: u32,
        endpoint_generation: u32,
    ) -> Option<logos_abi::StoreRequest> {
        let mut page = unsafe { (address as *mut Self).read_volatile() };
        let state = PersistencePageState::from_wire(page.state)?;
        if !client_identity(&page, service_generation, endpoint_generation)
            || !matches!(state, PersistencePageState::Request | PersistencePageState::Waiting)
        {
            return None;
        }
        let request = store_request_from_fields(
            page.request_id,
            page.operation,
            page.namespace,
            page.name_length,
            page.name,
            page.version,
            page.offset,
            page.length,
            page.page,
            page.deadline,
        )?;
        if state == PersistencePageState::Request {
            page.state = PersistencePageState::Waiting.wire();
            unsafe { (address as *mut Self).write_volatile(page) };
        }
        Some(request)
    }

    pub unsafe fn pending_at(
        address: u64,
        service_generation: u32,
        endpoint_generation: u32,
    ) -> bool {
        let page = unsafe { (address as *const Self).read_volatile() };
        client_identity(&page, service_generation, endpoint_generation)
            && PersistencePageState::from_wire(page.state) == Some(PersistencePageState::Request)
    }

    pub unsafe fn reply_pending_at(
        address: u64,
        service_generation: u32,
        endpoint_generation: u32,
    ) -> bool {
        let page = unsafe { (address as *const Self).read_volatile() };
        client_identity(&page, service_generation, endpoint_generation)
            && matches!(
                PersistencePageState::from_wire(page.state),
                Some(
                    PersistencePageState::Reply
                        | PersistencePageState::Denied
                        | PersistencePageState::Failed
                        | PersistencePageState::Cancelled
                        | PersistencePageState::TimedOut
                )
            )
    }

    pub unsafe fn reply_at(
        address: u64,
        service_generation: u32,
        endpoint_generation: u32,
        reply: logos_abi::StoreReply,
    ) -> bool {
        let mut page = unsafe { (address as *mut Self).read_volatile() };
        let Some(request) =
            (unsafe { Self::current_request_at(address, service_generation, endpoint_generation) })
        else {
            return false;
        };
        if !client_identity(&page, service_generation, endpoint_generation)
            || !reply.valid_for(request)
        {
            return false;
        }
        page.reply_status = reply.status as u32;
        page.reply_version = reply.version;
        page.reply_length = reply.length;
        page.state = persistence_state(reply.status).wire();
        unsafe { (address as *mut Self).write_volatile(page) };
        true
    }

    pub unsafe fn finish_at(
        address: u64,
        service_generation: u32,
        endpoint_generation: u32,
        id: u32,
    ) -> Option<logos_abi::StoreReply> {
        let page = unsafe { (address as *const Self).read_volatile() };
        let state = PersistencePageState::from_wire(page.state)?;
        if !client_identity(&page, service_generation, endpoint_generation)
            || page.request_id != id
            || !matches!(
                state,
                PersistencePageState::Reply
                    | PersistencePageState::Denied
                    | PersistencePageState::Failed
                    | PersistencePageState::Cancelled
                    | PersistencePageState::TimedOut
            )
        {
            return None;
        }
        let reply = store_reply_from_fields(
            page.request_id,
            page.reply_status,
            page.reply_version,
            page.reply_length,
        )?;
        unsafe { Self::reset_at(address, service_generation, endpoint_generation) };
        Some(reply)
    }

    pub unsafe fn cancel_at(
        address: u64,
        service_generation: u32,
        endpoint_generation: u32,
        id: u32,
    ) -> bool {
        let mut page = unsafe { (address as *mut Self).read_volatile() };
        if !client_identity(&page, service_generation, endpoint_generation)
            || page.request_id != id
            || !matches!(
                PersistencePageState::from_wire(page.state),
                Some(PersistencePageState::Request | PersistencePageState::Waiting)
            )
        {
            return false;
        }
        page.reply_status = logos_abi::PersistenceStatus::Cancelled as u32;
        page.reply_version = 0;
        page.reply_length = 0;
        page.state = PersistencePageState::Cancelled.wire();
        unsafe { (address as *mut Self).write_volatile(page) };
        true
    }
}

#[allow(clippy::missing_safety_doc)]
impl StoreServerPage {
    pub const fn new(service_generation: u32, endpoint_generation: u32) -> Self {
        Self {
            service_generation,
            endpoint_generation,
            state: PersistencePageState::Ready.wire(),
            request_id: 0,
            caller_low: 0,
            caller_high: 0,
            operation: 0,
            namespace: 0,
            name_length: 0,
            name: [0; logos_abi::MAX_OBJECT_NAME],
            version: 0,
            offset: 0,
            length: 0,
            page: 0,
            deadline: 0,
            transfer_page: 0,
            reply_status: 0,
            reply_version: 0,
            reply_length: 0,
            service_status: STORAGE_UNAVAILABLE,
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
        page.service_status = old.service_status;
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
        if !server_identity(&page, service_generation, endpoint_generation)
            || !matches!(
                PersistencePageState::from_wire(page.state),
                Some(PersistencePageState::Ready | PersistencePageState::Waiting)
            )
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
        (server_identity(&page, service_generation, endpoint_generation) && page.transfer_page != 0)
            .then_some(logos_abi::PageHandle(page.transfer_page))
    }

    pub unsafe fn wait_at(address: u64, service_generation: u32, endpoint_generation: u32) -> bool {
        let mut page = unsafe { (address as *mut Self).read_volatile() };
        if !server_identity(&page, service_generation, endpoint_generation)
            || PersistencePageState::from_wire(page.state) != Some(PersistencePageState::Ready)
        {
            return false;
        }
        page.state = PersistencePageState::Waiting.wire();
        unsafe { (address as *mut Self).write_volatile(page) };
        true
    }

    pub unsafe fn waiting_at(
        address: u64,
        service_generation: u32,
        endpoint_generation: u32,
    ) -> bool {
        let page = unsafe { (address as *const Self).read_volatile() };
        server_identity(&page, service_generation, endpoint_generation)
            && PersistencePageState::from_wire(page.state) == Some(PersistencePageState::Waiting)
    }

    pub unsafe fn deliver_at(
        address: u64,
        service_generation: u32,
        endpoint_generation: u32,
        caller: u64,
        request: logos_abi::StoreRequest,
    ) -> bool {
        if request.id == 0 || !request.valid() {
            return false;
        }
        let mut page = unsafe { (address as *mut Self).read_volatile() };
        if !server_identity(&page, service_generation, endpoint_generation)
            || PersistencePageState::from_wire(page.state) != Some(PersistencePageState::Waiting)
        {
            return false;
        }
        page.request_id = request.id;
        page.caller_low = caller as u32;
        page.caller_high = (caller >> 32) as u32;
        page.operation = request.operation as u32;
        page.namespace = request.namespace.0;
        page.name_length = u32::from(request.name_length);
        page.name = request.name;
        page.version = request.version as u32;
        page.offset = request.offset;
        page.length = request.length;
        page.page = request.page.0;
        page.deadline = request.deadline;
        page.reply_status = 0;
        page.reply_version = 0;
        page.reply_length = 0;
        page.state = PersistencePageState::Request.wire();
        unsafe { (address as *mut Self).write_volatile(page) };
        true
    }

    pub unsafe fn take_at(
        address: u64,
        service_generation: u32,
        endpoint_generation: u32,
    ) -> Option<StoreServerRequest> {
        let mut page = unsafe { (address as *mut Self).read_volatile() };
        if !server_identity(&page, service_generation, endpoint_generation)
            || PersistencePageState::from_wire(page.state) != Some(PersistencePageState::Request)
        {
            return None;
        }
        let request = store_request_from_fields(
            page.request_id,
            page.operation,
            page.namespace,
            page.name_length,
            page.name,
            page.version,
            page.offset,
            page.length,
            page.page,
            page.deadline,
        )?;
        page.state = PersistencePageState::Processing.wire();
        unsafe { (address as *mut Self).write_volatile(page) };
        Some(StoreServerRequest {
            id: page.request_id,
            caller: u64::from(page.caller_low) | (u64::from(page.caller_high) << 32),
            request,
        })
    }

    pub unsafe fn reply_at(
        address: u64,
        service_generation: u32,
        endpoint_generation: u32,
        reply: logos_abi::StoreReply,
    ) -> bool {
        let mut page = unsafe { (address as *mut Self).read_volatile() };
        let Some(request) = store_request_from_fields(
            page.request_id,
            page.operation,
            page.namespace,
            page.name_length,
            page.name,
            page.version,
            page.offset,
            page.length,
            page.page,
            page.deadline,
        ) else {
            return false;
        };
        if !server_identity(&page, service_generation, endpoint_generation)
            || PersistencePageState::from_wire(page.state) != Some(PersistencePageState::Processing)
            || !reply.valid_for(request)
        {
            return false;
        }
        page.reply_status = reply.status as u32;
        page.reply_version = reply.version;
        page.reply_length = reply.length;
        page.state = persistence_state(reply.status).wire();
        unsafe { (address as *mut Self).write_volatile(page) };
        true
    }

    pub unsafe fn take_reply_at(
        address: u64,
        service_generation: u32,
        endpoint_generation: u32,
        expected_id: u32,
    ) -> Option<logos_abi::StoreReply> {
        let page = unsafe { (address as *const Self).read_volatile() };
        if !server_identity(&page, service_generation, endpoint_generation)
            || page.request_id != expected_id
            || !matches!(
                PersistencePageState::from_wire(page.state),
                Some(
                    PersistencePageState::Reply
                        | PersistencePageState::Denied
                        | PersistencePageState::Failed
                        | PersistencePageState::Cancelled
                        | PersistencePageState::TimedOut
                )
            )
        {
            return None;
        }
        let reply = store_reply_from_fields(
            page.request_id,
            page.reply_status,
            page.reply_version,
            page.reply_length,
        )?;
        unsafe { Self::reset_at(address, service_generation, endpoint_generation) };
        Some(reply)
    }

    pub unsafe fn reply_pending_at(
        address: u64,
        service_generation: u32,
        endpoint_generation: u32,
    ) -> bool {
        let page = unsafe { (address as *const Self).read_volatile() };
        server_identity(&page, service_generation, endpoint_generation)
            && matches!(
                PersistencePageState::from_wire(page.state),
                Some(
                    PersistencePageState::Reply
                        | PersistencePageState::Denied
                        | PersistencePageState::Failed
                        | PersistencePageState::Cancelled
                        | PersistencePageState::TimedOut
                )
            )
    }

    pub unsafe fn set_status_at(
        address: u64,
        service_generation: u32,
        endpoint_generation: u32,
        status: u32,
    ) -> bool {
        if !(STORAGE_FORMATTED..=STORAGE_UNAVAILABLE).contains(&status) {
            return false;
        }
        let mut page = unsafe { (address as *mut Self).read_volatile() };
        if !server_identity(&page, service_generation, endpoint_generation) {
            return false;
        }
        page.service_status = status;
        unsafe { (address as *mut Self).write_volatile(page) };
        true
    }

    pub unsafe fn status_at(
        address: u64,
        service_generation: u32,
        endpoint_generation: u32,
    ) -> Option<u32> {
        let page = unsafe { (address as *const Self).read_volatile() };
        (server_identity(&page, service_generation, endpoint_generation)
            && (STORAGE_FORMATTED..=STORAGE_UNAVAILABLE).contains(&page.service_status))
        .then_some(page.service_status)
    }
}
