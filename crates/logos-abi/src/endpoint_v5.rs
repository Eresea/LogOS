#![allow(dead_code)]

pub const MAX_ENDPOINT_SLOTS: usize = 16;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(C)]
pub struct EndpointSlot {
    pub kind: u16,
    pub version: u16,
    pub flags: u32,
    pub page: u64,
    pub generation: u32,
}

impl EndpointSlot {
    pub const EMPTY: Self = Self { kind: 0, version: 0, flags: 0, page: 0, generation: 0 };

    pub const fn new(kind: u16, version: u16, flags: u32, page: u64, generation: u32) -> Self {
        Self { kind, version, flags, page, generation }
    }

    pub const fn valid(self) -> bool {
        self.kind != 0 && self.version != 0 && self.page != 0 && self.generation != 0
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(C)]
pub struct EndpointTable {
    pub generation: u32,
    pub count: u16,
    pub reserved: u16,
    pub slots: [EndpointSlot; MAX_ENDPOINT_SLOTS],
}

impl EndpointTable {
    pub const fn new(generation: u32) -> Self {
        Self { generation, count: 0, reserved: 0, slots: [EndpointSlot::EMPTY; MAX_ENDPOINT_SLOTS] }
    }

    pub fn insert(&mut self, slot: EndpointSlot) -> bool {
        let count = usize::from(self.count);
        if !slot.valid()
            || slot.generation != self.generation
            || count >= MAX_ENDPOINT_SLOTS
            || self.slots[..count].iter().any(|current| current.kind == slot.kind)
        {
            return false;
        }
        let Some(target) = self.slots.get_mut(count) else { return false };
        *target = slot;
        self.count += 1;
        true
    }

    pub fn find(&self, kind: u16) -> Option<EndpointSlot> {
        let count = usize::from(self.count).min(MAX_ENDPOINT_SLOTS);
        self.slots[..count].iter().copied().find(|slot| slot.kind == kind)
    }

    pub fn reset(&mut self, generation: u32) {
        self.generation = generation.max(1);
        self.count = 0;
        self.slots = [EndpointSlot::EMPTY; MAX_ENDPOINT_SLOTS];
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn table_is_generation_bound_and_bounded() {
        let mut table = EndpointTable::new(3);
        let slot = EndpointSlot::new(1, 1, 0, 0x1000, 3);
        assert!(table.insert(slot));
        assert_eq!(table.find(1), Some(slot));
        assert!(!table.insert(slot));
        assert!(!table.insert(EndpointSlot::new(2, 1, 0, 0x2000, 4)));
        table.reset(5);
        assert_eq!(table.count, 0);
        assert_eq!(table.generation, 5);
    }
}
