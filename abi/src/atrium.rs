use super::{GuiRect, GuiStatus, SurfaceHandle};

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
}

impl AtriumApp {
    pub const fn from_raw(raw: u8) -> Option<Self> {
        match raw {
            1 => Some(Self::Calculator),
            2 => Some(Self::Files),
            3 => Some(Self::Terminal),
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
    pub window_id: u16,
    pub reserved_window: u16,
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
            window_id: 0,
            reserved_window: 0,
            surface: SurfaceHandle::EMPTY,
            bounds: GuiRect::EMPTY,
            dx: 0,
            dy: 0,
        }
    }

    pub const fn is_valid(self) -> bool {
        self.request_id != 0
            && self.reserved == 0
            && self.reserved_window == 0
            && match self.operation {
                AtriumControlOperation::Section => AtriumSection::from_raw(self.section).is_some(),
                AtriumControlOperation::Launch => AtriumApp::from_raw(self.app).is_some(),
                AtriumControlOperation::Focus | AtriumControlOperation::Close => {
                    self.window_id != 0
                }
                AtriumControlOperation::Move => {
                    self.window_id != 0 && (self.dx != 0 || self.dy != 0)
                }
                AtriumControlOperation::Logout | AtriumControlOperation::Reset => {
                    self.section == 0 && self.app == 0 && self.window_id == 0
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
    pub window_id: u16,
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
            window_id: 0,
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

const _: () = assert!(core::mem::size_of::<AtriumControl>() <= super::MAX_IPC_BYTES);
const _: () = assert!(core::mem::size_of::<AtriumControlResponse>() <= super::MAX_IPC_BYTES);

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
}
