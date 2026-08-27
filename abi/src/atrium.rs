use super::{
    GuiRect, GuiStatus, InputMessage, KeyState, MAX_TEXT_BYTES, MessageKind, ServiceHandle,
    SurfaceHandle,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum AtriumSection {
    Boot = 1,
    Locked = 2,
    Home = 3,
}

impl AtriumSection {
    pub const fn from_raw(raw: u8) -> Option<Self> {
        match raw {
            1 => Some(Self::Boot),
            2 => Some(Self::Locked),
            3 => Some(Self::Home),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum AtriumApp {
    Calculator = 1,
    Files = 2,
    Terminal = 3,
    System = 4,
}

impl AtriumApp {
    pub const fn from_raw(raw: u8) -> Option<Self> {
        match raw {
            1 => Some(Self::Calculator),
            2 => Some(Self::Files),
            3 => Some(Self::Terminal),
            4 => Some(Self::System),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum AtriumControlOperation {
    Section = 1,
    Launch = 2,
    Focus = 3,
    Move = 4,
    Close = 5,
    Logout = 6,
    Reset = 7,
}

impl AtriumControlOperation {
    pub const fn from_raw(raw: u8) -> Option<Self> {
        match raw {
            1 => Some(Self::Section),
            2 => Some(Self::Launch),
            3 => Some(Self::Focus),
            4 => Some(Self::Move),
            5 => Some(Self::Close),
            6 => Some(Self::Logout),
            7 => Some(Self::Reset),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(C)]
pub struct AtriumControl {
    pub operation: AtriumControlOperation,
    pub section: u8,
    pub app: u8,
    pub reserved: u8,
    pub request_id: u32,
    pub surface_id: u16,
    pub reserved_surface: u16,
    pub surface: SurfaceHandle,
    pub bounds: GuiRect,
    pub dx: i32,
    pub dy: i32,
}

impl AtriumControl {
    pub const fn new(operation: AtriumControlOperation, request_id: u32) -> Self {
        Self {
            operation,
            section: 0,
            app: 0,
            reserved: 0,
            request_id,
            surface_id: 0,
            reserved_surface: 0,
            surface: SurfaceHandle::EMPTY,
            bounds: GuiRect::EMPTY,
            dx: 0,
            dy: 0,
        }
    }

    pub const fn is_valid(self) -> bool {
        self.request_id != 0
            && self.reserved == 0
            && self.reserved_surface == 0
            && match self.operation {
                AtriumControlOperation::Section => AtriumSection::from_raw(self.section).is_some(),
                AtriumControlOperation::Launch => AtriumApp::from_raw(self.app).is_some(),
                AtriumControlOperation::Focus | AtriumControlOperation::Close => {
                    self.surface_id != 0
                }
                AtriumControlOperation::Move => {
                    self.surface_id != 0 && (self.dx != 0 || self.dy != 0)
                }
                AtriumControlOperation::Logout | AtriumControlOperation::Reset => {
                    self.section == 0 && self.app == 0 && self.surface_id == 0
                }
            }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(C)]
pub struct AtriumControlResponse {
    pub operation: AtriumControlOperation,
    pub status: GuiStatus,
    pub reserved: u16,
    pub request_id: u32,
    pub surface_id: u16,
    pub reserved_tail: u16,
    pub surface: SurfaceHandle,
}

impl AtriumControlResponse {
    pub const fn new(request: AtriumControl, status: GuiStatus) -> Self {
        Self {
            operation: request.operation,
            status,
            reserved: 0,
            request_id: request.request_id,
            surface_id: 0,
            reserved_tail: 0,
            surface: SurfaceHandle::EMPTY,
        }
    }

    pub const fn is_valid_for(self, request: AtriumControl) -> bool {
        self.operation as u8 == request.operation as u8
            && self.request_id == request.request_id
            && self.reserved == 0
            && self.reserved_tail == 0
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum AtriumSurfaceOperation {
    Request = 1,
    Revoke = 2,
}

impl AtriumSurfaceOperation {
    pub const fn from_raw(raw: u8) -> Option<Self> {
        match raw {
            1 => Some(Self::Request),
            2 => Some(Self::Revoke),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(C)]
pub struct AtriumSurfaceRequest {
    pub operation: AtriumSurfaceOperation,
    pub app: u8,
    pub reserved: u16,
    pub request_id: u32,
    pub client: ServiceHandle,
}

impl AtriumSurfaceRequest {
    pub const fn new(app: AtriumApp, client: ServiceHandle, request_id: u32) -> Self {
        Self {
            operation: AtriumSurfaceOperation::Request,
            app: app as u8,
            reserved: 0,
            request_id,
            client,
        }
    }

    pub const fn is_valid(self) -> bool {
        self.operation as u8 == AtriumSurfaceOperation::Request as u8
            && AtriumApp::from_raw(self.app).is_some()
            && self.reserved == 0
            && self.request_id != 0
            && self.client.is_valid()
    }

    pub const fn app(self) -> Option<AtriumApp> {
        AtriumApp::from_raw(self.app)
    }

    pub const fn client(self) -> ServiceHandle {
        self.client
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(C)]
pub struct AtriumSurfaceResponse {
    pub operation: AtriumSurfaceOperation,
    pub status: GuiStatus,
    pub reserved: u16,
    pub request_id: u32,
    pub surface: SurfaceHandle,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(C)]
pub struct AtriumSurfaceInput {
    pub surface: SurfaceHandle,
    pub input: InputMessage,
}

impl AtriumSurfaceInput {
    pub const fn new(surface: SurfaceHandle, input: InputMessage) -> Self {
        Self { surface, input }
    }

    pub const fn is_valid(self) -> bool {
        if !self.surface.is_valid() {
            return false;
        }
        match self.input.kind {
            MessageKind::Key => {
                matches!(
                    self.input.state,
                    KeyState::Pressed | KeyState::Released | KeyState::Repeat
                ) && self.input.len == 0
            }
            MessageKind::Text | MessageKind::Paste => {
                matches!(self.input.state, KeyState::Pressed)
                    && self.input.len != 0
                    && self.input.len as usize <= MAX_TEXT_BYTES
            }
            MessageKind::Pointer => self.input.pointer_event().is_some(),
            _ => false,
        }
    }
}

impl AtriumSurfaceResponse {
    pub const fn new(request: AtriumSurfaceRequest, status: GuiStatus) -> Self {
        Self {
            operation: AtriumSurfaceOperation::Request,
            status,
            reserved: 0,
            request_id: request.request_id,
            surface: SurfaceHandle::EMPTY,
        }
    }

    pub const fn revoke(request_id: u32, surface: SurfaceHandle) -> Self {
        Self {
            operation: AtriumSurfaceOperation::Revoke,
            status: GuiStatus::NotFound,
            reserved: 0,
            request_id,
            surface,
        }
    }

    pub const fn is_valid_for(self, request: AtriumSurfaceRequest) -> bool {
        self.operation as u8 == AtriumSurfaceOperation::Request as u8
            && self.request_id == request.request_id
            && self.reserved == 0
    }

    pub const fn is_revoke(self) -> bool {
        self.operation as u8 == AtriumSurfaceOperation::Revoke as u8
            && self.status as u8 == GuiStatus::NotFound as u8
            && self.reserved == 0
            && self.request_id != 0
            && self.surface.is_valid()
    }
}

const _: () = assert!(core::mem::size_of::<AtriumControl>() <= super::MAX_IPC_BYTES);
const _: () = assert!(core::mem::size_of::<AtriumControlResponse>() <= super::MAX_IPC_BYTES);
const _: () = assert!(core::mem::size_of::<AtriumSurfaceRequest>() <= super::MAX_IPC_BYTES);
const _: () = assert!(core::mem::size_of::<AtriumSurfaceResponse>() <= super::MAX_IPC_BYTES);
const _: () = assert!(core::mem::size_of::<AtriumSurfaceInput>() <= super::MAX_IPC_BYTES);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn controls_validate_only_their_selected_payload() {
        let mut launch = AtriumControl::new(AtriumControlOperation::Launch, 1);
        assert!(!launch.is_valid());
        launch.app = AtriumApp::Calculator as u8;
        assert!(launch.is_valid());
        launch.reserved = 1;
        assert!(!launch.is_valid());
    }

    #[test]
    fn responses_are_correlated() {
        let request = AtriumControl::new(AtriumControlOperation::Logout, 7);
        let response = AtriumControlResponse::new(request, GuiStatus::Ok);
        assert!(response.is_valid_for(request));
        assert!(!response.is_valid_for(AtriumControl::new(AtriumControlOperation::Logout, 8)));
    }

    #[test]
    fn surface_requests_and_revocations_are_bounded_and_correlated() {
        let client = ServiceHandle::new(1, 1).unwrap();
        let request = AtriumSurfaceRequest::new(AtriumApp::Terminal, client, 7);
        assert!(request.is_valid());
        assert_eq!(request.app(), Some(AtriumApp::Terminal));
        assert_eq!(request.client(), client);
        let mut invalid = request;
        invalid.client = ServiceHandle::EMPTY;
        assert!(!invalid.is_valid());
        let response = AtriumSurfaceResponse::new(request, GuiStatus::Ok);
        assert!(response.is_valid_for(request));
        assert!(!response.is_valid_for(AtriumSurfaceRequest::new(AtriumApp::Terminal, client, 8)));
        let surface = SurfaceHandle::new(1, 1, 13).unwrap();
        assert!(AtriumSurfaceResponse::revoke(9, surface).is_revoke());
    }

    #[test]
    fn surface_input_requires_a_valid_target_and_input_shape() {
        let surface = SurfaceHandle::new(1, 2, 13).unwrap();
        let input = AtriumSurfaceInput::new(
            surface,
            InputMessage::key(crate::KeyCode::Enter, KeyState::Pressed, 0),
        );
        assert!(input.is_valid());
        let pointer = AtriumSurfaceInput::new(
            surface,
            InputMessage::pointer(4, 8, 1, crate::PointerState::Down).unwrap(),
        );
        assert!(pointer.is_valid());
        let mut malformed = input;
        malformed.surface = SurfaceHandle::EMPTY;
        assert!(!malformed.is_valid());
        malformed = input;
        malformed.input.kind = MessageKind::RenderCells;
        assert!(!malformed.is_valid());
    }
}
