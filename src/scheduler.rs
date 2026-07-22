const TASKS: usize = 2;

pub enum TaskState {
    Ready,
    Blocked,
    Complete,
}

pub trait Runnable {
    fn run(&mut self) -> TaskState;
}

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
    next: usize,
}

struct Entry<'a> {
    task: &'a mut dyn Runnable,
    blocked: bool,
}

impl<'a> Scheduler<'a> {
    pub const fn new() -> Self {
        Self { tasks: [None, None], next: 0 }
    }

    pub fn spawn(&mut self, task: &'a mut dyn Runnable) -> bool {
        for slot in &mut self.tasks {
            if slot.is_none() {
                *slot = Some(Entry { task, blocked: false });
                return true;
            }
        }
        false
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
                    }
                    TaskState::Complete => {}
                }
                return true;
            }
        }
        false
    }

    pub fn wake(&mut self, index: usize) -> bool {
        let Some(Some(entry)) = self.tasks.get_mut(index) else {
            return false;
        };
        if !entry.blocked {
            return false;
        }
        entry.blocked = false;
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
    scheduler.spawn(&mut first)
        && scheduler.spawn(&mut second)
        && !scheduler.spawn(&mut third)
        && scheduler.run_next()
        && scheduler.run_next()
        && !scheduler.run_next()
        && scheduler.wake(0)
        && scheduler.run_next()
        && !scheduler.run_next()
}
