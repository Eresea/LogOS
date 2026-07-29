#![no_std]

pub const ABI: u16 = 1;
pub const MAX_TEXT: usize = 64;
pub const MAX_SESSION_TEXT: usize = 256;

/// `foundation.session` v1 command request.  The transport stays bounded by
/// `logos_core::native_service::Context`; this is the shared wire contract.
#[derive(Clone, Copy, Eq, PartialEq)]
#[repr(u32)]
pub enum Syscall {
    Recovery = 1,
    Reboot,
    PowerOff,
    Ping,
    Tasks,
    Services,
    Drivers,
    Trace,
    Inspect,
    Restart,
    Cancel,
    SetInputLayout,
}

impl Syscall {
    pub fn from_name(name: &[u8]) -> Option<Self> {
        match name {
            b"recovery" => Some(Self::Recovery),
            b"reboot" => Some(Self::Reboot),
            b"poweroff" => Some(Self::PowerOff),
            b"ping" => Some(Self::Ping),
            b"tasks" => Some(Self::Tasks),
            b"services" => Some(Self::Services),
            b"drivers" => Some(Self::Drivers),
            b"trace" => Some(Self::Trace),
            b"inspect" => Some(Self::Inspect),
            b"restart" => Some(Self::Restart),
            b"cancel" => Some(Self::Cancel),
            _ => None,
        }
    }

    pub const fn from_wire(value: u32) -> Option<Self> {
        match value {
            1 => Some(Self::Recovery),
            2 => Some(Self::Reboot),
            3 => Some(Self::PowerOff),
            4 => Some(Self::Ping),
            5 => Some(Self::Tasks),
            6 => Some(Self::Services),
            7 => Some(Self::Drivers),
            8 => Some(Self::Trace),
            9 => Some(Self::Inspect),
            10 => Some(Self::Restart),
            11 => Some(Self::Cancel),
            12 => Some(Self::SetInputLayout),
            _ => None,
        }
    }

    pub const fn takes_argument(self) -> bool {
        matches!(self, Self::Inspect | Self::Restart | Self::Cancel | Self::SetInputLayout)
    }
}

/// Privileged or machine-state operation requested by Sessions from Core.
#[derive(Clone, Copy, Eq, PartialEq)]
#[repr(u32)]
pub enum Effect {
    EnterRecovery = 1,
    ResetMachine,
    PowerOffMachine,
    PingService,
    ReadTasks,
    ReadServices,
    ReadDrivers,
    ReadTrace,
    InspectResource,
    RestartService,
    CancelService,
    SetInputLayout,
}

impl Effect {
    pub const fn from_wire(value: u32) -> Option<Self> {
        match value {
            1 => Some(Self::EnterRecovery),
            2 => Some(Self::ResetMachine),
            3 => Some(Self::PowerOffMachine),
            4 => Some(Self::PingService),
            5 => Some(Self::ReadTasks),
            6 => Some(Self::ReadServices),
            7 => Some(Self::ReadDrivers),
            8 => Some(Self::ReadTrace),
            9 => Some(Self::InspectResource),
            10 => Some(Self::RestartService),
            11 => Some(Self::CancelService),
            12 => Some(Self::SetInputLayout),
            _ => None,
        }
    }

    pub const fn takes_argument(self) -> bool {
        matches!(
            self,
            Self::InspectResource
                | Self::RestartService
                | Self::CancelService
                | Self::SetInputLayout
        )
    }
}

/// Bounded `foundation.session` v1 request passed between native services.
#[derive(Clone, Copy)]
#[repr(C)]
pub struct SessionRequest {
    pub syscall: Syscall,
    pub argument: [u8; MAX_SESSION_TEXT],
    pub length: usize,
}

impl SessionRequest {
    pub const fn new(syscall: Syscall, argument: [u8; MAX_SESSION_TEXT], length: usize) -> Self {
        Self { syscall, argument, length }
    }

    pub fn valid(self) -> bool {
        self.length <= self.argument.len()
            && self.syscall.takes_argument() == (self.length != 0)
            && core::str::from_utf8(&self.argument[..self.length]).is_ok()
    }
}

#[derive(Clone, Copy)]
#[repr(C)]
pub struct EffectRequest {
    pub effect: Effect,
    pub argument: [u8; MAX_SESSION_TEXT],
    pub length: usize,
}

impl EffectRequest {
    pub const fn new(effect: Effect, argument: [u8; MAX_SESSION_TEXT], length: usize) -> Self {
        Self { effect, argument, length }
    }

    pub fn valid(self) -> bool {
        self.length <= self.argument.len()
            && self.effect.takes_argument() == (self.length != 0)
            && core::str::from_utf8(&self.argument[..self.length]).is_ok()
    }
}

/// Bounded `foundation.session` v1 reply returned to a terminal client.
#[derive(Clone, Copy)]
#[repr(C)]
pub struct SessionReply {
    pub text: [u8; MAX_SESSION_TEXT],
    pub length: usize,
}

impl SessionReply {
    pub fn from_bytes(bytes: &[u8]) -> Option<Self> {
        if bytes.len() > MAX_SESSION_TEXT {
            return None;
        }
        let mut text = [0; MAX_SESSION_TEXT];
        text[..bytes.len()].copy_from_slice(bytes);
        Some(Self { text, length: bytes.len() })
    }

    pub const fn valid(self) -> bool {
        self.length <= self.text.len()
    }
}

/// Typed result of one capability-gated Core effect. Sessions owns rendering
/// this value into a terminal reply.
#[derive(Clone, Copy, Eq, PartialEq)]
#[repr(u32)]
pub enum EffectResult {
    Completed = 1,
    Recovery,
    Unavailable,
    Pong,
    TasksActive,
    ServiceRunning,
    ServiceOverdue,
    DriverBound,
    TraceNone,
    TraceBoot,
    TraceTaskBlocked,
    TraceTaskWoken,
    TraceVirtioSubmit,
    TraceVirtioComplete,
    TraceDriverBound,
    TraceDriverQuiesced,
    TraceDriverRecovered,
    TraceDriverFailed,
    TraceFault,
    TraceSelfCheck,
    Inspected,
    RestartScheduled,
    CancelRequested,
    LayoutQwerty,
    LayoutAzerty,
    Denied,
    Unknown,
}

impl EffectResult {
    pub const fn from_wire(value: u32) -> Option<Self> {
        match value {
            1 => Some(Self::Completed),
            2 => Some(Self::Recovery),
            3 => Some(Self::Unavailable),
            4 => Some(Self::Pong),
            5 => Some(Self::TasksActive),
            6 => Some(Self::ServiceRunning),
            7 => Some(Self::ServiceOverdue),
            8 => Some(Self::DriverBound),
            9 => Some(Self::TraceNone),
            10 => Some(Self::TraceBoot),
            11 => Some(Self::TraceTaskBlocked),
            12 => Some(Self::TraceTaskWoken),
            13 => Some(Self::TraceVirtioSubmit),
            14 => Some(Self::TraceVirtioComplete),
            15 => Some(Self::TraceDriverBound),
            16 => Some(Self::TraceDriverQuiesced),
            17 => Some(Self::TraceDriverRecovered),
            18 => Some(Self::TraceDriverFailed),
            19 => Some(Self::TraceFault),
            20 => Some(Self::TraceSelfCheck),
            21 => Some(Self::Inspected),
            22 => Some(Self::RestartScheduled),
            23 => Some(Self::CancelRequested),
            24 => Some(Self::LayoutQwerty),
            25 => Some(Self::LayoutAzerty),
            26 => Some(Self::Denied),
            27 => Some(Self::Unknown),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Eq, PartialEq)]
#[repr(transparent)]
pub struct InputEvent(u8);

impl InputEvent {
    pub const BACKSPACE: Self = Self(0x08);
    pub const ENTER: Self = Self(b'\n');
    pub const ESCAPE: Self = Self(0x1b);

    pub const fn from_byte(byte: u8) -> Option<Self> {
        if byte == Self::BACKSPACE.0
            || byte == Self::ENTER.0
            || byte == Self::ESCAPE.0
            || (byte >= 0x20 && byte <= 0x7e)
        {
            Some(Self(byte))
        } else {
            None
        }
    }

    pub const fn byte(self) -> u8 {
        self.0
    }
}

#[derive(Clone, Copy, Eq, PartialEq)]
#[repr(u8)]
pub enum InputLayout {
    Qwerty = 1,
    Azerty = 2,
}

#[derive(Clone, Copy, Eq, PartialEq)]
#[repr(transparent)]
pub struct DisplayColor(u32);

impl DisplayColor {
    pub const BLACK: Self = Self(0);
    pub const GREEN: Self = Self(0x0000_ff00);

    pub const fn from_wire(value: u32) -> Option<Self> {
        if value & 0xff00_0000 == 0 { Some(Self(value)) } else { None }
    }

    pub const fn wire(self) -> u32 {
        self.0
    }

    pub const fn rgb(self) -> [u8; 3] {
        [self.0 as u8, (self.0 >> 8) as u8, (self.0 >> 16) as u8]
    }
}

impl InputLayout {
    pub const fn wire(self) -> u8 {
        match self {
            Self::Qwerty => b'q',
            Self::Azerty => b'a',
        }
    }

    pub const fn from_wire(value: u8) -> Option<Self> {
        match value {
            b'q' => Some(Self::Qwerty),
            b'a' => Some(Self::Azerty),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Eq, PartialEq)]
#[repr(u8)]
pub enum Service {
    Input,
    Display,
    Session,
    Terminal,
}

#[derive(Clone, Copy, Eq, PartialEq)]
#[repr(u8)]
pub enum Capability {
    Input,
    Display,
    Session,
}

#[derive(Clone, Copy, Eq, PartialEq)]
#[repr(u8)]
pub enum Lifecycle {
    Starting,
    Ready,
    Failed,
    Stopped,
}

#[derive(Clone, Copy, Eq, PartialEq)]
#[repr(u8)]
pub enum Operation {
    Ready,
    ReadInput,
    PresentText,
    ClearDisplay,
    SubmitCommand,
}

#[derive(Clone, Copy, Eq, PartialEq)]
#[repr(C)]
pub struct Message {
    pub abi: u16,
    pub service: Service,
    pub capability: Capability,
    pub operation: Operation,
    pub length: u8,
    pub text: [u8; MAX_TEXT],
}

impl Message {
    pub const fn empty(service: Service, capability: Capability, operation: Operation) -> Self {
        Self { abi: ABI, service, capability, operation, length: 0, text: [0; MAX_TEXT] }
    }

    pub fn with_text(mut self, text: &[u8]) -> Option<Self> {
        if text.len() > self.text.len() || core::str::from_utf8(text).is_err() {
            return None;
        }
        self.text[..text.len()].copy_from_slice(text);
        self.length = text.len() as u8;
        Some(self)
    }

    pub fn valid(&self) -> bool {
        self.abi == ABI && usize::from(self.length) <= self.text.len()
    }
}

pub fn self_check() -> bool {
    Message::empty(Service::Terminal, Capability::Display, Operation::PresentText)
        .with_text(b"LogOS")
        .is_some_and(|message| {
            message.valid() && &message.text[..message.length as usize] == b"LogOS"
        })
        && InputEvent::from_byte(b'a').is_some_and(|event| event.byte() == b'a')
        && InputEvent::from_byte(0).is_none()
        && DisplayColor::from_wire(DisplayColor::GREEN.wire()) == Some(DisplayColor::GREEN)
        && DisplayColor::from_wire(0xff00_0000).is_none()
        && Syscall::from_wire(Syscall::Restart as u32)
            .is_some_and(|call| call == Syscall::Restart && call.takes_argument())
        && Syscall::from_name(b"restart") == Some(Syscall::Restart)
        && Syscall::from_name(b"missing").is_none()
        && Syscall::from_wire(0).is_none()
        && Effect::from_wire(Effect::RestartService as u32) == Some(Effect::RestartService)
        && Effect::from_wire(0).is_none()
        && SessionRequest::new(Syscall::Inspect, [0; MAX_SESSION_TEXT], 1).valid()
        && !SessionRequest::new(Syscall::Reboot, [0; MAX_SESSION_TEXT], 1).valid()
        && EffectRequest::new(Effect::InspectResource, [0; MAX_SESSION_TEXT], 1).valid()
        && SessionReply::from_bytes(b"ok").is_some_and(|reply| reply.valid() && reply.length == 2)
        && EffectResult::from_wire(EffectResult::RestartScheduled as u32)
            == Some(EffectResult::RestartScheduled)
        && EffectResult::from_wire(0).is_none()
}
