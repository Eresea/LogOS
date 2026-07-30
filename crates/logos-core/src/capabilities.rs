const CAPABILITIES: usize = 16;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CapabilityKind {
    Input,
    Display,
    Session,
    Debug,
    Service,
    Recovery,
    Secret,
    Inference,
    Memory,
    Block,
    StoreRead,
    StoreWrite,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Capability(u32);

pub struct CapabilityManager {
    slots: [Slot; CAPABILITIES],
}

#[derive(Clone, Copy)]
struct Slot {
    kind: CapabilityKind,
    resource: u32,
    generation: u16,
    active: bool,
}

impl CapabilityManager {
    pub const fn new() -> Self {
        const EMPTY: Slot =
            Slot { kind: CapabilityKind::Debug, resource: 0, generation: 1, active: false };
        Self { slots: [EMPTY; CAPABILITIES] }
    }

    pub fn grant(&mut self, kind: CapabilityKind) -> Option<Capability> {
        self.grant_scoped(kind, 0)
    }

    pub fn grant_scoped(&mut self, kind: CapabilityKind, resource: u32) -> Option<Capability> {
        self.slots.iter_mut().enumerate().find(|(_, slot)| !slot.active).map(|(index, slot)| {
            slot.kind = kind;
            slot.resource = resource;
            slot.active = true;
            Capability((u32::from(slot.generation) << 16) | index as u32)
        })
    }

    pub fn allows(&self, capability: Capability, kind: CapabilityKind) -> bool {
        self.allows_scoped(capability, kind, 0)
    }

    pub fn allows_scoped(
        &self,
        capability: Capability,
        kind: CapabilityKind,
        resource: u32,
    ) -> bool {
        let index = capability.0 as u16 as usize;
        let generation = (capability.0 >> 16) as u16;
        self.slots.get(index).is_some_and(|slot| {
            slot.active
                && slot.kind == kind
                && slot.resource == resource
                && slot.generation == generation
        })
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
        let scoped = manager.grant_scoped(CapabilityKind::StoreRead, 7).unwrap();
        assert!(manager.allows_scoped(scoped, CapabilityKind::StoreRead, 7));
        assert!(!manager.allows_scoped(scoped, CapabilityKind::StoreRead, 8));
        for _ in 0..CAPABILITIES - 1 {
            assert!(manager.grant(CapabilityKind::Debug).is_some());
        }
        assert!(manager.grant(CapabilityKind::Debug).is_none());
    }
}
