use crate::{
    address_space::AddressSpace,
    cpu::{EntryState, Privilege},
    memory::PhysicalMemory,
    payload::Payload,
    scheduler::{Event, Runnable, TaskState},
};

pub struct Service<'a> {
    privilege: &'a Privilege,
    space: AddressSpace,
    entry: u64,
    context_physical: u64,
    context: u64,
    started: bool,
    blocked: bool,
    event: Event,
    complete: bool,
    gate: crate::cpu::GateState,
}

#[derive(Clone, Copy)]
pub struct InputEndpoint {
    context_physical: u64,
}

impl InputEndpoint {
    pub fn deliver(self, input: logos_abi::InputEvent) -> bool {
        unsafe {
            logos_core::native_service::Context::deliver_input_at(
                self.context_physical,
                input.byte(),
            )
        }
    }
}

#[derive(Clone, Copy)]
pub struct SyscallEndpoint {
    context_physical: u64,
}

#[derive(Clone, Copy)]
pub struct SessionEndpoint {
    context_physical: u64,
}

#[derive(Clone, Copy)]
pub struct DisplayEndpoint {
    context_physical: u64,
}

impl DisplayEndpoint {
    pub fn pending(self) -> bool {
        unsafe { logos_core::native_service::Context::display_waiting_at(self.context_physical) }
    }

    pub const fn context(self) -> u64 {
        self.context_physical
    }
}

impl SyscallEndpoint {
    pub fn request(self) -> Option<logos_abi::SessionRequest> {
        unsafe { logos_core::native_service::Context::syscall_at(self.context_physical) }
    }

    pub fn reply(self, bytes: &[u8]) -> bool {
        unsafe { logos_core::native_service::Context::reply_at(self.context_physical, bytes) }
    }

    #[cfg(feature = "test-hooks")]
    pub fn reply_matches(self, expected: &[u8]) -> bool {
        unsafe { logos_core::native_service::Context::response_at(self.context_physical) }
            .is_some_and(|response| response.text[..response.length] == *expected)
    }
}

impl SessionEndpoint {
    pub fn deliver(self, request: logos_abi::SessionRequest) -> bool {
        unsafe {
            logos_core::native_service::Context::deliver_session_at(self.context_physical, request)
        }
    }

    pub fn reply(self) -> Option<logos_abi::SessionReply> {
        unsafe { logos_core::native_service::Context::session_reply_at(self.context_physical) }
    }

    pub fn effect(self) -> Option<logos_abi::EffectRequest> {
        unsafe { logos_core::native_service::Context::session_effect_at(self.context_physical) }
    }

    pub fn reply_effect(self, reply: logos_abi::EffectResult) -> bool {
        unsafe {
            logos_core::native_service::Context::reply_effect_at(self.context_physical, reply)
        }
    }
}

impl<'a> Service<'a> {
    pub fn load(
        memory: &mut PhysicalMemory,
        payload: Payload,
        privilege: &'a Privilege,
    ) -> Option<Self> {
        let mut space = AddressSpace::new(memory)?;
        let Some(entry) = space.map_image(memory, payload) else {
            let _ = space.release(memory);
            return None;
        };
        let Some((context_physical, context)) = space.map_context(memory) else {
            let _ = space.release(memory);
            return None;
        };
        Some(Self {
            privilege,
            space,
            entry,
            context_physical,
            context,
            started: false,
            blocked: false,
            event: Event::INPUT,
            complete: false,
            gate: crate::cpu::GateState::new(),
        })
    }

    pub fn start(&mut self) -> bool {
        let state =
            self.privilege.run_entry(&mut self.space, self.entry, self.context, &mut self.gate);
        self.started = true;
        self.advance(state)
    }

    pub const fn input_endpoint(&self) -> InputEndpoint {
        InputEndpoint { context_physical: self.context_physical }
    }

    pub const fn syscall_endpoint(&self) -> SyscallEndpoint {
        SyscallEndpoint { context_physical: self.context_physical }
    }

    pub const fn display_endpoint(&self) -> DisplayEndpoint {
        DisplayEndpoint { context_physical: self.context_physical }
    }

    pub const fn session_endpoint(&self) -> SessionEndpoint {
        SessionEndpoint { context_physical: self.context_physical }
    }

    pub fn resume(&mut self) -> bool {
        let state = self.privilege.resume_entry(&mut self.space, self.context, &mut self.gate);
        self.advance(state)
    }

    pub const fn complete(&self) -> bool {
        self.complete
    }

    pub fn release(self, memory: &mut PhysicalMemory) -> bool {
        self.space.release(memory)
    }

    fn advance(&mut self, state: Option<EntryState>) -> bool {
        match state {
            Some(EntryState::Input) | Some(EntryState::Command) | Some(EntryState::Display) => {
                self.blocked = true;
                self.event = if state == Some(EntryState::Command) {
                    Event::COMMAND
                } else if state == Some(EntryState::Display) {
                    Event::DISPLAY
                } else {
                    Event::INPUT
                };
                true
            }
            Some(EntryState::Returned) => {
                self.blocked = false;
                self.complete = unsafe {
                    logos_core::native_service::Context::complete_at(self.context_physical)
                };
                self.complete
            }
            None => false,
        }
    }
}

impl Runnable for Service<'_> {
    fn run(&mut self) -> TaskState {
        if self.complete {
            return TaskState::Complete;
        }
        if !self.started {
            return if self.start() { TaskState::Blocked(self.event) } else { TaskState::Complete };
        }
        if self.resume() {
            if self.complete { TaskState::Complete } else { TaskState::Blocked(self.event) }
        } else {
            TaskState::Complete
        }
    }
}
