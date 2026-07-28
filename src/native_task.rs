use crate::{
    address_space::AddressSpace,
    cpu::{EntryState, Privilege},
    memory::PhysicalMemory,
    payload::Payload,
};

pub struct Terminal {
    space: AddressSpace,
    entry: u64,
    context_physical: u64,
    context: u64,
    blocked: bool,
    complete: bool,
}

impl Terminal {
    pub fn load(memory: &mut PhysicalMemory, payload: Payload) -> Option<Self> {
        let mut space = AddressSpace::new(memory)?;
        let Some(entry) = space.map_image(memory, payload) else {
            let _ = space.release(memory);
            return None;
        };
        let Some((context_physical, context)) = space.map_context(memory) else {
            let _ = space.release(memory);
            return None;
        };
        Some(Self { space, entry, context_physical, context, blocked: false, complete: false })
    }

    pub fn start(&mut self, privilege: &Privilege) -> bool {
        let state = privilege.run_entry(&mut self.space, self.entry, self.context);
        self.advance(state)
    }

    pub fn deliver_input(&self, input: u8) -> bool {
        self.blocked
            && unsafe {
                logos_core::native_service::Context::deliver_input_at(self.context_physical, input)
            }
    }

    pub fn resume(&mut self, privilege: &Privilege) -> bool {
        let state = privilege.resume_entry(&mut self.space);
        self.advance(state)
    }

    pub const fn blocked(&self) -> bool {
        self.blocked
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
