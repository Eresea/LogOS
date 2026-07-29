const CAPABILITIES: usize = 8;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CapabilityKind {
    Input,
    Debug,
    Service,
    Recovery,
    Secret,
    Inference,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
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
        self.slots.iter_mut().enumerate().find(|(_, slot)| !slot.active).map(|(index, slot)| {
            slot.kind = kind;
            slot.active = true;
            Capability((u32::from(slot.generation) << 16) | index as u32)
        })
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
        let Some(slot) = self.slots.get_mut(index) else { return false };
        if !slot.active || slot.generation != generation {
            return false;
        }
        slot.active = false;
        slot.generation = slot.generation.wrapping_add(1);
        true
    }
}

impl Default for CapabilityManager {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grants_are_bounded_typed_and_revocable() {
        let mut manager = CapabilityManager::new();
        let first = manager.grant(CapabilityKind::Service).unwrap();
        assert!(manager.allows(first, CapabilityKind::Service));
        assert!(!manager.allows(first, CapabilityKind::Debug));
        assert!(manager.revoke(first));
        assert!(!manager.allows(first, CapabilityKind::Service));
        for _ in 0..CAPABILITIES {
            assert!(manager.grant(CapabilityKind::Debug).is_some());
        }
        assert!(manager.grant(CapabilityKind::Debug).is_none());
    }
}
