const TASKS: usize = 2;

pub enum TaskState {
    Ready,
    Blocked,
    Complete,
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
    blocked: bool,
    generation: u16,
}

impl<'a> Scheduler<'a> {
    pub const fn new() -> Self {
        Self { tasks: [None, None], generations: [1; TASKS], next: 0 }
    }

    pub fn spawn(&mut self, task: &'a mut dyn Runnable) -> Option<TaskHandle> {
        for (index, slot) in self.tasks.iter_mut().enumerate() {
            if slot.is_none() {
                let generation = self.generations[index];
                *slot = Some(Entry { task, blocked: false, generation });
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
                if entry.blocked {
                    self.tasks[index] = Some(entry);
                    continue;
                }
                match entry.task.run() {
                    TaskState::Ready => self.tasks[index] = Some(entry),
                    TaskState::Blocked => {
                        entry.blocked = true;
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
        if !entry.blocked || entry.generation != generation {
            return false;
        }
        entry.blocked = false;
        crate::trace::record(crate::trace::Event::TaskWoken);
        true
    }
}

pub fn self_check() -> bool {
    fn block(task: &mut Task) -> TaskState {
        if task.runs() == 1 { TaskState::Blocked } else { TaskState::Complete }
    }

    fn complete(_: &mut Task) -> TaskState {
        TaskState::Complete
    }

    let mut first = Task::new(block);
    let mut second = Task::new(complete);
    let mut third = Task::new(complete);
    let mut scheduler = Scheduler::new();
    let Some(first_handle) = scheduler.spawn(&mut first) else {
        return false;
    };
    scheduler.spawn(&mut second).is_some()
        && scheduler.spawn(&mut third).is_none()
        && scheduler.run_next()
        && scheduler.run_next()
        && !scheduler.run_next()
        && scheduler.wake(first_handle)
        && scheduler.run_next()
        && !scheduler.run_next()
}
