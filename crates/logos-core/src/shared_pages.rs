use logos_abi::PageHandle;

const PAGES: usize = 32;

#[derive(Clone, Copy)]
struct Slot {
    address: u64,
    owner: u32,
    borrower: Option<u32>,
    generation: u16,
    active: bool,
}

pub struct SharedPages {
    slots: [Slot; PAGES],
}

impl SharedPages {
    pub const fn new() -> Self {
        const EMPTY: Slot =
            Slot { address: 0, owner: 0, borrower: None, generation: 1, active: false };
        Self { slots: [EMPTY; PAGES] }
    }

    pub fn register(&mut self, owner: u32, address: u64, quota: usize) -> Option<PageHandle> {
        if address == 0
            || !address.is_multiple_of(logos_abi::PAGE_SIZE as u64)
            || self.slots.iter().filter(|slot| slot.active && slot.owner == owner).count() >= quota
        {
            return None;
        }
        self.slots.iter_mut().enumerate().find(|(_, slot)| !slot.active).map(|(index, slot)| {
            slot.address = address;
            slot.owner = owner;
            slot.borrower = None;
            slot.active = true;
            PageHandle((u32::from(slot.generation) << 16) | index as u32)
        })
    }

    pub fn lend(&mut self, owner: u32, handle: PageHandle, borrower: u32) -> bool {
        let Some(slot) = self.slot_mut(handle) else {
            return false;
        };
        if slot.owner != owner || slot.borrower.is_some() || borrower == owner {
            return false;
        }
        slot.borrower = Some(borrower);
        true
    }

    pub fn return_loan(&mut self, borrower: u32, handle: PageHandle) -> bool {
        let Some(slot) = self.slot_mut(handle) else {
            return false;
        };
        if slot.borrower != Some(borrower) {
            return false;
        }
        slot.borrower = None;
        true
    }

    pub fn address(&self, principal: u32, handle: PageHandle) -> Option<u64> {
        let slot = self.slot(handle)?;
        (slot.owner == principal || slot.borrower == Some(principal)).then_some(slot.address)
    }

    pub fn release(&mut self, owner: u32, handle: PageHandle) -> Option<u64> {
        let slot = self.slot_mut(handle)?;
        if slot.owner != owner || slot.borrower.is_some() {
            return None;
        }
        let address = slot.address;
        slot.active = false;
        slot.generation = slot.generation.wrapping_add(1).max(1);
        Some(address)
    }

    pub fn reclaim(&mut self, owner: u32, mut release: impl FnMut(u64)) -> usize {
        let mut reclaimed = 0;
        for slot in &mut self.slots {
            if slot.active && slot.owner == owner {
                release(slot.address);
                slot.active = false;
                slot.borrower = None;
                slot.generation = slot.generation.wrapping_add(1).max(1);
                reclaimed += 1;
            } else if slot.active && slot.borrower == Some(owner) {
                slot.borrower = None;
            }
        }
        reclaimed
    }

    fn slot(&self, handle: PageHandle) -> Option<&Slot> {
        let slot = self.slots.get(handle.0 as u16 as usize)?;
        (slot.active && slot.generation == (handle.0 >> 16) as u16).then_some(slot)
    }

    fn slot_mut(&mut self, handle: PageHandle) -> Option<&mut Slot> {
        let slot = self.slots.get_mut(handle.0 as u16 as usize)?;
        (slot.active && slot.generation == (handle.0 >> 16) as u16).then_some(slot)
    }
}

impl Default for SharedPages {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pages_are_scoped_quota_bound_and_generation_tagged() {
        let mut pages = SharedPages::new();
        let page = pages.register(1, 0x1000, 1).unwrap();
        assert!(pages.register(1, 0x2000, 1).is_none());
        assert_eq!(pages.address(2, page), None);
        assert!(pages.lend(1, page, 2));
        assert_eq!(pages.address(2, page), Some(0x1000));
        assert!(pages.release(1, page).is_none());
        assert!(pages.return_loan(2, page));
        assert_eq!(pages.release(1, page), Some(0x1000));
        assert_eq!(pages.address(1, page), None);
    }

    #[test]
    fn owner_exit_reclaims_owned_pages_and_loans() {
        let mut pages = SharedPages::new();
        let owned = pages.register(1, 0x1000, 2).unwrap();
        let loaned = pages.register(2, 0x2000, 1).unwrap();
        assert!(pages.lend(2, loaned, 1));
        let mut address = 0;
        assert_eq!(pages.reclaim(1, |page| address = page), 1);
        assert_eq!(address, 0x1000);
        assert_eq!(pages.address(1, owned), None);
        assert_eq!(pages.address(2, loaned), Some(0x2000));
    }
}
