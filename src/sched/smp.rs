use core::{
    cell::UnsafeCell,
    future::Future,
    pin::Pin,
    sync::atomic::{AtomicBool, AtomicPtr, AtomicU8, AtomicU32, AtomicUsize, Ordering, fence},
    task::{Context, Poll, RawWaker, RawWakerVTable, Waker},
};

use super::scheduler::{Event, Runnable, TaskState};

pub const MAX_SMP_TASKS: usize = 8;
pub const MAX_ASYNC_TASKS: usize = 8;
pub const MAX_WAKER_TOKENS: usize = MAX_ASYNC_TASKS * 2;
pub const MAX_CPUS: usize = crate::arch::acpi::MAX_CPUS;

const VACANT: u8 = 0;
const RUNNABLE: u8 = 1;
const RUNNING: u8 = 2;
const BLOCKED: u8 = 3;
const FAILED: u8 = 4;
const STATE_MASK: u32 = 0x0f;
const WAKE_PENDING: u32 = 1 << 4;
const GENERATION_SHIFT: u32 = 8;
const GENERATION_MASK: u32 = 0xffff;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SchedulingState {
    Vacant,
    Runnable,
    Running,
    Blocked,
    Failed,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SpawnError {
    Full,
    WakerCapacity,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TaskHandle(u32);

impl TaskHandle {
    pub const fn slot(self) -> usize {
        (self.0 & 0xffff) as usize
    }

    pub const fn generation(self) -> u16 {
        (self.0 >> 16) as u16
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CpuSnapshot {
    pub apic_id: u32,
    pub active: bool,
    pub idle: bool,
    pub cursor: usize,
}

struct CpuState {
    apic_id: AtomicU32,
    active: AtomicBool,
    cursor: AtomicUsize,
    idle: AtomicBool,
    reschedule: AtomicBool,
}

impl CpuState {
    const fn new() -> Self {
        Self {
            apic_id: AtomicU32::new(0),
            active: AtomicBool::new(false),
            cursor: AtomicUsize::new(0),
            idle: AtomicBool::new(true),
            reschedule: AtomicBool::new(false),
        }
    }
}

enum Entry<'a> {
    Runnable(&'a mut dyn Runnable),
    Future(Pin<&'a mut (dyn Future<Output = ()> + Send)>),
}

struct Slot<'a> {
    task: UnsafeCell<Option<Entry<'a>>>,
    version: AtomicU32,
    waiting: AtomicU8,
    token: AtomicU8,
    last_cpu: AtomicU8,
}

impl<'a> Slot<'a> {
    const fn new() -> Self {
        Self {
            task: UnsafeCell::new(None),
            version: AtomicU32::new(pack(1, VACANT, false)),
            waiting: AtomicU8::new(0),
            token: AtomicU8::new(0),
            last_cpu: AtomicU8::new(0),
        }
    }
}

struct WakerToken {
    references: AtomicUsize,
    scheduler: AtomicPtr<()>,
    handle: AtomicU32,
}

impl WakerToken {
    const fn new() -> Self {
        Self {
            references: AtomicUsize::new(0),
            scheduler: AtomicPtr::new(core::ptr::null_mut()),
            handle: AtomicU32::new(0),
        }
    }

    fn reserve(&self, scheduler: *mut (), handle: TaskHandle) -> bool {
        if self.references.compare_exchange(0, 1, Ordering::AcqRel, Ordering::Acquire).is_err() {
            return false;
        }
        self.scheduler.store(scheduler, Ordering::Relaxed);
        self.handle.store(handle.0, Ordering::Relaxed);
        true
    }

    fn acquire(&self) {
        debug_assert!(self.references.load(Ordering::Acquire) != 0);
        self.references.fetch_add(1, Ordering::Relaxed);
    }

    fn release(&self) {
        let previous = self.references.fetch_sub(1, Ordering::Release);
        debug_assert!(previous != 0);
        if previous == 1 {
            fence(Ordering::Acquire);
        }
    }
}

pub struct SmpScheduler<'a> {
    tasks: [Slot<'a>; MAX_SMP_TASKS],
    cpus: [CpuState; MAX_CPUS],
    wakers: [WakerToken; MAX_WAKER_TOKENS],
    notify: fn(u32) -> bool,
}

// Safety: a task slot is claimed by a generation/state CAS before its mutable
// entry is accessed. Callers must not move or mutably borrow the scheduler after
// publishing shared access to it; this is the fixed-storage equivalent of an
// executor's stable allocation requirement.
unsafe impl Sync for SmpScheduler<'_> {}

impl<'a> SmpScheduler<'a> {
    pub fn new() -> Self {
        let scheduler = Self {
            tasks: [const { Slot::new() }; MAX_SMP_TASKS],
            cpus: [const { CpuState::new() }; MAX_CPUS],
            wakers: [const { WakerToken::new() }; MAX_WAKER_TOKENS],
            notify: default_notify,
        };
        scheduler.cpus[0].active.store(true, Ordering::Release);
        scheduler
    }

    #[allow(dead_code)]
    pub fn with_notifier(mut self, notify: fn(u32) -> bool) -> Self {
        self.notify = notify;
        self
    }

    pub fn configure_cpu(&mut self, index: usize, apic_id: u32, active: bool) -> bool {
        let Some(cpu) = self.cpus.get(index) else {
            return false;
        };
        cpu.apic_id.store(apic_id, Ordering::Release);
        cpu.active.store(active, Ordering::Release);
        cpu.idle.store(true, Ordering::Release);
        cpu.reschedule.store(false, Ordering::Release);
        true
    }

    pub fn configure_topology(&mut self, topology: &crate::arch::acpi::CpuTopology) -> usize {
        for cpu in &self.cpus {
            cpu.active.store(false, Ordering::Release);
            cpu.reschedule.store(false, Ordering::Release);
        }
        let mut next = 0;
        if let Some(bsp) = topology.bsp() {
            let _ = self.configure_cpu(0, bsp.apic_id, bsp.usable());
            next = 1;
        }
        for index in 0..topology.count() {
            let Some(cpu) = topology.get(index) else {
                continue;
            };
            if !cpu.usable() || cpu.bsp || next == MAX_CPUS {
                continue;
            }
            let _ = self.configure_cpu(next, cpu.apic_id, true);
            next += 1;
        }
        if next == 0 {
            let _ = self.configure_cpu(0, 0, true);
            1
        } else {
            next
        }
    }

    pub fn cpu(&self, index: usize) -> Option<CpuSnapshot> {
        let cpu = self.cpus.get(index)?;
        Some(CpuSnapshot {
            apic_id: cpu.apic_id.load(Ordering::Acquire),
            active: cpu.active.load(Ordering::Acquire),
            idle: cpu.idle.load(Ordering::Acquire),
            cursor: cpu.cursor.load(Ordering::Acquire),
        })
    }

    #[allow(dead_code)]
    pub fn spawn_runnable(&mut self, task: &'a mut dyn Runnable) -> Result<TaskHandle, SpawnError> {
        self.spawn_entry(Entry::Runnable(task), None)
    }

    pub fn spawn_future<F>(&mut self, future: &'a mut F) -> Result<TaskHandle, SpawnError>
    where
        F: Future<Output = ()> + Send + Unpin,
    {
        let future: Pin<&'a mut F> = Pin::new(future);
        self.spawn_pinned_future(future)
    }

    pub fn spawn_pinned_future<F>(
        &mut self,
        future: Pin<&'a mut F>,
    ) -> Result<TaskHandle, SpawnError>
    where
        F: Future<Output = ()> + Send,
    {
        let future: Pin<&'a mut (dyn Future<Output = ()> + Send)> = future;
        self.spawn_entry(Entry::Future(future), Some(()))
    }

    fn spawn_entry(
        &mut self,
        entry: Entry<'a>,
        future: Option<()>,
    ) -> Result<TaskHandle, SpawnError> {
        let Some((index, generation)) = self.free_slot() else {
            return Err(SpawnError::Full);
        };
        let handle = TaskHandle((u32::from(generation) << 16) | index as u32);
        let token = if future.is_some() {
            self.reserve_waker(handle).ok_or(SpawnError::WakerCapacity)?
        } else {
            usize::MAX
        };
        let slot = &self.tasks[index];
        unsafe { *slot.task.get() = Some(entry) };
        slot.waiting.store(0, Ordering::Release);
        slot.last_cpu.store(0, Ordering::Release);
        slot.token.store(token.wrapping_add(1) as u8, Ordering::Release);
        slot.version.store(pack(generation, RUNNABLE, false), Ordering::Release);
        Ok(handle)
    }

    fn free_slot(&self) -> Option<(usize, u16)> {
        self.tasks.iter().enumerate().find_map(|(index, slot)| {
            let word = slot.version.load(Ordering::Acquire);
            (state(word) == VACANT).then_some((index, generation(word)))
        })
    }

    fn reserve_waker(&self, handle: TaskHandle) -> Option<usize> {
        let scheduler = self as *const Self as *mut ();
        self.wakers
            .iter()
            .enumerate()
            .find_map(|(index, token)| token.reserve(scheduler, handle).then_some(index))
    }

    pub fn run_next(&self, cpu: usize) -> bool {
        let Some(cpu_state) = self.cpus.get(cpu) else {
            return false;
        };
        if !cpu_state.active.load(Ordering::Acquire) {
            return false;
        }
        if crate::arch::interrupts::take_scheduler_notification() {
            cpu_state.reschedule.store(true, Ordering::Release);
        }
        cpu_state.reschedule.swap(false, Ordering::Acquire);
        let start = cpu_state.cursor.fetch_add(1, Ordering::Relaxed) % MAX_SMP_TASKS;
        for offset in 0..MAX_SMP_TASKS {
            let index = (start + offset) % MAX_SMP_TASKS;
            let slot = &self.tasks[index];
            let current = slot.version.load(Ordering::Acquire);
            if state(current) != RUNNABLE {
                continue;
            }
            let claimed = pack(generation(current), RUNNING, false);
            if slot
                .version
                .compare_exchange(current, claimed, Ordering::AcqRel, Ordering::Acquire)
                .is_ok()
            {
                slot.last_cpu.store(cpu as u8, Ordering::Release);
                cpu_state.idle.store(false, Ordering::Release);
                let ran = self.run_claimed(index);
                cpu_state.idle.store(true, Ordering::Release);
                return ran;
            }
        }
        cpu_state.idle.store(true, Ordering::Release);
        false
    }

    #[allow(dead_code)]
    pub fn run(&self, cpu: usize, handle: TaskHandle) -> bool {
        let Some(cpu_state) = self.cpus.get(cpu) else {
            return false;
        };
        if !cpu_state.active.load(Ordering::Acquire) {
            return false;
        }
        let Some(slot) = self.tasks.get(handle.slot()) else {
            return false;
        };
        let current = slot.version.load(Ordering::Acquire);
        if generation(current) != handle.generation() || state(current) != RUNNABLE {
            return false;
        }
        if slot
            .version
            .compare_exchange(
                current,
                pack(handle.generation(), RUNNING, false),
                Ordering::AcqRel,
                Ordering::Acquire,
            )
            .is_err()
        {
            return false;
        }
        slot.last_cpu.store(cpu as u8, Ordering::Release);
        cpu_state.idle.store(false, Ordering::Release);
        let ran = self.run_claimed(handle.slot());
        cpu_state.idle.store(true, Ordering::Release);
        ran
    }

    fn run_claimed(&self, index: usize) -> bool {
        let slot = &self.tasks[index];
        let outcome = unsafe {
            let Some(entry) = (*slot.task.get()).as_mut() else {
                return false;
            };
            match entry {
                Entry::Runnable(task) => PollOutcome::Runnable(task.run()),
                Entry::Future(future) => {
                    let Some(waker) = self.waker(index) else {
                        return false;
                    };
                    let mut context = Context::from_waker(&waker);
                    PollOutcome::Future(future.as_mut().poll(&mut context))
                }
            }
        };
        match outcome {
            PollOutcome::Runnable(TaskState::Ready) => {
                slot.waiting.store(0, Ordering::Release);
                self.to_runnable(index);
            }
            PollOutcome::Runnable(TaskState::Blocked(event)) => {
                slot.waiting.store(event.bits(), Ordering::Release);
                self.finish_pending(index);
            }
            PollOutcome::Runnable(TaskState::Complete) | PollOutcome::Future(Poll::Ready(())) => {
                self.complete(index);
            }
            PollOutcome::Runnable(TaskState::Failed) => {
                slot.waiting.store(Event::FAILURE.bits(), Ordering::Release);
                self.to_failed(index);
            }
            PollOutcome::Future(Poll::Pending) => {
                slot.waiting.store(0, Ordering::Release);
                self.finish_pending(index);
            }
        }
        true
    }

    fn waker(&self, index: usize) -> Option<Waker> {
        let token = self.tasks[index].token.load(Ordering::Acquire);
        let token =
            token.checked_sub(1).map(usize::from).and_then(|index| self.wakers.get(index))?;
        token.acquire();
        let raw = RawWaker::new(token as *const WakerToken as *const (), &WAKER_VTABLE);
        Some(unsafe { Waker::from_raw(raw) })
    }

    fn finish_pending(&self, index: usize) {
        let slot = &self.tasks[index];
        loop {
            let current = slot.version.load(Ordering::Acquire);
            let current_generation = generation(current);
            if state(current) != RUNNING {
                return;
            }
            let next = if current & WAKE_PENDING != 0 {
                pack(current_generation, RUNNABLE, false)
            } else {
                pack(current_generation, BLOCKED, false)
            };
            if slot
                .version
                .compare_exchange(current, next, Ordering::AcqRel, Ordering::Acquire)
                .is_err()
            {
                continue;
            }
            if state(next) == BLOCKED {
                let after = slot.version.load(Ordering::Acquire);
                if generation(after) == current_generation && after & WAKE_PENDING != 0 {
                    let _ = slot.version.compare_exchange(
                        after,
                        pack(current_generation, RUNNABLE, false),
                        Ordering::AcqRel,
                        Ordering::Acquire,
                    );
                    self.notify_task(index);
                }
            } else {
                self.notify_task(index);
            }
            return;
        }
    }

    fn to_runnable(&self, index: usize) {
        let slot = &self.tasks[index];
        loop {
            let current = slot.version.load(Ordering::Acquire);
            if state(current) != RUNNING {
                return;
            }
            let next = pack(generation(current), RUNNABLE, false);
            if slot
                .version
                .compare_exchange(current, next, Ordering::AcqRel, Ordering::Acquire)
                .is_ok()
            {
                return;
            }
        }
    }

    fn to_failed(&self, index: usize) {
        let slot = &self.tasks[index];
        loop {
            let current = slot.version.load(Ordering::Acquire);
            if state(current) != RUNNING {
                return;
            }
            if slot
                .version
                .compare_exchange(
                    current,
                    pack(generation(current), FAILED, false),
                    Ordering::AcqRel,
                    Ordering::Acquire,
                )
                .is_ok()
            {
                return;
            }
        }
    }

    fn complete(&self, index: usize) {
        let slot = &self.tasks[index];
        slot.waiting.store(0, Ordering::Release);
        let token = slot.token.swap(0, Ordering::AcqRel);
        unsafe { (*slot.task.get()).take() };
        loop {
            let current = slot.version.load(Ordering::Acquire);
            if state(current) != RUNNING {
                break;
            }
            let next_generation = next_generation(generation(current));
            let next = pack(next_generation, VACANT, false);
            if slot
                .version
                .compare_exchange(current, next, Ordering::AcqRel, Ordering::Acquire)
                .is_ok()
            {
                break;
            }
        }
        if token != 0 {
            self.wakers[token as usize - 1].release();
        }
    }

    pub fn wake(&self, handle: TaskHandle) -> bool {
        let Some(slot) = self.tasks.get(handle.slot()) else {
            return false;
        };
        loop {
            let current = slot.version.load(Ordering::Acquire);
            if generation(current) != handle.generation() {
                return false;
            }
            let next = match state(current) {
                RUNNABLE => {
                    slot.waiting.store(0, Ordering::Release);
                    return true;
                }
                BLOCKED => pack(handle.generation(), RUNNABLE, false),
                RUNNING if current & WAKE_PENDING == 0 => pack(handle.generation(), RUNNING, true),
                RUNNING => {
                    self.notify_task(handle.slot());
                    return true;
                }
                _ => return false,
            };
            if slot
                .version
                .compare_exchange(current, next, Ordering::AcqRel, Ordering::Acquire)
                .is_ok()
            {
                self.notify_task(handle.slot());
                return true;
            }
        }
    }

    #[allow(dead_code)]
    pub fn wake_event(&self, event: Event) -> usize {
        let mut count = 0;
        for (index, slot) in self.tasks.iter().enumerate() {
            if slot.waiting.load(Ordering::Acquire) != event.bits() {
                continue;
            }
            let current = slot.version.load(Ordering::Acquire);
            let handle = TaskHandle((u32::from(generation(current)) << 16) | index as u32);
            if self.wake(handle) {
                count += 1;
            }
        }
        count
    }

    #[allow(dead_code)]
    pub fn fail(&self, handle: TaskHandle) -> bool {
        let Some(slot) = self.tasks.get(handle.slot()) else {
            return false;
        };
        loop {
            let current = slot.version.load(Ordering::Acquire);
            if generation(current) != handle.generation() {
                return false;
            }
            if !matches!(state(current), RUNNABLE | BLOCKED) {
                return false;
            }
            if slot
                .version
                .compare_exchange(
                    current,
                    pack(handle.generation(), FAILED, false),
                    Ordering::AcqRel,
                    Ordering::Acquire,
                )
                .is_ok()
            {
                slot.waiting.store(Event::FAILURE.bits(), Ordering::Release);
                return true;
            }
        }
    }

    #[allow(dead_code)]
    pub fn failed(&self, handle: TaskHandle) -> bool {
        self.state(handle) == Some(SchedulingState::Failed)
    }

    #[allow(dead_code)]
    pub fn restart(&self, handle: TaskHandle) -> Option<TaskHandle> {
        let slot = self.tasks.get(handle.slot())?;
        let current = slot.version.load(Ordering::Acquire);
        if generation(current) != handle.generation() || state(current) != FAILED {
            return None;
        }
        if slot
            .version
            .compare_exchange(
                current,
                pack(handle.generation(), RUNNING, false),
                Ordering::AcqRel,
                Ordering::Acquire,
            )
            .is_err()
        {
            return None;
        }
        let restarted = unsafe {
            (*slot.task.get()).as_mut().is_some_and(|entry| match entry {
                Entry::Runnable(task) => task.restart(),
                Entry::Future(_) => false,
            })
        };
        if !restarted {
            slot.version.store(current, Ordering::Release);
            return None;
        }
        let generation = next_generation(handle.generation());
        slot.waiting.store(0, Ordering::Release);
        slot.version.store(pack(generation, RUNNABLE, false), Ordering::Release);
        Some(TaskHandle((u32::from(generation) << 16) | handle.slot() as u32))
    }

    pub fn state(&self, handle: TaskHandle) -> Option<SchedulingState> {
        let slot = self.tasks.get(handle.slot())?;
        let current = slot.version.load(Ordering::Acquire);
        (generation(current) == handle.generation()).then(|| match state(current) {
            VACANT => SchedulingState::Vacant,
            RUNNABLE => SchedulingState::Runnable,
            RUNNING => SchedulingState::Running,
            BLOCKED => SchedulingState::Blocked,
            FAILED => SchedulingState::Failed,
            _ => SchedulingState::Vacant,
        })
    }

    fn notify_task(&self, index: usize) {
        let slot = &self.tasks[index];
        let cpu = slot.last_cpu.load(Ordering::Acquire) as usize;
        let Some(cpu_state) = self.cpus.get(cpu) else {
            return;
        };
        cpu_state.reschedule.store(true, Ordering::Release);
        if cpu != 0 && cpu_state.active.load(Ordering::Acquire) {
            let _ = (self.notify)(cpu_state.apic_id.load(Ordering::Acquire));
        }
    }
}

enum PollOutcome {
    Runnable(TaskState),
    Future(Poll<()>),
}

const fn pack(generation: u16, state: u8, wake_pending: bool) -> u32 {
    ((generation as u32) << GENERATION_SHIFT)
        | (state as u32)
        | if wake_pending { WAKE_PENDING } else { 0 }
}

fn generation(word: u32) -> u16 {
    ((word >> GENERATION_SHIFT) & GENERATION_MASK) as u16
}

fn state(word: u32) -> u8 {
    (word & STATE_MASK) as u8
}

fn next_generation(generation: u16) -> u16 {
    generation.wrapping_add(1).max(1)
}

fn default_notify(apic_id: u32) -> bool {
    #[cfg(test)]
    {
        let _ = apic_id;
        true
    }
    #[cfg(not(test))]
    {
        crate::arch::interrupts::notify_cpu(apic_id)
    }
}

unsafe fn clone_waker(data: *const ()) -> RawWaker {
    let token = unsafe { &*(data as *const WakerToken) };
    token.acquire();
    RawWaker::new(data, &WAKER_VTABLE)
}

unsafe fn wake_waker(data: *const ()) {
    let token = unsafe { &*(data as *const WakerToken) };
    let scheduler = token.scheduler.load(Ordering::Acquire);
    let handle = TaskHandle(token.handle.load(Ordering::Acquire));
    if !scheduler.is_null() {
        let scheduler = scheduler.cast::<SmpScheduler<'static>>();
        unsafe { (&*scheduler).wake(handle) };
    }
    token.release();
}

unsafe fn wake_waker_by_ref(data: *const ()) {
    let token = unsafe { &*(data as *const WakerToken) };
    let scheduler = token.scheduler.load(Ordering::Acquire);
    let handle = TaskHandle(token.handle.load(Ordering::Acquire));
    if !scheduler.is_null() {
        let scheduler = scheduler.cast::<SmpScheduler<'static>>();
        unsafe { (&*scheduler).wake(handle) };
    }
}

unsafe fn drop_waker(data: *const ()) {
    let token = unsafe { &*(data as *const WakerToken) };
    token.release();
}

static WAKER_VTABLE: RawWakerVTable =
    RawWakerVTable::new(clone_waker, wake_waker, wake_waker_by_ref, drop_waker);

pub fn self_check(topology: &crate::arch::acpi::CpuTopology) -> bool {
    struct YieldOnce(u8);
    impl Future for YieldOnce {
        type Output = ();

        fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
            self.0 += 1;
            if self.0 == 1 {
                cx.waker().wake_by_ref();
                Poll::Pending
            } else {
                Poll::Ready(())
            }
        }
    }

    let mut future = YieldOnce(0);
    let mut scheduler = SmpScheduler::new();
    if topology.truncated() && topology.count() != crate::arch::acpi::MAX_CPUS {
        return false;
    }
    if scheduler.configure_topology(topology) == 0
        || !scheduler.cpu(0).is_some_and(|cpu| cpu.active)
    {
        return false;
    }
    let Ok(handle) = scheduler.spawn_future(&mut future) else {
        return false;
    };
    scheduler.run_next(0)
        && scheduler.state(handle) == Some(SchedulingState::Runnable)
        && scheduler.run_next(0)
        && scheduler.state(handle).is_none()
}

#[cfg(test)]
mod tests {
    use super::*;

    struct BlockOnce {
        runs: u8,
    }

    impl Runnable for BlockOnce {
        fn run(&mut self) -> TaskState {
            self.runs += 1;
            if self.runs == 1 { TaskState::Blocked(Event::INPUT) } else { TaskState::Complete }
        }
    }

    #[test]
    fn duplicate_wake_does_not_duplicate_runnable_state() {
        let mut task = BlockOnce { runs: 0 };
        let mut scheduler = SmpScheduler::new().with_notifier(|_| true);
        let handle = scheduler.spawn_runnable(&mut task).unwrap();
        assert!(scheduler.run_next(0));
        assert_eq!(scheduler.state(handle), Some(SchedulingState::Blocked));
        assert!(scheduler.wake(handle));
        assert!(scheduler.wake(handle));
        assert_eq!(scheduler.state(handle), Some(SchedulingState::Runnable));
        assert!(scheduler.run_next(0));
        assert!(!scheduler.run_next(0));
    }

    #[test]
    fn stale_waker_cannot_wake_reused_slot() {
        let mut first = YieldFuture { polls: 0, waker: None };
        let mut second = BlockFuture;
        let mut scheduler = SmpScheduler::new().with_notifier(|_| true);
        let first_handle = scheduler.spawn_future(&mut first).unwrap();
        assert!(scheduler.run_next(0));
        let old_waker = first.waker.take().unwrap();
        let stale_waker = old_waker.clone();
        old_waker.wake();
        assert!(scheduler.run_next(0));
        let second_handle = scheduler.spawn_future(&mut second).unwrap();
        stale_waker.wake();
        assert_ne!(first_handle.generation(), second_handle.generation());
        assert_eq!(scheduler.state(second_handle), Some(SchedulingState::Runnable));
    }

    struct YieldFuture {
        polls: u8,
        waker: Option<Waker>,
    }

    impl Future for YieldFuture {
        type Output = ();

        fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
            self.polls += 1;
            if self.polls == 1 {
                self.waker = Some(cx.waker().clone());
                Poll::Pending
            } else {
                Poll::Ready(())
            }
        }
    }

    struct BlockFuture;

    impl Future for BlockFuture {
        type Output = ();

        fn poll(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<Self::Output> {
            Poll::Pending
        }
    }

    #[test]
    fn future_wake_race_resumes_once() {
        let mut future = YieldFuture { polls: 0, waker: None };
        let mut scheduler = SmpScheduler::new().with_notifier(|_| true);
        let handle = scheduler.spawn_future(&mut future).unwrap();
        assert!(scheduler.run_next(0));
        future.waker.take().unwrap().wake();
        assert_eq!(scheduler.state(handle), Some(SchedulingState::Runnable));
        assert!(scheduler.run_next(1) == false);
        assert!(scheduler.run_next(0));
        assert!(scheduler.state(handle).is_none());
    }

    #[test]
    fn bounded_task_and_waker_capacity_is_explicit() {
        let mut tasks = [BlockFuture; MAX_SMP_TASKS];
        let mut scheduler = SmpScheduler::new().with_notifier(|_| true);
        for task in &mut tasks {
            assert!(scheduler.spawn_future(task).is_ok());
        }
        assert_eq!(scheduler.spawn_future(&mut BlockFuture), Err(SpawnError::Full));
    }
}
