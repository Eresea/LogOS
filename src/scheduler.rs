const TASKS: usize = 2;

pub enum TaskState {
    Ready,
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
    tasks: [Option<&'a mut dyn Runnable>; TASKS],
    next: usize,
}

impl<'a> Scheduler<'a> {
    pub const fn new() -> Self {
        Self { tasks: [None, None], next: 0 }
    }

    pub fn spawn(&mut self, task: &'a mut dyn Runnable) -> bool {
        for slot in &mut self.tasks {
            if slot.is_none() {
                *slot = Some(task);
                return true;
            }
        }
        false
    }

    pub fn run_next(&mut self) -> bool {
        for _ in 0..TASKS {
            let index = self.next;
            self.next = (self.next + 1) % TASKS;
            if let Some(task) = self.tasks[index].take() {
                if matches!(task.run(), TaskState::Ready) {
                    self.tasks[index] = Some(task);
                }
                return true;
            }
        }
        false
    }
}

pub fn self_check() -> bool {
    fn complete(_: &mut Task) -> TaskState {
        TaskState::Complete
    }

    let mut first = Task::new(complete);
    let mut second = Task::new(complete);
    let mut third = Task::new(complete);
    let mut scheduler = Scheduler::new();
    scheduler.spawn(&mut first)
        && scheduler.spawn(&mut second)
        && !scheduler.spawn(&mut third)
        && scheduler.run_next()
        && scheduler.run_next()
        && !scheduler.run_next()
}
