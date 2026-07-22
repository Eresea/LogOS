const TASKS: usize = 2;

pub enum TaskState {
    Ready,
    Complete,
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

pub struct Scheduler {
    tasks: [Option<Task>; TASKS],
    next: usize,
}

impl Scheduler {
    pub const fn new() -> Self {
        Self { tasks: [None, None], next: 0 }
    }

    pub fn spawn(&mut self, task: Task) -> bool {
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
            if let Some(task) = &mut self.tasks[index] {
                task.runs += 1;
                if matches!((task.entry)(task), TaskState::Complete) {
                    self.tasks[index] = None;
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

    let mut scheduler = Scheduler::new();
    scheduler.spawn(Task::new(complete))
        && scheduler.spawn(Task::new(complete))
        && !scheduler.spawn(Task::new(complete))
        && scheduler.run_next()
        && scheduler.run_next()
        && !scheduler.run_next()
}
