use crate::{
    address_space::AddressSpace,
    cpu::{EntryState, Privilege},
    memory::PhysicalMemory,
    payload::Payload,
    scheduler::{Event, Runnable, TaskState},
};

pub struct Terminal<'a> {
    privilege: &'a Privilege,
    space: AddressSpace,
    entry: u64,
    context_physical: u64,
    context: u64,
    started: bool,
    blocked: bool,
    complete: bool,
}

#[derive(Clone, Copy)]
pub struct InputEndpoint {
    context_physical: u64,
}

impl InputEndpoint {
    pub fn deliver(self, input: u8) -> bool {
        unsafe {
            logos_core::native_service::Context::deliver_input_at(self.context_physical, input)
        }
    }
}

impl<'a> Terminal<'a> {
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
            complete: false,
        })
    }

    pub fn start(&mut self) -> bool {
        let state = self.privilege.run_entry(&mut self.space, self.entry, self.context);
        self.started = true;
        self.advance(state)
    }

    pub const fn input_endpoint(&self) -> InputEndpoint {
        InputEndpoint { context_physical: self.context_physical }
    }

    pub fn resume(&mut self) -> bool {
        let state = self.privilege.resume_entry(&mut self.space);
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
            Some(EntryState::Blocked) => {
                self.blocked = true;
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

impl Runnable for Terminal<'_> {
    fn run(&mut self) -> TaskState {
        if self.complete {
            return TaskState::Complete;
        }
        if !self.started {
            return if self.start() {
                TaskState::Blocked(Event::INPUT)
            } else {
                TaskState::Complete
            };
        }
        if self.resume() {
            if self.complete { TaskState::Complete } else { TaskState::Blocked(Event::INPUT) }
        } else {
            TaskState::Complete
        }
    }
}
