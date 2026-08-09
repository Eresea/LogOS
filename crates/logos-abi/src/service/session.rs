use super::*;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u32)]
pub enum SessionPageState {
    Ready = 1,
    Waiting = 2,
    Request = 3,
    Processing = 4,
    Reply = 5,
    Failed = 6,
    Cancelled = 7,
    Denied = 8,
}

impl SessionPageState {
    pub const fn wire(self) -> u32 {
        self as u32
    }

    pub const fn from_wire(value: u32) -> Option<Self> {
        Some(match value {
            1 => Self::Ready,
            2 => Self::Waiting,
            3 => Self::Request,
            4 => Self::Processing,
            5 => Self::Reply,
            6 => Self::Failed,
            7 => Self::Cancelled,
            8 => Self::Denied,
            _ => return None,
        })
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u32)]
pub enum SessionStatus {
    Complete = 1,
    Denied = 2,
    Failed = 3,
    Cancelled = 4,
}

impl SessionStatus {
    pub const fn wire(self) -> u32 {
        self as u32
    }

    pub const fn from_wire(value: u32) -> Option<Self> {
        Some(match value {
            1 => Self::Complete,
            2 => Self::Denied,
            3 => Self::Failed,
            4 => Self::Cancelled,
            _ => return None,
        })
    }

    const fn state(self) -> SessionPageState {
        match self {
            Self::Complete => SessionPageState::Reply,
            Self::Denied => SessionPageState::Denied,
            Self::Failed => SessionPageState::Failed,
            Self::Cancelled => SessionPageState::Cancelled,
        }
    }
}

/// Terminal-owned Session request/reply endpoint. Core mediates this page.
#[derive(Clone, Copy)]
#[repr(C)]
pub struct SessionClientPage {
    pub service_generation: u32,
    pub endpoint_generation: u32,
    pub state: u32,
    pub request_id: u32,
    pub operation: u32,
    pub request_length: u32,
    pub reply_status: u32,
    pub reply_length: u32,
    pub request: [u8; MAX_TEXT],
    pub reply: [u8; MAX_TEXT],
}

/// Sessions-owned server endpoint. It is never mapped into a client service.
#[derive(Clone, Copy)]
#[repr(C)]
pub struct SessionServerPage {
    pub service_generation: u32,
    pub endpoint_generation: u32,
    pub state: u32,
    pub request_id: u32,
    pub caller_low: u32,
    pub caller_high: u32,
    pub operation: u32,
    pub request_length: u32,
    pub reply_status: u32,
    pub reply_length: u32,
    pub request: [u8; MAX_TEXT],
    pub reply: [u8; MAX_TEXT],
}

/// Sessions-owned privileged-effect endpoint. Core alone executes requests.
#[derive(Clone, Copy)]
#[repr(C)]
pub struct EffectPage {
    pub service_generation: u32,
    pub endpoint_generation: u32,
    pub state: u32,
    pub request_id: u32,
    pub effect: u32,
    pub request_length: u32,
    pub result: u32,
    pub reply_length: u32,
    pub request: [u8; MAX_TEXT],
    pub reply: [u8; MAX_TEXT],
}

#[derive(Clone, Copy)]
pub struct SessionClientRequest {
    pub id: u32,
    pub request: logos_abi::SessionRequest,
}

#[derive(Clone, Copy)]
pub struct SessionClientReply {
    pub id: u32,
    pub status: SessionStatus,
    pub reply: logos_abi::SessionReply,
}

#[derive(Clone, Copy)]
pub struct SessionServerRequest {
    pub id: u32,
    pub caller: u64,
    pub request: logos_abi::SessionRequest,
}

#[derive(Clone, Copy)]
pub struct SessionServerReply {
    pub id: u32,
    pub status: SessionStatus,
    pub reply: logos_abi::SessionReply,
}

#[derive(Clone, Copy)]
pub struct EffectMessage {
    pub id: u32,
    pub request: logos_abi::EffectRequest,
}

#[derive(Clone, Copy)]
pub struct EffectResponse {
    pub id: u32,
    pub reply: logos_abi::EffectReply,
}

impl SessionClientPage {
    pub const fn new(service_generation: u32, endpoint_generation: u32) -> Self {
        Self {
            service_generation,
            endpoint_generation,
            state: SessionPageState::Ready.wire(),
            request_id: 0,
            operation: 0,
            request_length: 0,
            reply_status: 0,
            reply_length: 0,
            request: [0; MAX_TEXT],
            reply: [0; MAX_TEXT],
        }
    }

    /// # Safety
    /// `address` must point to a live, aligned client page mapping.
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

    /// # Safety
    /// The mapped client service owns the page while creating the request.
    pub unsafe fn request_at(
        address: u64,
        service_generation: u32,
        endpoint_generation: u32,
        id: u32,
        request: logos_abi::SessionRequest,
    ) -> bool {
        if id == 0 || !request.valid() {
            return false;
        }
        let mut page = unsafe { (address as *mut Self).read_volatile() };
        if !client_identity(&page, service_generation, endpoint_generation)
            || SessionPageState::from_wire(page.state) != Some(SessionPageState::Ready)
        {
            return false;
        }
        page.request_id = id;
        page.operation = request.syscall as u32;
        page.request_length = request.length as u32;
        page.request = [0; MAX_TEXT];
        page.request[..request.length].copy_from_slice(&request.argument[..request.length]);
        page.reply_status = 0;
        page.reply_length = 0;
        page.reply = [0; MAX_TEXT];
        page.state = SessionPageState::Request.wire();
        unsafe { (address as *mut Self).write_volatile(page) };
        true
    }

    /// # Safety
    /// Core owns the transition from request to waiting.
    pub unsafe fn take_request_at(
        address: u64,
        service_generation: u32,
        endpoint_generation: u32,
    ) -> Option<SessionClientRequest> {
        let mut page = unsafe { (address as *mut Self).read_volatile() };
        if !client_identity(&page, service_generation, endpoint_generation)
            || SessionPageState::from_wire(page.state) != Some(SessionPageState::Request)
        {
            return None;
        }
        let request = decode_session_request(page.operation, page.request_length, page.request)?;
        if page.request_id == 0 {
            return None;
        }
        page.state = SessionPageState::Waiting.wire();
        unsafe { (address as *mut Self).write_volatile(page) };
        Some(SessionClientRequest { id: page.request_id, request })
    }

    /// # Safety
    /// Core may inspect the current request while coordinating a synchronous relay.
    pub unsafe fn current_request_at(
        address: u64,
        service_generation: u32,
        endpoint_generation: u32,
    ) -> Option<SessionClientRequest> {
        let mut page = unsafe { (address as *mut Self).read_volatile() };
        let state = SessionPageState::from_wire(page.state)?;
        if !client_identity(&page, service_generation, endpoint_generation)
            || page.request_id == 0
            || !matches!(state, SessionPageState::Request | SessionPageState::Waiting)
        {
            return None;
        }
        let request = decode_session_request(page.operation, page.request_length, page.request)?;
        if state == SessionPageState::Request {
            page.state = SessionPageState::Waiting.wire();
            unsafe { (address as *mut Self).write_volatile(page) };
        }
        Some(SessionClientRequest { id: page.request_id, request })
    }

    /// # Safety
    /// Core reads only scalar state and generation fields.
    pub unsafe fn pending_at(
        address: u64,
        service_generation: u32,
        endpoint_generation: u32,
    ) -> bool {
        let page = unsafe { (address as *const Self).read_volatile() };
        client_identity(&page, service_generation, endpoint_generation)
            && SessionPageState::from_wire(page.state) == Some(SessionPageState::Request)
    }

    /// # Safety
    /// Core owns client completion.
    pub unsafe fn reply_at(
        address: u64,
        service_generation: u32,
        endpoint_generation: u32,
        id: u32,
        status: SessionStatus,
        reply: logos_abi::SessionReply,
    ) -> bool {
        if !reply.valid() || reply.length > MAX_TEXT {
            return false;
        }
        let mut page = unsafe { (address as *mut Self).read_volatile() };
        if !client_identity(&page, service_generation, endpoint_generation)
            || page.request_id != id
            || SessionPageState::from_wire(page.state) != Some(SessionPageState::Waiting)
        {
            return false;
        }
        page.reply_status = status.wire();
        page.reply_length = reply.length as u32;
        page.reply = [0; MAX_TEXT];
        page.reply[..reply.length].copy_from_slice(&reply.text[..reply.length]);
        page.state = status.state().wire();
        unsafe { (address as *mut Self).write_volatile(page) };
        true
    }

    /// # Safety
    /// The mapped client service owns reply consumption.
    pub unsafe fn finish_at(
        address: u64,
        service_generation: u32,
        endpoint_generation: u32,
        id: u32,
    ) -> Option<SessionClientReply> {
        let mut page = unsafe { (address as *mut Self).read_volatile() };
        let state = SessionPageState::from_wire(page.state)?;
        if !client_identity(&page, service_generation, endpoint_generation)
            || page.request_id != id
            || !matches!(
                state,
                SessionPageState::Reply
                    | SessionPageState::Denied
                    | SessionPageState::Failed
                    | SessionPageState::Cancelled
            )
        {
            return None;
        }
        let status = SessionStatus::from_wire(page.reply_status)?;
        let reply = decode_session_reply(page.reply_length, page.reply)?;
        page = Self::new(service_generation, endpoint_generation);
        unsafe { (address as *mut Self).write_volatile(page) };
        Some(SessionClientReply { id, status, reply })
    }

    /// # Safety
    /// Core may inspect a completed reply before waking the client service.
    pub unsafe fn reply_at_current(
        address: u64,
        service_generation: u32,
        endpoint_generation: u32,
    ) -> Option<SessionClientReply> {
        let page = unsafe { (address as *const Self).read_volatile() };
        let state = SessionPageState::from_wire(page.state)?;
        if !client_identity(&page, service_generation, endpoint_generation)
            || page.request_id == 0
            || !matches!(
                state,
                SessionPageState::Reply
                    | SessionPageState::Denied
                    | SessionPageState::Failed
                    | SessionPageState::Cancelled
            )
        {
            return None;
        }
        Some(SessionClientReply {
            id: page.request_id,
            status: SessionStatus::from_wire(page.reply_status)?,
            reply: decode_session_reply(page.reply_length, page.reply)?,
        })
    }

    /// # Safety
    /// Core may cancel only the current request.
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
                SessionPageState::from_wire(page.state),
                Some(SessionPageState::Request | SessionPageState::Waiting)
            )
        {
            return false;
        }
        page.reply_status = SessionStatus::Cancelled.wire();
        page.reply_length = 0;
        page.reply = [0; MAX_TEXT];
        page.state = SessionPageState::Cancelled.wire();
        unsafe { (address as *mut Self).write_volatile(page) };
        true
    }
}

impl SessionServerPage {
    pub const fn new(service_generation: u32, endpoint_generation: u32) -> Self {
        Self {
            service_generation,
            endpoint_generation,
            state: SessionPageState::Ready.wire(),
            request_id: 0,
            caller_low: 0,
            caller_high: 0,
            operation: 0,
            request_length: 0,
            reply_status: 0,
            reply_length: 0,
            request: [0; MAX_TEXT],
            reply: [0; MAX_TEXT],
        }
    }

    /// # Safety
    /// `address` must point to a live, aligned server page mapping.
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

    /// # Safety
    /// The Sessions service owns the ready-to-waiting transition.
    pub unsafe fn wait_at(address: u64, service_generation: u32, endpoint_generation: u32) -> bool {
        let mut page = unsafe { (address as *mut Self).read_volatile() };
        if !server_identity(&page, service_generation, endpoint_generation)
            || SessionPageState::from_wire(page.state) != Some(SessionPageState::Ready)
        {
            return false;
        }
        page.state = SessionPageState::Waiting.wire();
        unsafe { (address as *mut Self).write_volatile(page) };
        true
    }

    /// # Safety
    /// Core reads only scalar state and generation fields.
    pub unsafe fn waiting_at(
        address: u64,
        service_generation: u32,
        endpoint_generation: u32,
    ) -> bool {
        let page = unsafe { (address as *const Self).read_volatile() };
        server_identity(&page, service_generation, endpoint_generation)
            && SessionPageState::from_wire(page.state) == Some(SessionPageState::Waiting)
    }

    /// # Safety
    /// Core owns delivery into a waiting server page.
    pub unsafe fn deliver_at(
        address: u64,
        service_generation: u32,
        endpoint_generation: u32,
        id: u32,
        caller: u64,
        request: logos_abi::SessionRequest,
    ) -> bool {
        if id == 0 || !request.valid() {
            return false;
        }
        let mut page = unsafe { (address as *mut Self).read_volatile() };
        if !server_identity(&page, service_generation, endpoint_generation)
            || SessionPageState::from_wire(page.state) != Some(SessionPageState::Waiting)
        {
            return false;
        }
        page.request_id = id;
        page.caller_low = caller as u32;
        page.caller_high = (caller >> 32) as u32;
        page.operation = request.syscall as u32;
        page.request_length = request.length as u32;
        page.request = [0; MAX_TEXT];
        page.request[..request.length].copy_from_slice(&request.argument[..request.length]);
        page.state = SessionPageState::Request.wire();
        unsafe { (address as *mut Self).write_volatile(page) };
        true
    }

    /// # Safety
    /// The Sessions service owns request consumption.
    pub unsafe fn take_at(
        address: u64,
        service_generation: u32,
        endpoint_generation: u32,
    ) -> Option<SessionServerRequest> {
        let mut page = unsafe { (address as *mut Self).read_volatile() };
        if !server_identity(&page, service_generation, endpoint_generation)
            || SessionPageState::from_wire(page.state) != Some(SessionPageState::Request)
            || page.request_id == 0
        {
            return None;
        }
        let request = decode_session_request(page.operation, page.request_length, page.request)?;
        page.state = SessionPageState::Processing.wire();
        unsafe { (address as *mut Self).write_volatile(page) };
        Some(SessionServerRequest {
            id: page.request_id,
            caller: u64::from(page.caller_low) | (u64::from(page.caller_high) << 32),
            request,
        })
    }

    /// # Safety
    /// The Sessions service replies only to its current processing request.
    pub unsafe fn reply_at(
        address: u64,
        service_generation: u32,
        endpoint_generation: u32,
        id: u32,
        status: SessionStatus,
        reply: logos_abi::SessionReply,
    ) -> bool {
        if !reply.valid() || reply.length > MAX_TEXT {
            return false;
        }
        let mut page = unsafe { (address as *mut Self).read_volatile() };
        if !server_identity(&page, service_generation, endpoint_generation)
            || page.request_id != id
            || SessionPageState::from_wire(page.state) != Some(SessionPageState::Processing)
        {
            return false;
        }
        page.reply_status = status.wire();
        page.reply_length = reply.length as u32;
        page.reply = [0; MAX_TEXT];
        page.reply[..reply.length].copy_from_slice(&reply.text[..reply.length]);
        page.state = status.state().wire();
        unsafe { (address as *mut Self).write_volatile(page) };
        true
    }

    /// # Safety
    /// Core owns reply consumption and deterministic reset.
    pub unsafe fn take_reply_at(
        address: u64,
        service_generation: u32,
        endpoint_generation: u32,
        id: u32,
    ) -> Option<SessionServerReply> {
        let mut page = unsafe { (address as *mut Self).read_volatile() };
        let state = SessionPageState::from_wire(page.state)?;
        if !server_identity(&page, service_generation, endpoint_generation)
            || page.request_id != id
            || !matches!(
                state,
                SessionPageState::Reply
                    | SessionPageState::Denied
                    | SessionPageState::Failed
                    | SessionPageState::Cancelled
            )
        {
            return None;
        }
        let status = SessionStatus::from_wire(page.reply_status)?;
        let reply = decode_session_reply(page.reply_length, page.reply)?;
        page = Self::new(service_generation, endpoint_generation);
        unsafe { (address as *mut Self).write_volatile(page) };
        Some(SessionServerReply { id, status, reply })
    }

    /// # Safety
    /// Core reads only scalar state and generation fields.
    pub unsafe fn reply_pending_at(
        address: u64,
        service_generation: u32,
        endpoint_generation: u32,
    ) -> bool {
        let page = unsafe { (address as *const Self).read_volatile() };
        server_identity(&page, service_generation, endpoint_generation)
            && matches!(
                SessionPageState::from_wire(page.state),
                Some(
                    SessionPageState::Reply
                        | SessionPageState::Denied
                        | SessionPageState::Failed
                        | SessionPageState::Cancelled
                )
            )
    }
}

impl EffectPage {
    pub const fn new(service_generation: u32, endpoint_generation: u32) -> Self {
        Self {
            service_generation,
            endpoint_generation,
            state: SessionPageState::Ready.wire(),
            request_id: 0,
            effect: 0,
            request_length: 0,
            result: 0,
            reply_length: 0,
            request: [0; MAX_TEXT],
            reply: [0; MAX_TEXT],
        }
    }

    /// # Safety
    /// `address` must point to a live, aligned effect page mapping.
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

    /// # Safety
    /// The Sessions service owns effect request creation.
    pub unsafe fn request_at(
        address: u64,
        service_generation: u32,
        endpoint_generation: u32,
        id: u32,
        request: logos_abi::EffectRequest,
    ) -> bool {
        if id == 0 || !request.valid() {
            return false;
        }
        let mut page = unsafe { (address as *mut Self).read_volatile() };
        if !effect_identity(&page, service_generation, endpoint_generation)
            || SessionPageState::from_wire(page.state) != Some(SessionPageState::Ready)
        {
            return false;
        }
        page.request_id = id;
        page.effect = request.effect as u32;
        page.request_length = request.length as u32;
        page.request = [0; MAX_TEXT];
        page.request[..request.length].copy_from_slice(&request.argument[..request.length]);
        page.result = 0;
        page.reply_length = 0;
        page.reply = [0; MAX_TEXT];
        page.state = SessionPageState::Request.wire();
        unsafe { (address as *mut Self).write_volatile(page) };
        true
    }

    /// # Safety
    /// Core owns effect request consumption.
    pub unsafe fn take_at(
        address: u64,
        service_generation: u32,
        endpoint_generation: u32,
    ) -> Option<EffectMessage> {
        let mut page = unsafe { (address as *mut Self).read_volatile() };
        if !effect_identity(&page, service_generation, endpoint_generation)
            || SessionPageState::from_wire(page.state) != Some(SessionPageState::Request)
            || page.request_id == 0
        {
            return None;
        }
        let request = decode_effect_request(page.effect, page.request_length, page.request)?;
        page.state = SessionPageState::Waiting.wire();
        unsafe { (address as *mut Self).write_volatile(page) };
        Some(EffectMessage { id: page.request_id, request })
    }

    /// # Safety
    /// Core reads only scalar state and generation fields.
    pub unsafe fn pending_at(
        address: u64,
        service_generation: u32,
        endpoint_generation: u32,
    ) -> bool {
        let page = unsafe { (address as *const Self).read_volatile() };
        effect_identity(&page, service_generation, endpoint_generation)
            && SessionPageState::from_wire(page.state) == Some(SessionPageState::Request)
    }

    /// # Safety
    /// Core owns effect completion after authorization and execution.
    pub unsafe fn reply_at(
        address: u64,
        service_generation: u32,
        endpoint_generation: u32,
        id: u32,
        reply: logos_abi::EffectReply,
    ) -> bool {
        if !reply.valid() || reply.length as usize > MAX_TEXT {
            return false;
        }
        let mut page = unsafe { (address as *mut Self).read_volatile() };
        if !effect_identity(&page, service_generation, endpoint_generation)
            || page.request_id != id
            || SessionPageState::from_wire(page.state) != Some(SessionPageState::Waiting)
        {
            return false;
        }
        page.result = reply.result as u32;
        page.reply_length = u32::from(reply.length);
        page.reply = [0; MAX_TEXT];
        page.reply[..usize::from(reply.length)]
            .copy_from_slice(&reply.text[..usize::from(reply.length)]);
        page.state = if reply.result == logos_abi::EffectResult::Denied {
            SessionPageState::Denied.wire()
        } else {
            SessionPageState::Reply.wire()
        };
        unsafe { (address as *mut Self).write_volatile(page) };
        true
    }

    /// # Safety
    /// The Sessions service owns result consumption.
    pub unsafe fn finish_at(
        address: u64,
        service_generation: u32,
        endpoint_generation: u32,
        id: u32,
    ) -> Option<EffectResponse> {
        let mut page = unsafe { (address as *mut Self).read_volatile() };
        let state = SessionPageState::from_wire(page.state)?;
        if !effect_identity(&page, service_generation, endpoint_generation)
            || page.request_id != id
            || !matches!(
                state,
                SessionPageState::Reply
                    | SessionPageState::Denied
                    | SessionPageState::Failed
                    | SessionPageState::Cancelled
            )
        {
            return None;
        }
        let result = logos_abi::EffectResult::from_wire(page.result)?;
        let length = usize::try_from(page.reply_length).ok()?;
        if length > MAX_TEXT || page.reply[length..].iter().any(|byte| *byte != 0) {
            return None;
        }
        let reply = logos_abi::EffectReply::new(result, &page.reply[..length]);
        page = Self::new(service_generation, endpoint_generation);
        unsafe { (address as *mut Self).write_volatile(page) };
        Some(EffectResponse { id, reply })
    }

    /// # Safety
    /// Core may recover the current ID only while an effect waits for completion.
    pub unsafe fn waiting_id_at(
        address: u64,
        service_generation: u32,
        endpoint_generation: u32,
    ) -> Option<u32> {
        let page = unsafe { (address as *const Self).read_volatile() };
        (effect_identity(&page, service_generation, endpoint_generation)
            && page.request_id != 0
            && SessionPageState::from_wire(page.state) == Some(SessionPageState::Waiting))
        .then_some(page.request_id)
    }
}
