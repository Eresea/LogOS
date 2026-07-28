// ponytail: fixed task metadata; add dynamic task storage when services exceed eight tasks.
const TASKS: usize = 8;

pub enum TaskState {
    Ready,
    Blocked(Event),
    Complete,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Event(u8);

impl Event {
    pub const VIRTIO: Self = Self(1);
    pub const INPUT: Self = Self(2);
    pub const COMMAND: Self = Self(4);
    const SELF_CHECK: Self = Self(3);
}

pub trait Runnable {
    fn run(&mut self) -> TaskState;
}

#[derive(Clone, Copy)]
pub struct TaskHandle(u32);

pub struct Task {
    entry: fn(&mut Task) -> TaskState,
    runs: usize,
}

impl Task {
    pub const fn new(entry: fn(&mut Task) -> TaskState) -> Self {
        Self { entry, runs: 0 }
    }

    pub const fn runs(&self) -> usize {
        self.runs
    }
}

impl Runnable for Task {
    fn run(&mut self) -> TaskState {
        self.runs += 1;
        (self.entry)(self)
    }
}

pub struct Scheduler<'a> {
    tasks: [Option<Entry<'a>>; TASKS],
    generations: [u16; TASKS],
    next: usize,
}

struct Entry<'a> {
    task: &'a mut dyn Runnable,
    waiting: Option<Event>,
    generation: u16,
}

impl<'a> Scheduler<'a> {
    pub const fn new() -> Self {
        Self { tasks: [const { None }; TASKS], generations: [1; TASKS], next: 0 }
    }

    pub fn spawn(&mut self, task: &'a mut dyn Runnable) -> Option<TaskHandle> {
        for (index, slot) in self.tasks.iter_mut().enumerate() {
            if slot.is_none() {
                let generation = self.generations[index];
                *slot = Some(Entry { task, waiting: None, generation });
                return Some(TaskHandle((u32::from(generation) << 16) | index as u32));
            }
        }
        None
    }

    pub fn run_next(&mut self) -> bool {
        for _ in 0..TASKS {
            let index = self.next;
            self.next = (self.next + 1) % TASKS;
            if let Some(mut entry) = self.tasks[index].take() {
                if entry.waiting.is_some() {
                    self.tasks[index] = Some(entry);
                    continue;
                }
                match entry.task.run() {
                    TaskState::Ready => self.tasks[index] = Some(entry),
                    TaskState::Blocked(event) => {
                        entry.waiting = Some(event);
                        self.tasks[index] = Some(entry);
                        crate::trace::record(crate::trace::Event::TaskBlocked);
                    }
                    TaskState::Complete => {
                        self.generations[index] = self.generations[index].wrapping_add(1);
                    }
                }
                return true;
            }
        }
        false
    }

    pub fn wake(&mut self, handle: TaskHandle) -> bool {
        let index = handle.0 as u16 as usize;
        let generation = (handle.0 >> 16) as u16;
        let Some(Some(entry)) = self.tasks.get_mut(index) else {
            return false;
        };
        if entry.waiting.is_none() || entry.generation != generation {
            return false;
        }
        entry.waiting = None;
        crate::trace::record(crate::trace::Event::TaskWoken);
        true
    }

    pub fn wake_event(&mut self, event: Event) -> usize {
        let mut woken = 0;
        for entry in self.tasks.iter_mut().flatten() {
            if entry.waiting == Some(event) {
                entry.waiting = None;
                woken += 1;
            }
        }
        if woken > 0 {
            crate::trace::record(crate::trace::Event::TaskWoken);
        }
        woken
    }
}

pub fn self_check() -> bool {
    fn block(task: &mut Task) -> TaskState {
        if task.runs() == 1 { TaskState::Blocked(Event::SELF_CHECK) } else { TaskState::Complete }
    }

    let mut first = Task::new(block);
    let mut second = Task::new(block);
    let mut third = Task::new(block);
    let mut scheduler = Scheduler::new();
    let Some(first_handle) = scheduler.spawn(&mut first) else {
        return false;
    };
    scheduler.spawn(&mut second).is_some()
        && scheduler.spawn(&mut third).is_some()
        && scheduler.run_next()
        && scheduler.run_next()
        && scheduler.run_next()
        && !scheduler.run_next()
        && scheduler.wake_event(Event::SELF_CHECK) == 3
        && !scheduler.wake(first_handle)
        && scheduler.run_next()
        && scheduler.run_next()
        && scheduler.run_next()
        && !scheduler.run_next()
}
