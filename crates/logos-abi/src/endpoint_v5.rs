#![allow(dead_code)]

pub const MAX_ENDPOINT_SLOTS: usize = 16;
pub const ENDPOINT_VERSION: u16 = 1;

pub const KIND_INPUT: u16 = 1;
pub const KIND_DISPLAY: u16 = 2;
pub const KIND_SESSION_CLIENT: u16 = 3;
pub const KIND_SESSION_SERVER: u16 = 4;
pub const KIND_EFFECT: u16 = 5;
pub const KIND_STORE_CLIENT: u16 = 6;
pub const KIND_STORE_SERVER: u16 = 7;
pub const KIND_BLOCK_CLIENT: u16 = 8;
pub const KIND_REMOTE: u16 = 9;
pub const KIND_NETWORK_DEVICE: u16 = 10;
pub const KIND_NETWORK_EVENT: u16 = 11;
pub const KIND_NETWORK_CLIENT: u16 = 12;
pub const KIND_NETWORK_SERVER: u16 = 13;
pub const KIND_NETWORK_STREAM: u16 = 14;

pub const fn known_kind(kind: u16) -> bool {
    matches!(kind, KIND_INPUT..=KIND_NETWORK_STREAM)
}

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
        known_kind(self.kind)
            && self.version != 0
            && self.page != 0
            && self.page.is_multiple_of(4096)
            && self.generation != 0
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
        Self {
            generation: if generation == 0 { 1 } else { generation },
            count: 0,
            reserved: 0,
            slots: [EndpointSlot::EMPTY; MAX_ENDPOINT_SLOTS],
        }
    }

    pub fn insert(&mut self, slot: EndpointSlot) -> bool {
        let count = usize::from(self.count);
        if !slot.valid()
            || slot.generation != self.generation
            || count >= MAX_ENDPOINT_SLOTS
            || self.reserved != 0
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

    #[test]
    fn slots_reject_unknown_kinds_and_unaligned_pages() {
        let mut table = EndpointTable::new(1);
        assert!(!table.insert(EndpointSlot::new(99, ENDPOINT_VERSION, 0, 0x1000, 1)));
        assert!(!table.insert(EndpointSlot::new(KIND_INPUT, ENDPOINT_VERSION, 0, 0x1001, 1,)));
        assert!(table.insert(EndpointSlot::new(KIND_INPUT, ENDPOINT_VERSION, 0, 0x1000, 1,)));
    }
}
