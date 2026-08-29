//! Fixed ownership tracking for GPU frame resources.

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum FrameQueueError {
    Full,
    InvalidLease,
    StaleLease,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum FrameSlotState {
    Free,
    Building { token: u32, sequence: u32 },
    Submitted { token: u32, sequence: u32 },
    Presented { sequence: u32 },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct FrameLease {
    pub(crate) slot: usize,
    pub(crate) token: u32,
    pub(crate) sequence: u32,
}

pub(crate) struct FrameQueue<const SLOTS: usize> {
    states: [FrameSlotState; SLOTS],
    next_token: u32,
    active: Option<usize>,
}

impl<const SLOTS: usize> FrameQueue<SLOTS> {
    pub(crate) const fn new() -> Self {
        Self { states: [FrameSlotState::Free; SLOTS], next_token: 1, active: None }
    }

    #[cfg(target_os = "uefi")]
    pub(crate) fn present_initial(&mut self, slot: usize, sequence: u32) {
        if let Some(state) = self.states.get_mut(slot) {
            *state = FrameSlotState::Presented { sequence };
            self.active = Some(slot);
        }
    }

    pub(crate) fn acquire(&mut self, sequence: u32) -> Result<FrameLease, FrameQueueError> {
        let slot = self
            .states
            .iter()
            .position(|state| matches!(state, FrameSlotState::Free))
            .ok_or(FrameQueueError::Full)?;
        let token = self.next_token;
        self.next_token = self.next_token.wrapping_add(1).max(1);
        self.states[slot] = FrameSlotState::Building { token, sequence };
        Ok(FrameLease { slot, token, sequence })
    }

    pub(crate) fn submit(&mut self, lease: FrameLease) -> Result<(), FrameQueueError> {
        match self.states.get_mut(lease.slot) {
            Some(state)
                if *state
                    == (FrameSlotState::Building {
                        token: lease.token,
                        sequence: lease.sequence,
                    }) =>
            {
                *state = FrameSlotState::Submitted { token: lease.token, sequence: lease.sequence };
                Ok(())
            }
            Some(FrameSlotState::Building { .. }) => Err(FrameQueueError::StaleLease),
            Some(_) => Err(FrameQueueError::InvalidLease),
            None => Err(FrameQueueError::InvalidLease),
        }
    }

    pub(crate) fn complete(&mut self, lease: FrameLease) -> Result<(), FrameQueueError> {
        match self.states.get_mut(lease.slot) {
            Some(state)
                if *state
                    == (FrameSlotState::Submitted {
                        token: lease.token,
                        sequence: lease.sequence,
                    }) =>
            {
                if let Some(active) = self.active.replace(lease.slot) {
                    if active != lease.slot {
                        self.states[active] = FrameSlotState::Free;
                    }
                }
                self.states[lease.slot] = FrameSlotState::Presented { sequence: lease.sequence };
                Ok(())
            }
            Some(FrameSlotState::Submitted { .. }) => Err(FrameQueueError::StaleLease),
            Some(_) => Err(FrameQueueError::InvalidLease),
            None => Err(FrameQueueError::InvalidLease),
        }
    }

    #[cfg(test)]
    pub(crate) fn state(&self, slot: usize) -> Option<FrameSlotState> {
        self.states.get(slot).copied()
    }
}

impl<const SLOTS: usize> Default for FrameQueue<SLOTS> {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn double_buffer_reuses_only_after_completion() {
        let mut queue = FrameQueue::<2>::new();
        let first = queue.acquire(10).unwrap();
        queue.submit(first).unwrap();
        let second = queue.acquire(11).unwrap();
        assert_ne!(first.slot, second.slot);
        assert_eq!(queue.acquire(12), Err(FrameQueueError::Full));
        queue.complete(first).unwrap();
        assert_eq!(queue.state(first.slot), Some(FrameSlotState::Presented { sequence: 10 }));
        assert_eq!(
            queue.state(second.slot),
            Some(FrameSlotState::Building { token: second.token, sequence: 11 })
        );
    }

    #[test]
    fn completion_rejects_stale_token() {
        let mut queue = FrameQueue::<1>::new();
        let lease = queue.acquire(7).unwrap();
        queue.submit(lease).unwrap();
        let stale = FrameLease { token: lease.token.wrapping_add(1), ..lease };
        assert_eq!(queue.complete(stale), Err(FrameQueueError::StaleLease));
        assert_eq!(queue.complete(lease), Ok(()));
    }
}
