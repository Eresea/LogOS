#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ResourceHandle {
    pub slot: u16,
    pub generation: u32,
}

#[derive(Clone, Copy)]
struct ResourceSlot {
    owner: u64,
    generation: u32,
    active: bool,
}

pub struct ResourcePool<const N: usize> {
    slots: [ResourceSlot; N],
}

impl<const N: usize> ResourcePool<N> {
    pub const fn new() -> Self {
        const EMPTY: ResourceSlot = ResourceSlot { owner: 0, generation: 1, active: false };
        Self { slots: [EMPTY; N] }
    }

    pub fn acquire(&mut self, owner: u64) -> Option<ResourceHandle> {
        if owner == 0 {
            return None;
        }
        let (index, slot) = self.slots.iter_mut().enumerate().find(|(_, slot)| !slot.active)?;
        slot.owner = owner;
        slot.active = true;
        Some(ResourceHandle { slot: index as u16, generation: slot.generation })
    }

    pub fn owns(&self, owner: u64, handle: ResourceHandle) -> bool {
        self.slots.get(handle.slot as usize).is_some_and(|slot| {
            slot.active && slot.owner == owner && slot.generation == handle.generation
        })
    }

    pub fn release(&mut self, owner: u64, handle: ResourceHandle) -> bool {
        let Some(slot) = self.slots.get_mut(handle.slot as usize) else { return false };
        if !slot.active || slot.owner != owner || slot.generation != handle.generation {
            return false;
        }
        slot.active = false;
        slot.owner = 0;
        slot.generation = slot.generation.wrapping_add(1).max(1);
        true
    }

    pub fn reclaim(&mut self, owner: u64) -> usize {
        let mut count = 0;
        for slot in &mut self.slots {
            if slot.active && slot.owner == owner {
                slot.active = false;
                slot.owner = 0;
                slot.generation = slot.generation.wrapping_add(1).max(1);
                count += 1;
            }
        }
        count
    }
}

impl<const N: usize> Default for ResourcePool<N> {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn leases_are_bounded_owner_and_generation_scoped() {
        let mut pool = ResourcePool::<1>::new();
        let handle = pool.acquire(7).unwrap();
        assert!(pool.owns(7, handle));
        assert!(pool.acquire(8).is_none());
        assert!(!pool.release(8, handle));
        assert!(pool.release(7, handle));
        let replacement = pool.acquire(7).unwrap();
        assert_ne!(replacement.generation, handle.generation);
        assert!(!pool.owns(7, handle));
    }
}
