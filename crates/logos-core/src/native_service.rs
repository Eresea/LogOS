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
pub const ACKNOWLEDGED: u32 = 1;

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
}

#[derive(Clone, Copy)]
pub struct TextRequest {
    pub x: u32,
    pub y: u32,
    pub color: logos_abi::DisplayColor,
    pub text: [u8; MAX_TEXT],
    pub length: usize,
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
        }
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
            || context.text_length as usize != core::mem::size_of::<logos_abi::StoreRequest>()
        {
            return None;
        }
        let request = unsafe {
            core::ptr::read_unaligned(context.text.as_ptr().cast::<logos_abi::StoreRequest>())
        };
        request.valid().then_some(request)
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
        unsafe {
            core::ptr::write_unaligned(
                context.text.as_mut_ptr().cast::<logos_abi::StoreRequest>(),
                request,
            );
        }
        context.text_length = core::mem::size_of::<logos_abi::StoreRequest>() as u32;
        context.operation = STORE_REQUEST;
        unsafe { (address as *mut Self).write_volatile(context) };
        true
    }

    /// # Safety
    /// `address` must point to a live, aligned `Context` mapping.
    pub unsafe fn reply_store_at(address: u64, status: logos_abi::PersistenceStatus) -> bool {
        if unsafe { Self::store_at(address) }.is_none() {
            return false;
        }
        let mut context = unsafe { (address as *mut Self).read_volatile() };
        context.operation = STORE_REPLY;
        context.x = status as u32;
        context.text_length = 0;
        unsafe { (address as *mut Self).write_volatile(context) };
        true
    }

    /// # Safety
    /// `address` must point to a live, aligned `Context` mapping.
    pub unsafe fn store_reply_at(address: u64) -> Option<logos_abi::PersistenceStatus> {
        let context = unsafe { (address as *const Self).read_volatile() };
        if context.abi != ABI
            || context.reserved != 0
            || context.operation != STORE_REPLY
            || context.status != ACKNOWLEDGED
        {
            return None;
        }
        match context.x {
            1 => Some(logos_abi::PersistenceStatus::Complete),
            2 => Some(logos_abi::PersistenceStatus::Invalid),
            3 => Some(logos_abi::PersistenceStatus::Denied),
            4 => Some(logos_abi::PersistenceStatus::Cancelled),
            5 => Some(logos_abi::PersistenceStatus::TimedOut),
            6 => Some(logos_abi::PersistenceStatus::Io),
            7 => Some(logos_abi::PersistenceStatus::Corrupt),
            8 => Some(logos_abi::PersistenceStatus::Recovered),
            9 => Some(logos_abi::PersistenceStatus::OutOfMemory),
            10 => Some(logos_abi::PersistenceStatus::Full),
            11 => Some(logos_abi::PersistenceStatus::NotFound),
            _ => None,
        }
    }

    /// # Safety
    /// `address` must point to a live, aligned `Context` mapping.
    pub unsafe fn reply_effect_at(address: u64, reply: logos_abi::EffectResult) -> bool {
        if unsafe { Self::session_effect_at(address) }.is_none() {
            return false;
        }
        let mut context = unsafe { (address as *mut Self).read_volatile() };
        context.x = reply as u32;
        context.text = [0; MAX_TEXT];
        context.text_length = 0;
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
    pub entry: extern "C" fn(*mut Context) -> !,
}

impl Header {
    pub const fn new(name: [u8; 16], entry: extern "C" fn(*mut Context) -> !) -> Self {
        Self { magic: MAGIC, abi: ABI, reserved: 0, name, entry }
    }

    pub fn entry_address(&self) -> usize {
        self.entry as usize
    }

    pub fn valid_for(&self, name: &[u8]) -> bool {
        self.magic == MAGIC && self.abi == ABI && self.reserved == 0 && self.name_starts_with(name)
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
        unsafe { Context::deliver_session_at((&mut syscall as *mut Context) as u64, request) };
    syscall.operation = SESSION_EFFECT;
    syscall.x = logos_abi::Effect::ReadTasks as u32;
    let effect = unsafe {
        Context::reply_effect_at(
            (&mut syscall as *mut Context) as u64,
            logos_abi::EffectResult::TasksActive,
        )
    } && syscall.x == logos_abi::EffectResult::TasksActive as u32;
    syscall.operation = SESSION_REPLY;
    syscall.text[..2].copy_from_slice(b"ok");
    syscall.text_length = 2;
    let reply = unsafe { Context::session_reply_at((&syscall as *const Context) as u64) }
        .is_some_and(|reply| reply.length == 2 && reply.text[..2] == *b"ok");
    let reset = unsafe { Context::reset_at((&mut syscall as *mut Context) as u64) }
        && syscall.abi == ABI
        && syscall.operation == 0;
    Header::new(*b"terminal\0\0\0\0\0\0\0\0", self_check_entry).valid_for(b"terminal")
        && !Header::new(*b"terminal\0\0\0\0\0\0\0\0", self_check_entry).valid_for(b"other")
        && valid
        && unknown
        && malformed
        && delivered
        && effect
        && reply
        && reset
}

extern "C" fn self_check_entry(_: *mut Context) -> ! {
    loop {
        core::hint::spin_loop();
    }
}
