const CAPABILITIES: usize = 8;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum CapabilityKind {
    Debug,
    Service,
    Recovery,
    Secret,
    Inference,
}

#[derive(Clone, Copy)]
pub struct Capability(u32);

pub struct CapabilityManager {
    slots: [Slot; CAPABILITIES],
}

#[derive(Clone, Copy)]
struct Slot {
    kind: CapabilityKind,
    generation: u16,
    active: bool,
}

impl CapabilityManager {
    pub const fn new() -> Self {
        const EMPTY: Slot = Slot { kind: CapabilityKind::Debug, generation: 1, active: false };
        Self { slots: [EMPTY; CAPABILITIES] }
    }

    pub fn grant(&mut self, kind: CapabilityKind) -> Option<Capability> {
        for (index, slot) in self.slots.iter_mut().enumerate() {
            if !slot.active {
                slot.kind = kind;
                slot.active = true;
                return Some(Capability((u32::from(slot.generation) << 16) | index as u32));
            }
        }
        None
    }

    pub fn allows(&self, capability: Capability, kind: CapabilityKind) -> bool {
        let index = capability.0 as u16 as usize;
        let generation = (capability.0 >> 16) as u16;
        self.slots
            .get(index)
            .is_some_and(|slot| slot.active && slot.kind == kind && slot.generation == generation)
    }

    pub fn revoke(&mut self, capability: Capability) -> bool {
        let index = capability.0 as u16 as usize;
        let generation = (capability.0 >> 16) as u16;
        let Some(slot) = self.slots.get_mut(index) else {
            return false;
        };
        if !slot.active || slot.generation != generation {
            return false;
        }
        slot.active = false;
        slot.generation = slot.generation.wrapping_add(1);
        true
    }
}
