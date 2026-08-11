use core::sync::atomic::{AtomicU32, AtomicUsize, Ordering};

const VACANT: u8 = 0;
const RUNNABLE: u8 = 1;
const RUNNING: u8 = 2;
const BLOCKED: u8 = 3;
const STATE_MASK: u32 = 0x0f;
const WAKE_PENDING: u32 = 1 << 4;
const GENERATION_SHIFT: u32 = 8;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Handle {
    pub slot: usize,
    pub generation: u16,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum State {
    Vacant,
    Runnable,
    Running,
    Blocked,
}

struct Slot {
    version: AtomicU32,
}

impl Slot {
    const fn new() -> Self {
        Self { version: AtomicU32::new(pack(1, VACANT, false)) }
    }
}

pub struct Registry<const N: usize> {
    slots: [Slot; N],
    cursors: [AtomicUsize; 2],
}

unsafe impl<const N: usize> Sync for Registry<N> {}

impl<const N: usize> Registry<N> {
    pub const fn new() -> Self {
        Self { slots: [const { Slot::new() }; N], cursors: [const { AtomicUsize::new(0) }; 2] }
    }

    pub fn spawn(&self) -> Option<Handle> {
        self.slots.iter().enumerate().find_map(|(slot, entry)| {
            let current = entry.version.load(Ordering::Acquire);
            (state(current) == VACANT).then_some((slot, generation(current))).and_then(
                |(slot, generation)| {
                    entry
                        .version
                        .compare_exchange(
                            current,
                            pack(generation, RUNNABLE, false),
                            Ordering::AcqRel,
                            Ordering::Acquire,
                        )
                        .ok()
                        .map(|_| Handle { slot, generation })
                },
            )
        })
    }

    pub fn claim(&self, cpu: usize) -> Option<Handle> {
        let cursor = self.cursors.get(cpu)?;
        let start = cursor.fetch_add(1, Ordering::Relaxed) % N;
        for offset in 0..N {
            let slot = (start + offset) % N;
            let current = self.slots[slot].version.load(Ordering::Acquire);
            if state(current) != RUNNABLE {
                continue;
            }
            if self.slots[slot]
                .version
                .compare_exchange(
                    current,
                    pack(generation(current), RUNNING, false),
                    Ordering::AcqRel,
                    Ordering::Acquire,
                )
                .is_ok()
            {
                return Some(Handle { slot, generation: generation(current) });
            }
        }
        None
    }

    pub fn finish_pending(&self, handle: Handle) -> bool {
        let slot = &self.slots[handle.slot];
        loop {
            let current = slot.version.load(Ordering::Acquire);
            if generation(current) != handle.generation || state(current) != RUNNING {
                return false;
            }
            let next = if current & WAKE_PENDING != 0 {
                pack(handle.generation, RUNNABLE, false)
            } else {
                pack(handle.generation, BLOCKED, false)
            };
            if slot
                .version
                .compare_exchange(current, next, Ordering::AcqRel, Ordering::Acquire)
                .is_ok()
            {
                return state(next) == RUNNABLE;
            }
        }
    }

    pub fn wake(&self, handle: Handle) -> bool {
        let Some(slot) = self.slots.get(handle.slot) else {
            return false;
        };
        loop {
            let current = slot.version.load(Ordering::Acquire);
            if generation(current) != handle.generation {
                return false;
            }
            let next = match state(current) {
                RUNNABLE => return true,
                RUNNING if current & WAKE_PENDING == 0 => pack(handle.generation, RUNNING, true),
                BLOCKED => pack(handle.generation, RUNNABLE, false),
                _ => return false,
            };
            if slot
                .version
                .compare_exchange(current, next, Ordering::AcqRel, Ordering::Acquire)
                .is_ok()
            {
                return true;
            }
        }
    }

    pub fn complete(&self, handle: Handle) -> bool {
        let slot = &self.slots[handle.slot];
        let current = slot.version.load(Ordering::Acquire);
        if generation(current) != handle.generation || state(current) != RUNNING {
            return false;
        }
        slot.version
            .compare_exchange(
                current,
                pack(next_generation(handle.generation), VACANT, false),
                Ordering::AcqRel,
                Ordering::Acquire,
            )
            .is_ok()
    }

    pub fn state(&self, handle: Handle) -> Option<State> {
        let current = self.slots.get(handle.slot)?.version.load(Ordering::Acquire);
        (generation(current) == handle.generation).then(|| match state(current) {
            VACANT => State::Vacant,
            RUNNABLE => State::Runnable,
            RUNNING => State::Running,
            BLOCKED => State::Blocked,
            _ => State::Vacant,
        })
    }
}

const fn pack(generation: u16, state: u8, wake_pending: bool) -> u32 {
    ((generation as u32) << GENERATION_SHIFT)
        | state as u32
        | if wake_pending { WAKE_PENDING } else { 0 }
}

fn generation(version: u32) -> u16 {
    (version >> GENERATION_SHIFT) as u16
}

fn state(version: u32) -> u8 {
    (version & STATE_MASK) as u8
}

fn next_generation(generation: u16) -> u16 {
    generation.wrapping_add(1).max(1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exclusive_claim_with_two_workers() {
        let registry = Registry::<1>::new();
        let handle = registry.spawn().unwrap();
        let (left, right) = std::thread::scope(|scope| {
            let left = scope.spawn(|| registry.claim(0));
            let right = scope.spawn(|| registry.claim(1));
            (left.join().unwrap(), right.join().unwrap())
        });
        assert_eq!(left.is_some() as usize + right.is_some() as usize, 1);
        assert_eq!(left.or(right), Some(handle));
    }

    #[test]
    fn wake_during_and_after_pending_transition_is_preserved() {
        let registry = Registry::<1>::new();
        let handle = registry.spawn().unwrap();
        let running = registry.claim(0).unwrap();
        assert_eq!(registry.state(running), Some(State::Running));
        assert!(registry.wake(running));
        assert!(registry.finish_pending(running));
        assert_eq!(registry.state(handle), Some(State::Runnable));
        let running = registry.claim(1).unwrap();
        assert!(!registry.finish_pending(running));
        assert_eq!(registry.state(handle), Some(State::Blocked));
        assert!(registry.wake(handle));
        assert_eq!(registry.state(handle), Some(State::Runnable));
    }

    #[test]
    fn duplicate_cross_cpu_wake_is_idempotent() {
        let registry = Registry::<1>::new();
        let handle = registry.spawn().unwrap();
        let running = registry.claim(0).unwrap();
        assert!(!registry.finish_pending(running));
        assert!(registry.wake(handle));
        assert!(registry.wake(handle));
        assert_eq!(registry.state(handle), Some(State::Runnable));
        let claimed = registry.claim(1).unwrap();
        assert_eq!(claimed, handle);
        assert_eq!(registry.state(handle), Some(State::Running));
        assert!(registry.claim(0).is_none());
    }

    #[test]
    fn completion_reuses_capacity_and_rejects_stale_generation() {
        let registry = Registry::<1>::new();
        let old = registry.spawn().unwrap();
        assert!(registry.spawn().is_none());
        let old_running = registry.claim(0).unwrap();
        assert_eq!(old_running, old);
        assert_eq!(registry.state(old), Some(State::Running));
        assert!(registry.complete(old_running));
        assert!(registry.state(old).is_none());
        let new = registry.spawn().unwrap();
        assert_eq!(new.slot, old.slot);
        assert_ne!(old.generation, new.generation);
        assert!(!registry.wake(old));
        let running = registry.claim(0).unwrap();
        assert_eq!(running, new);
        assert!(registry.complete(running));
        assert!(!registry.complete(running));
        assert!(registry.state(new).is_none());
    }

    #[test]
    fn multiple_cpus_claim_different_tasks() {
        let registry = Registry::<2>::new();
        let first = registry.spawn().unwrap();
        let second = registry.spawn().unwrap();
        let left = registry.claim(0).unwrap();
        let right = registry.claim(1).unwrap();
        assert_ne!(left.slot, right.slot);
        assert!(registry.complete(left));
        assert!(registry.complete(right));
        assert!(registry.state(first).is_none());
        assert!(registry.state(second).is_none());
    }
}
