#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PollResult {
    Pending,
    Ready,
    Cancelled,
}

pub trait PollTask {
    fn poll(&mut self) -> PollResult;
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TaskHandle {
    pub slot: u16,
    pub generation: u32,
}

struct Slot<T> {
    task: Option<T>,
    generation: u32,
}

pub struct PollRuntime<T, const N: usize> {
    slots: [Slot<T>; N],
}

impl<T, const N: usize> PollRuntime<T, N> {
    pub const fn new() -> Self {
        Self { slots: [const { Slot { task: None, generation: 1 } }; N] }
    }

    pub fn spawn(&mut self, task: T) -> Option<TaskHandle> {
        let (index, slot) =
            self.slots.iter_mut().enumerate().find(|(_, slot)| slot.task.is_none())?;
        slot.task = Some(task);
        Some(TaskHandle { slot: index as u16, generation: slot.generation })
    }

    pub fn cancel(&mut self, handle: TaskHandle) -> bool {
        let Some(slot) = self.slots.get_mut(handle.slot as usize) else { return false };
        if slot.generation != handle.generation || slot.task.is_none() {
            return false;
        }
        slot.task = None;
        slot.generation = slot.generation.wrapping_add(1).max(1);
        true
    }

    pub fn poll_once(&mut self) -> usize
    where
        T: PollTask,
    {
        let mut progress = 0;
        for slot in &mut self.slots {
            let Some(task) = slot.task.as_mut() else { continue };
            match task.poll() {
                PollResult::Pending => {}
                PollResult::Ready | PollResult::Cancelled => {
                    slot.task = None;
                    slot.generation = slot.generation.wrapping_add(1).max(1);
                    progress += 1;
                }
            }
        }
        progress
    }
}

impl<T, const N: usize> Default for PollRuntime<T, N> {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Default)]
    struct StepTask(u8);

    impl PollTask for StepTask {
        fn poll(&mut self) -> PollResult {
            self.0 += 1;
            if self.0 == 1 { PollResult::Pending } else { PollResult::Ready }
        }
    }

    #[test]
    fn runtime_is_bounded_and_generation_safe() {
        let mut runtime = PollRuntime::<StepTask, 1>::new();
        let handle = runtime.spawn(StepTask::default()).unwrap();
        assert_eq!(runtime.poll_once(), 0);
        assert_eq!(runtime.poll_once(), 1);
        assert!(!runtime.cancel(handle));
    }
}
