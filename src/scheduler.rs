use core::{
    cell::UnsafeCell,
    sync::atomic::{AtomicBool, AtomicU8, AtomicU64, AtomicUsize, Ordering},
};

use crate::process::{ProcessHandle, UserLaunch};

pub const MAX_TASKS: usize = 24;
pub const MAX_CPUS: usize = 8;
#[cfg(target_os = "uefi")]
pub const TASK_STACK_SIZE: usize = 256 * 1024;
#[cfg(not(target_os = "uefi"))]
pub const TASK_STACK_SIZE: usize = 16 * 1024;
pub const SCHEDULER_STACK_SIZE: usize = 64 * 1024;
pub const SCHEDULER_STACK_GUARD_BYTES: usize = 256;
pub const IDLE_STACK_SIZE: usize = 4 * 1024;

const _: () = assert!(SCHEDULER_STACK_SIZE > SCHEDULER_STACK_GUARD_BYTES);

const VACANT: u64 = 0;
const INITIALIZING: u64 = 1;
const RUNNABLE: u64 = 2;
const RUNNING: u64 = 3;
const BLOCKED: u64 = 4;
const COMPLETED: u64 = 5;
const STOPPING: u64 = 6;
const STATE_MASK: u64 = 0x0f;
const WAKE_PENDING: u64 = 1 << 4;
const GENERATION_SHIFT: u32 = 8;
const GENERATION_MASK: u64 = (1 << (64 - GENERATION_SHIFT)) - 1;
const INITIAL_GENERATION: u64 = 1;
const NO_CPU_TASK: u8 = u8::MAX;
const NO_DEADLINE: u64 = u64::MAX;

pub type TaskEntry = fn();

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TaskState {
    Runnable,
    Running,
    Blocked,
    Stopping,
    Completed,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SpawnError {
    Capacity,
    AddressSpace,
    UserLaunch,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TaskHandle {
    slot: u8,
    generation: u64,
}

impl TaskHandle {
    pub const fn slot(self) -> usize {
        self.slot as usize
    }

    pub const fn generation(self) -> u64 {
        self.generation
    }

    pub const fn raw(self) -> u64 {
        ((self.generation & GENERATION_MASK) << GENERATION_SHIFT) | self.slot as u64
    }

    pub const fn from_raw(raw: u64) -> Self {
        Self { slot: raw as u8, generation: (raw >> GENERATION_SHIFT) & GENERATION_MASK }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FinishState {
    Runnable,
    Blocked,
    TimedBlocked,
    Completed,
}

#[repr(C, align(16))]
struct TaskStack([u8; TASK_STACK_SIZE]);

struct TaskSlot {
    entry: AtomicUsize,
    #[allow(dead_code)]
    stack: UnsafeCell<TaskStack>,
    state: AtomicU64,
    wake_deadline: AtomicU64,
    saved_rsp: AtomicUsize,
    context_saved: AtomicBool,
    wait_mask: AtomicU64,
    wait_object: AtomicU64,
    address_space: AtomicUsize,
    process: AtomicU64,
    user_entry: AtomicUsize,
    user_stack_top: AtomicUsize,
    address_space_published: AtomicBool,
}

impl TaskSlot {
    const fn new() -> Self {
        Self {
            entry: AtomicUsize::new(0),
            stack: UnsafeCell::new(TaskStack([0; TASK_STACK_SIZE])),
            state: AtomicU64::new(pack(INITIAL_GENERATION, VACANT)),
            wake_deadline: AtomicU64::new(NO_DEADLINE),
            saved_rsp: AtomicUsize::new(0),
            context_saved: AtomicBool::new(false),
            wait_mask: AtomicU64::new(0),
            wait_object: AtomicU64::new(0),
            address_space: AtomicUsize::new(0),
            process: AtomicU64::new(0),
            user_entry: AtomicUsize::new(0),
            user_stack_top: AtomicUsize::new(0),
            address_space_published: AtomicBool::new(false),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ScheduledUserLaunch {
    process: ProcessHandle,
    launch: UserLaunch,
}

impl ScheduledUserLaunch {
    pub const fn process(self) -> ProcessHandle {
        self.process
    }

    pub const fn entry(self) -> usize {
        self.launch.entry()
    }

    pub const fn stack_top(self) -> usize {
        self.launch.stack_top()
    }

    pub const fn address_space_root(self) -> usize {
        self.launch.address_space_root().raw()
    }

    pub const fn launch(self) -> UserLaunch {
        self.launch
    }
}

struct CpuState {
    online: AtomicBool,
    cursor: AtomicUsize,
    current_slot: AtomicU8,
    current_generation: AtomicU64,
    ticks: AtomicU64,
    switches: AtomicU64,
}

impl CpuState {
    const fn new() -> Self {
        Self {
            online: AtomicBool::new(false),
            cursor: AtomicUsize::new(0),
            current_slot: AtomicU8::new(NO_CPU_TASK),
            current_generation: AtomicU64::new(0),
            ticks: AtomicU64::new(0),
            switches: AtomicU64::new(0),
        }
    }
}

pub struct Scheduler {
    tasks: [TaskSlot; MAX_TASKS],
    cpus: [CpuState; MAX_CPUS],
    event_pending: AtomicU64,
    event_signal_mask: AtomicU64,
    event_wakes: AtomicU64,
}

// A slot's mutable entry and stack are accessed only after a state CAS has
// exclusively claimed the slot. Publication uses Release and all claimants
// use Acquire, so the fixed storage is safe to share between CPUs.
unsafe impl Sync for Scheduler {}

impl Scheduler {
    pub const fn new() -> Self {
        Self {
            tasks: [const { TaskSlot::new() }; MAX_TASKS],
            cpus: [const { CpuState::new() }; MAX_CPUS],
            event_pending: AtomicU64::new(0),
            event_signal_mask: AtomicU64::new(0),
            event_wakes: AtomicU64::new(0),
        }
    }

    pub fn online_cpu(&self, cpu: usize) -> bool {
        let Some(cpu_state) = self.cpus.get(cpu) else {
            return false;
        };
        cpu_state.online.store(true, Ordering::Release);
        true
    }

    pub fn cpu_online(&self, cpu: usize) -> bool {
        self.cpus.get(cpu).is_some_and(|state| state.online.load(Ordering::Acquire))
    }

    pub fn spawn(&self, entry: TaskEntry) -> Result<TaskHandle, SpawnError> {
        self.spawn_with_address_space(entry, 0)
    }

    pub fn spawn_with_address_space(
        &self,
        entry: TaskEntry,
        address_space: usize,
    ) -> Result<TaskHandle, SpawnError> {
        self.spawn_internal(entry, address_space, None)
    }

    /// Reserve a scheduler slot for a loaded process.
    ///
    /// `entry` remains the kernel trampoline for now. The user register set is
    /// published atomically with the task's runnable state for the future
    /// ring-3 context path.
    pub fn spawn_user(
        &self,
        entry: TaskEntry,
        process: ProcessHandle,
        launch: UserLaunch,
    ) -> Result<TaskHandle, SpawnError> {
        self.spawn_internal(
            entry,
            launch.address_space_root().raw(),
            Some(ScheduledUserLaunch { process, launch }),
        )
    }

    fn spawn_internal(
        &self,
        entry: TaskEntry,
        address_space: usize,
        user_launch: Option<ScheduledUserLaunch>,
    ) -> Result<TaskHandle, SpawnError> {
        if address_space != 0 && address_space & 0xfff != 0 {
            return Err(SpawnError::AddressSpace);
        }
        if user_launch.is_some_and(|launch| launch.process().raw() == 0) {
            return Err(SpawnError::UserLaunch);
        }
        for (index, slot) in self.tasks.iter().enumerate() {
            let old = slot.state.load(Ordering::Acquire);
            if state(old) != VACANT {
                continue;
            }
            let generation = generation(old);
            let reserved = pack(generation, INITIALIZING);
            if slot
                .state
                .compare_exchange(old, reserved, Ordering::AcqRel, Ordering::Acquire)
                .is_err()
            {
                continue;
            }
            slot.entry.store(entry as usize, Ordering::Release);
            slot.wake_deadline.store(NO_DEADLINE, Ordering::Release);
            slot.saved_rsp.store(0, Ordering::Release);
            slot.context_saved.store(false, Ordering::Release);
            slot.wait_mask.store(0, Ordering::Release);
            slot.wait_object.store(0, Ordering::Release);
            slot.address_space.store(address_space, Ordering::Release);
            slot.address_space_published.store(address_space != 0, Ordering::Release);
            if let Some(launch) = user_launch {
                slot.process.store(launch.process().raw(), Ordering::Release);
                slot.user_entry.store(launch.entry(), Ordering::Release);
                slot.user_stack_top.store(launch.stack_top(), Ordering::Release);
            } else {
                slot.process.store(0, Ordering::Release);
                slot.user_entry.store(0, Ordering::Release);
                slot.user_stack_top.store(0, Ordering::Release);
            }
            slot.state.store(pack(generation, RUNNABLE), Ordering::Release);
            return Ok(TaskHandle { slot: index as u8, generation });
        }
        Err(SpawnError::Capacity)
    }

    #[cfg_attr(not(target_os = "uefi"), allow(dead_code))]
    /// Arm one bounded timer deadline for a currently running task.
    pub(crate) fn arm_deadline(&self, handle: TaskHandle, deadline: u64) -> bool {
        let Some(slot) = self.tasks.get(handle.slot as usize) else {
            return false;
        };
        let word = slot.state.load(Ordering::Acquire);
        if generation(word) != handle.generation || state(word) != RUNNING {
            return false;
        }
        slot.wake_deadline.store(deadline, Ordering::Release);
        true
    }

    /// Register a bounded event wait. Returns `true` when the caller must
    /// block; an already pending event returns `false` so the caller can
    /// recheck its condition without entering the scheduler. A zero mask is
    /// allowed only with a finite deadline for a timeout-only sleep.
    #[allow(dead_code)]
    pub(crate) fn wait_for_events(
        &self,
        handle: TaskHandle,
        mask: u64,
        deadline: u64,
    ) -> Option<bool> {
        let valid_mask = if logos_abi::EVENT_COUNT == 64 {
            u64::MAX
        } else {
            (1u64 << logos_abi::EVENT_COUNT) - 1
        };
        if mask & !valid_mask != 0 || (mask == 0 && deadline == NO_DEADLINE) {
            return None;
        }
        let slot = self.tasks.get(handle.slot as usize)?;
        let word = slot.state.load(Ordering::Acquire);
        if generation(word) != handle.generation || state(word) != RUNNING {
            return None;
        }

        slot.wait_mask.store(0, Ordering::Release);
        slot.wait_object.store(0, Ordering::Release);
        slot.wake_deadline.store(deadline, Ordering::Release);
        if mask == 0 {
            return Some(true);
        }
        if self.event_pending.load(Ordering::Acquire) & mask != 0 {
            self.event_pending.fetch_and(!mask, Ordering::AcqRel);
            slot.wake_deadline.store(NO_DEADLINE, Ordering::Release);
            return Some(false);
        }
        slot.wait_mask.store(mask, Ordering::Release);
        if self.event_pending.load(Ordering::Acquire) & mask != 0 {
            slot.wait_mask.store(0, Ordering::Release);
            self.event_pending.fetch_and(!mask, Ordering::AcqRel);
            slot.wake_deadline.store(NO_DEADLINE, Ordering::Release);
            return Some(false);
        }
        Some(true)
    }

    /// Register a wait on one runtime event-set object.
    #[allow(dead_code)]
    pub(crate) fn wait_for_event_object(
        &self,
        handle: TaskHandle,
        object: u64,
        deadline: u64,
    ) -> Option<bool> {
        if object == 0 {
            return None;
        }
        let slot = self.tasks.get(handle.slot as usize)?;
        let word = slot.state.load(Ordering::Acquire);
        if generation(word) != handle.generation || state(word) != RUNNING {
            return None;
        }
        slot.wait_mask.store(0, Ordering::Release);
        slot.wait_object.store(0, Ordering::Release);
        slot.wake_deadline.store(deadline, Ordering::Release);
        slot.wait_object.store(object, Ordering::Release);
        Some(true)
    }

    /// Signal event edges from a producer or IRQ-safe adapter. Pending bits
    /// remain latched until a matching waiter consumes them, closing the
    /// check-then-sleep race without an allocator or lock.
    #[allow(dead_code)]
    pub(crate) fn signal_events(&self, mask: u64) -> usize {
        let valid_mask = if logos_abi::EVENT_COUNT == 64 {
            u64::MAX
        } else {
            (1u64 << logos_abi::EVENT_COUNT) - 1
        };
        let mask = mask & valid_mask;
        if mask == 0 {
            return 0;
        }
        self.event_pending.fetch_or(mask, Ordering::AcqRel);
        self.event_signal_mask.fetch_or(mask, Ordering::AcqRel);
        let mut woken = 0;
        for (index, slot) in self.tasks.iter().enumerate() {
            if slot.wait_object.load(Ordering::Acquire) != 0 {
                continue;
            }
            let wait_mask = slot.wait_mask.load(Ordering::Acquire);
            if wait_mask & mask == 0 {
                continue;
            }
            let word = slot.state.load(Ordering::Acquire);
            if !matches!(state(word), BLOCKED | RUNNING) {
                continue;
            }
            let handle = TaskHandle { slot: index as u8, generation: generation(word) };
            let was_blocked = state(word) == BLOCKED;
            if self.wake(handle) {
                if was_blocked {
                    self.event_wakes.fetch_add(1, Ordering::Relaxed);
                }
                woken += 1;
            }
        }
        woken
    }

    /// Wake tasks waiting on one runtime event-set object.
    #[allow(dead_code)]
    pub(crate) fn signal_event_object(&self, object: u64) -> usize {
        if object == 0 {
            return 0;
        }
        let mut woken = 0;
        for (index, slot) in self.tasks.iter().enumerate() {
            if slot.wait_object.load(Ordering::Acquire) != object {
                continue;
            }
            let word = slot.state.load(Ordering::Acquire);
            if !matches!(state(word), BLOCKED | RUNNING) {
                continue;
            }
            let handle = TaskHandle { slot: index as u8, generation: generation(word) };
            if self.wake(handle) {
                self.event_wakes.fetch_add(1, Ordering::Relaxed);
                woken += 1;
            }
        }
        woken
    }

    #[cfg_attr(not(target_os = "uefi"), allow(dead_code))]
    pub(crate) fn reset_events(&self) {
        self.event_pending.store(0, Ordering::Release);
        for (index, slot) in self.tasks.iter().enumerate() {
            let word = slot.state.load(Ordering::Acquire);
            if state(word) == BLOCKED {
                let handle = TaskHandle { slot: index as u8, generation: generation(word) };
                self.wake(handle);
            }
            slot.wait_mask.store(0, Ordering::Release);
            slot.wait_object.store(0, Ordering::Release);
            slot.wake_deadline.store(NO_DEADLINE, Ordering::Release);
        }
    }

    #[allow(dead_code)]
    pub(crate) fn event_signal_mask(&self) -> u64 {
        self.event_signal_mask.load(Ordering::Acquire)
    }

    #[allow(dead_code)]
    pub(crate) fn event_wakes(&self) -> u64 {
        self.event_wakes.load(Ordering::Acquire)
    }

    #[cfg_attr(not(target_os = "uefi"), allow(dead_code))]
    /// Wake blocked tasks whose deadlines have elapsed. Returns the wake count.
    pub(crate) fn wake_due(&self, now: u64) -> usize {
        let mut woken = 0;
        for slot in &self.tasks {
            let word = slot.state.load(Ordering::Acquire);
            if state(word) != BLOCKED {
                continue;
            }
            let deadline = slot.wake_deadline.load(Ordering::Acquire);
            if deadline == NO_DEADLINE || deadline > now {
                continue;
            }
            if slot
                .wake_deadline
                .compare_exchange(deadline, NO_DEADLINE, Ordering::AcqRel, Ordering::Acquire)
                .is_err()
            {
                continue;
            }
            slot.wait_mask.store(0, Ordering::Release);
            slot.wait_object.store(0, Ordering::Release);
            if slot
                .state
                .compare_exchange(
                    word,
                    pack(generation(word), RUNNABLE),
                    Ordering::AcqRel,
                    Ordering::Acquire,
                )
                .is_ok()
            {
                woken += 1;
            }
        }
        woken
    }

    pub fn state(&self, handle: TaskHandle) -> Option<TaskState> {
        let slot = self.tasks.get(handle.slot as usize)?;
        let word = slot.state.load(Ordering::Acquire);
        if generation(word) != handle.generation {
            return None;
        }
        match state(word) {
            RUNNABLE => Some(TaskState::Runnable),
            RUNNING => Some(TaskState::Running),
            BLOCKED => Some(TaskState::Blocked),
            STOPPING => Some(TaskState::Stopping),
            COMPLETED => Some(TaskState::Completed),
            _ => None,
        }
    }

    pub fn wake(&self, handle: TaskHandle) -> bool {
        let Some(slot) = self.tasks.get(handle.slot as usize) else {
            return false;
        };
        loop {
            let old = slot.state.load(Ordering::Acquire);
            if generation(old) != handle.generation {
                return false;
            }
            let next = match state(old) {
                BLOCKED => {
                    slot.wake_deadline.store(NO_DEADLINE, Ordering::Release);
                    slot.wait_mask.store(0, Ordering::Release);
                    slot.wait_object.store(0, Ordering::Release);
                    pack(handle.generation, RUNNABLE)
                }
                RUNNING => {
                    slot.wait_mask.store(0, Ordering::Release);
                    slot.wait_object.store(0, Ordering::Release);
                    old | WAKE_PENDING
                }
                RUNNABLE => {
                    slot.wait_mask.store(0, Ordering::Release);
                    slot.wait_object.store(0, Ordering::Release);
                    old
                }
                _ => return false,
            };
            if next == old {
                return true;
            }
            if slot.state.compare_exchange(old, next, Ordering::AcqRel, Ordering::Acquire).is_ok() {
                return true;
            }
        }
    }

    /// Request bounded termination of a task before reclaiming its address
    /// space. Running tasks finish on their next scheduler boundary; tasks
    /// that cannot be running are completed immediately.
    pub fn request_stop(&self, handle: TaskHandle) -> bool {
        let Some(slot) = self.tasks.get(handle.slot as usize) else {
            return false;
        };
        loop {
            let old = slot.state.load(Ordering::Acquire);
            if generation(old) != handle.generation {
                return false;
            }
            let next = match state(old) {
                RUNNING => pack(handle.generation, STOPPING),
                RUNNABLE | BLOCKED => pack(handle.generation, COMPLETED),
                STOPPING | COMPLETED => return true,
                _ => return false,
            };
            slot.wake_deadline.store(NO_DEADLINE, Ordering::Release);
            slot.wait_mask.store(0, Ordering::Release);
            slot.wait_object.store(0, Ordering::Release);
            if slot.state.compare_exchange(old, next, Ordering::AcqRel, Ordering::Acquire).is_ok() {
                return true;
            }
        }
    }

    pub fn claim_next(&self, cpu: usize) -> Option<TaskHandle> {
        let cpu_state = self.cpus.get(cpu)?;
        if !cpu_state.online.load(Ordering::Acquire) {
            return None;
        }
        let start = cpu_state.cursor.fetch_add(1, Ordering::Relaxed) % MAX_TASKS;
        for offset in 0..MAX_TASKS {
            let index = (start + offset) % MAX_TASKS;
            let slot = &self.tasks[index];
            let old = slot.state.load(Ordering::Acquire);
            if state(old) != RUNNABLE {
                continue;
            }
            let next = pack(generation(old), RUNNING);
            if slot.state.compare_exchange(old, next, Ordering::AcqRel, Ordering::Acquire).is_ok() {
                let handle = TaskHandle { slot: index as u8, generation: generation(old) };
                cpu_state.current_slot.store(handle.slot, Ordering::Release);
                cpu_state.current_generation.store(handle.generation, Ordering::Release);
                cpu_state.switches.fetch_add(1, Ordering::Relaxed);
                return Some(handle);
            }
        }
        None
    }

    /// Publish a context only after the assembly path has left the task stack.
    pub fn save_context(&self, handle: TaskHandle, saved_rsp: usize) -> bool {
        let Some(slot) = self.tasks.get(handle.slot as usize) else {
            return false;
        };
        let word = slot.state.load(Ordering::Acquire);
        if generation(word) != handle.generation || !matches!(state(word), RUNNING | STOPPING) {
            return false;
        }
        slot.saved_rsp.store(saved_rsp, Ordering::Release);
        slot.context_saved.store(true, Ordering::Release);
        true
    }

    pub fn saved_context(&self, handle: TaskHandle) -> Option<usize> {
        let slot = self.tasks.get(handle.slot as usize)?;
        let word = slot.state.load(Ordering::Acquire);
        (generation(word) == handle.generation && slot.context_saved.load(Ordering::Acquire))
            .then(|| slot.saved_rsp.load(Ordering::Acquire))
    }

    pub fn finish(&self, handle: TaskHandle, outcome: FinishState) -> bool {
        let Some(slot) = self.tasks.get(handle.slot as usize) else {
            return false;
        };
        if !slot.context_saved.load(Ordering::Acquire) {
            return false;
        }
        loop {
            let old = slot.state.load(Ordering::Acquire);
            if generation(old) != handle.generation || !matches!(state(old), RUNNING | STOPPING) {
                return false;
            }
            let next_state = match outcome {
                FinishState::Runnable => RUNNABLE,
                FinishState::Blocked if old & WAKE_PENDING == 0 => BLOCKED,
                FinishState::Blocked => RUNNABLE,
                FinishState::TimedBlocked if old & WAKE_PENDING == 0 => BLOCKED,
                FinishState::TimedBlocked => RUNNABLE,
                FinishState::Completed => COMPLETED,
            };
            let next_state = if state(old) == STOPPING { COMPLETED } else { next_state };
            if next_state != BLOCKED {
                slot.wait_mask.store(0, Ordering::Release);
                slot.wait_object.store(0, Ordering::Release);
            }
            if next_state != BLOCKED || matches!(outcome, FinishState::Blocked) {
                slot.wake_deadline.store(NO_DEADLINE, Ordering::Release);
            }
            if slot
                .state
                .compare_exchange(
                    old,
                    pack(handle.generation, next_state),
                    Ordering::AcqRel,
                    Ordering::Acquire,
                )
                .is_ok()
            {
                return true;
            }
        }
    }

    pub fn reclaim_completed(&self, handle: TaskHandle) -> bool {
        let Some(slot) = self.tasks.get(handle.slot as usize) else {
            return false;
        };
        let old = pack(handle.generation, COMPLETED);
        if slot
            .state
            .compare_exchange(
                old,
                pack(handle.generation, INITIALIZING),
                Ordering::AcqRel,
                Ordering::Acquire,
            )
            .is_err()
        {
            return false;
        }
        slot.entry.store(0, Ordering::Release);
        slot.address_space.store(0, Ordering::Release);
        slot.process.store(0, Ordering::Release);
        slot.user_entry.store(0, Ordering::Release);
        slot.user_stack_top.store(0, Ordering::Release);
        slot.address_space_published.store(false, Ordering::Release);
        slot.wake_deadline.store(NO_DEADLINE, Ordering::Release);
        slot.wait_mask.store(0, Ordering::Release);
        slot.wait_object.store(0, Ordering::Release);
        slot.saved_rsp.store(0, Ordering::Release);
        slot.context_saved.store(false, Ordering::Release);
        slot.state.store(pack(next_generation(handle.generation), VACANT), Ordering::Release);
        true
    }

    pub fn current_task(&self, cpu: usize) -> Option<TaskHandle> {
        let cpu_state = self.cpus.get(cpu)?;
        let slot = cpu_state.current_slot.load(Ordering::Acquire);
        (slot != NO_CPU_TASK).then_some(TaskHandle {
            slot,
            generation: cpu_state.current_generation.load(Ordering::Acquire),
        })
    }

    #[cfg_attr(not(target_os = "uefi"), allow(dead_code))]
    pub(crate) fn clear_current(&self, cpu: usize) {
        if let Some(cpu_state) = self.cpus.get(cpu) {
            cpu_state.current_slot.store(NO_CPU_TASK, Ordering::Release);
            cpu_state.current_generation.store(0, Ordering::Release);
        }
    }

    #[cfg_attr(not(target_os = "uefi"), allow(dead_code))]
    pub(crate) fn task_stack_top(&self, handle: TaskHandle) -> Option<usize> {
        let slot = self.tasks.get(handle.slot as usize)?;
        let word = slot.state.load(Ordering::Acquire);
        (generation(word) == handle.generation)
            .then(|| unsafe { (*slot.stack.get()).0.as_ptr_range().end as usize })
    }

    #[cfg_attr(not(target_os = "uefi"), allow(dead_code))]
    pub(crate) fn set_initial_context(&self, handle: TaskHandle, saved_rsp: usize) -> bool {
        let Some(slot) = self.tasks.get(handle.slot as usize) else {
            return false;
        };
        let word = slot.state.load(Ordering::Acquire);
        if generation(word) != handle.generation || state(word) != RUNNING {
            return false;
        }
        if slot.context_saved.load(Ordering::Acquire) {
            return true;
        }
        slot.saved_rsp.store(saved_rsp, Ordering::Release);
        slot.context_saved.store(true, Ordering::Release);
        true
    }

    pub fn record_tick(&self, cpu: usize) {
        if let Some(cpu_state) = self.cpus.get(cpu) {
            cpu_state.ticks.fetch_add(1, Ordering::Relaxed);
        }
    }

    pub fn ticks(&self, cpu: usize) -> Option<u64> {
        self.cpus.get(cpu).map(|state| state.ticks.load(Ordering::Acquire))
    }

    pub fn switches(&self, cpu: usize) -> Option<u64> {
        self.cpus.get(cpu).map(|state| state.switches.load(Ordering::Acquire))
    }

    #[cfg_attr(not(target_os = "uefi"), allow(dead_code))]
    pub(crate) fn cursor(&self, cpu: usize) -> Option<usize> {
        self.cpus.get(cpu).map(|state| state.cursor.load(Ordering::Acquire))
    }

    pub fn entry(&self, handle: TaskHandle) -> Option<TaskEntry> {
        let slot = self.tasks.get(handle.slot as usize)?;
        let word = slot.state.load(Ordering::Acquire);
        if generation(word) != handle.generation {
            return None;
        }
        let pointer = slot.entry.load(Ordering::Acquire);
        (pointer != 0).then(|| unsafe { core::mem::transmute(pointer) })
    }

    pub fn address_space(&self, handle: TaskHandle) -> Option<usize> {
        let slot = self.tasks.get(handle.slot as usize)?;
        let word = slot.state.load(Ordering::Acquire);
        if generation(word) != handle.generation {
            return None;
        }
        slot.address_space_published
            .load(Ordering::Acquire)
            .then(|| slot.address_space.load(Ordering::Acquire))
            .or(Some(0))
    }

    #[cfg_attr(not(target_os = "uefi"), allow(dead_code))]
    pub(crate) fn normalize_kernel_task(&self, handle: TaskHandle) {
        let Some(slot) = self.tasks.get(handle.slot as usize) else {
            return;
        };
        let word = slot.state.load(Ordering::Acquire);
        if generation(word) == handle.generation
            && !slot.address_space_published.load(Ordering::Acquire)
        {
            slot.address_space.store(0, Ordering::Release);
            slot.user_entry.store(0, Ordering::Release);
            slot.user_stack_top.store(0, Ordering::Release);
        }
    }

    pub fn user_launch(&self, handle: TaskHandle) -> Option<ScheduledUserLaunch> {
        let slot = self.tasks.get(handle.slot as usize)?;
        let word = slot.state.load(Ordering::Acquire);
        if generation(word) != handle.generation {
            return None;
        }
        let process = ProcessHandle::from_raw(slot.process.load(Ordering::Acquire));
        if process.raw() == 0 {
            return None;
        }
        let root =
            crate::process::AddressSpaceRoot::new(slot.address_space.load(Ordering::Acquire))?;
        let launch = UserLaunch::new(
            slot.user_entry.load(Ordering::Acquire),
            slot.user_stack_top.load(Ordering::Acquire),
            root,
        )?;
        Some(ScheduledUserLaunch { process, launch })
    }
}

impl Default for Scheduler {
    fn default() -> Self {
        Self::new()
    }
}

const fn pack(generation: u64, state: u64) -> u64 {
    ((generation & GENERATION_MASK) << GENERATION_SHIFT) | state
}

const fn generation(word: u64) -> u64 {
    word >> GENERATION_SHIFT
}

const fn state(word: u64) -> u64 {
    word & STATE_MASK
}

const fn next_generation(generation: u64) -> u64 {
    let next = (generation & GENERATION_MASK).wrapping_add(1) & GENERATION_MASK;
    if next == 0 { INITIAL_GENERATION } else { next }
}

pub static SCHEDULER: Scheduler = Scheduler::new();

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::sync::Barrier;
    use std::thread;
    use std::vec::Vec;

    fn empty() {}

    fn running(scheduler: &Scheduler) -> TaskHandle {
        let handle = scheduler.spawn(empty).unwrap();
        scheduler.online_cpu(0);
        scheduler.claim_next(0).unwrap();
        handle
    }

    #[test]
    fn transitions_require_saved_context_before_requeue() {
        let scheduler = Scheduler::new();
        let handle = running(&scheduler);
        assert!(!scheduler.finish(handle, FinishState::Runnable));
        assert_eq!(scheduler.state(handle), Some(TaskState::Running));
        assert!(scheduler.save_context(handle, 0x1234));
        assert!(scheduler.finish(handle, FinishState::Runnable));
        assert_eq!(scheduler.state(handle), Some(TaskState::Runnable));
        assert_eq!(scheduler.saved_context(handle), Some(0x1234));
    }

    #[test]
    fn blocked_task_wakes_and_duplicate_wakes_are_cheap() {
        let scheduler = Scheduler::new();
        let handle = running(&scheduler);
        assert!(scheduler.save_context(handle, 0x2000));
        assert!(scheduler.finish(handle, FinishState::Blocked));
        assert_eq!(scheduler.state(handle), Some(TaskState::Blocked));
        assert!(scheduler.wake(handle));
        assert!(scheduler.wake(handle));
        assert_eq!(scheduler.state(handle), Some(TaskState::Runnable));
    }

    #[test]
    fn event_signal_before_wait_is_latched() {
        let scheduler = Scheduler::new();
        let handle = running(&scheduler);
        let event = logos_abi::ipc_read_event_mask(0);
        assert_eq!(scheduler.signal_events(event), 0);
        assert_eq!(scheduler.wait_for_events(handle, event, 10), Some(false));
        assert_eq!(scheduler.state(handle), Some(TaskState::Running));
    }

    #[test]
    fn event_wait_blocks_and_signal_wakes_receiver() {
        let scheduler = Scheduler::new();
        let handle = running(&scheduler);
        let event = logos_abi::ipc_read_event_mask(0);
        assert_eq!(scheduler.wait_for_events(handle, event, 10), Some(true));
        assert!(scheduler.save_context(handle, 0x2100));
        assert!(scheduler.finish(handle, FinishState::TimedBlocked));
        assert_eq!(scheduler.state(handle), Some(TaskState::Blocked));
        assert_eq!(scheduler.signal_events(event), 1);
        assert_eq!(scheduler.state(handle), Some(TaskState::Runnable));
        assert_eq!(scheduler.wake_due(10), 0);
    }

    #[test]
    fn runtime_event_object_wait_blocks_and_signal_wakes_receiver() {
        let scheduler = Scheduler::new();
        let handle = running(&scheduler);
        assert_eq!(scheduler.wait_for_event_object(handle, 0x1000, 10), Some(true));
        assert!(scheduler.save_context(handle, 0x2180));
        assert!(scheduler.finish(handle, FinishState::TimedBlocked));
        assert_eq!(scheduler.signal_event_object(0x1000), 1);
        assert_eq!(scheduler.event_wakes(), 1);
        assert_eq!(scheduler.state(handle), Some(TaskState::Runnable));
        assert_eq!(scheduler.wake_due(10), 0);
    }

    #[test]
    fn timeout_only_wait_blocks_and_wakes() {
        let scheduler = Scheduler::new();
        let handle = running(&scheduler);
        assert_eq!(scheduler.wait_for_events(handle, 0, 10), Some(true));
        assert!(scheduler.save_context(handle, 0x2140));
        assert!(scheduler.finish(handle, FinishState::TimedBlocked));
        assert_eq!(scheduler.wake_due(9), 0);
        assert_eq!(scheduler.wake_due(10), 1);
        assert_eq!(scheduler.state(handle), Some(TaskState::Runnable));
    }

    #[test]
    fn indefinite_timeout_only_wait_is_rejected() {
        let scheduler = Scheduler::new();
        let handle = running(&scheduler);
        assert_eq!(scheduler.wait_for_events(handle, 0, u64::MAX), None);
    }

    #[test]
    fn event_signal_racing_with_block_claim_keeps_task_runnable() {
        let scheduler = Scheduler::new();
        let handle = running(&scheduler);
        let event = logos_abi::ipc_write_event_mask(0);
        assert_eq!(scheduler.wait_for_events(handle, event, 0), Some(true));
        assert_eq!(scheduler.signal_events(event), 1);
        assert!(scheduler.save_context(handle, 0x2150));
        assert!(scheduler.finish(handle, FinishState::Blocked));
        assert_eq!(scheduler.state(handle), Some(TaskState::Runnable));
    }

    #[test]
    fn event_wait_accepts_a_bounded_wait_any_mask() {
        let scheduler = Scheduler::new();
        let handle = running(&scheduler);
        let mask = logos_abi::ipc_read_event_mask(0) | logos_abi::ipc_read_event_mask(3);
        assert_eq!(scheduler.wait_for_events(handle, mask, 10), Some(true));
        assert!(scheduler.save_context(handle, 0x2180));
        assert!(scheduler.finish(handle, FinishState::TimedBlocked));
        assert_eq!(scheduler.signal_events(logos_abi::ipc_read_event_mask(3)), 1);
        assert_eq!(scheduler.state(handle), Some(TaskState::Runnable));
    }

    #[test]
    fn event_wait_timeout_clears_the_waiter() {
        let scheduler = Scheduler::new();
        let handle = running(&scheduler);
        let event = logos_abi::keyboard_read_event_mask();
        assert_eq!(scheduler.wait_for_events(handle, event, 20), Some(true));
        assert!(scheduler.save_context(handle, 0x21b0));
        assert!(scheduler.finish(handle, FinishState::TimedBlocked));
        assert_eq!(scheduler.wake_due(19), 0);
        assert_eq!(scheduler.wake_due(20), 1);
        assert_eq!(scheduler.signal_events(event), 0);
    }

    #[test]
    fn reset_events_clears_wait_deadlines() {
        let scheduler = Scheduler::new();
        let handle = running(&scheduler);
        let event = logos_abi::keyboard_read_event_mask();
        assert_eq!(scheduler.wait_for_events(handle, event, 20), Some(true));
        assert!(scheduler.save_context(handle, 0x21d0));
        assert!(scheduler.finish(handle, FinishState::TimedBlocked));
        scheduler.reset_events();
        assert_eq!(scheduler.state(handle), Some(TaskState::Runnable));
        assert_eq!(scheduler.wake_due(20), 0);
        assert_eq!(scheduler.signal_events(event), 0);
    }

    #[test]
    fn timed_wait_wakes_at_or_after_its_deadline() {
        let scheduler = Scheduler::new();
        let handle = running(&scheduler);
        assert!(scheduler.arm_deadline(handle, 10));
        assert!(scheduler.save_context(handle, 0x2200));
        assert!(scheduler.finish(handle, FinishState::TimedBlocked));
        assert_eq!(scheduler.wake_due(9), 0);
        assert_eq!(scheduler.state(handle), Some(TaskState::Blocked));
        assert_eq!(scheduler.wake_due(10), 1);
        assert_eq!(scheduler.state(handle), Some(TaskState::Runnable));
    }

    #[test]
    fn explicit_wake_cancels_a_timed_wait() {
        let scheduler = Scheduler::new();
        let handle = running(&scheduler);
        assert!(scheduler.arm_deadline(handle, 20));
        assert!(scheduler.save_context(handle, 0x2300));
        assert!(scheduler.finish(handle, FinishState::TimedBlocked));
        assert!(scheduler.wake(handle));
        assert_eq!(scheduler.wake_due(20), 0);
        assert_eq!(scheduler.state(handle), Some(TaskState::Runnable));
    }

    #[test]
    fn wake_racing_with_timed_block_keeps_task_runnable() {
        let scheduler = Scheduler::new();
        let handle = running(&scheduler);
        assert!(scheduler.arm_deadline(handle, 30));
        assert!(scheduler.wake(handle));
        assert!(scheduler.save_context(handle, 0x2500));
        assert!(scheduler.finish(handle, FinishState::TimedBlocked));
        assert_eq!(scheduler.state(handle), Some(TaskState::Runnable));
        assert_eq!(scheduler.wake_due(30), 0);
    }

    #[test]
    fn concurrent_timer_scans_wake_a_waiter_once() {
        let scheduler = Arc::new(Scheduler::new());
        let handle = scheduler.spawn(empty).unwrap();
        scheduler.online_cpu(0);
        assert!(scheduler.claim_next(0).is_some());
        assert!(scheduler.arm_deadline(handle, 40));
        assert!(scheduler.save_context(handle, 0x2600));
        assert!(scheduler.finish(handle, FinishState::TimedBlocked));
        let mut workers = Vec::new();
        for _ in 0..MAX_CPUS {
            let scheduler = Arc::clone(&scheduler);
            workers.push(thread::spawn(move || scheduler.wake_due(40)));
        }
        let wakes: usize = workers.into_iter().map(|worker| worker.join().unwrap()).sum();
        assert_eq!(wakes, 1);
        assert_eq!(scheduler.state(handle), Some(TaskState::Runnable));
    }

    #[test]
    fn wake_pending_survives_a_concurrent_block_claim() {
        let scheduler = Scheduler::new();
        let handle = running(&scheduler);
        assert!(scheduler.wake(handle));
        assert!(scheduler.save_context(handle, 0x2400));
        assert!(scheduler.finish(handle, FinishState::Blocked));
        assert_eq!(scheduler.state(handle), Some(TaskState::Runnable));
    }

    #[test]
    fn wake_racing_with_block_keeps_task_runnable() {
        let scheduler = Arc::new(Scheduler::new());
        let handle = scheduler.spawn(empty).unwrap();
        scheduler.online_cpu(0);
        assert!(scheduler.claim_next(0).is_some());
        assert!(scheduler.save_context(handle, 0x3000));
        let wake_scheduler = Arc::clone(&scheduler);
        let wake = thread::spawn(move || wake_scheduler.wake(handle));
        assert!(wake.join().unwrap());
        assert!(scheduler.finish(handle, FinishState::Blocked));
        assert_eq!(scheduler.state(handle), Some(TaskState::Runnable));
    }

    #[test]
    fn stale_generation_cannot_touch_reused_slot() {
        let scheduler = Scheduler::new();
        let first = scheduler.spawn(empty).unwrap();
        scheduler.online_cpu(0);
        assert!(scheduler.claim_next(0).is_some());
        assert!(scheduler.save_context(first, 0x4000));
        assert!(scheduler.finish(first, FinishState::Completed));
        assert!(scheduler.reclaim_completed(first));
        let second = scheduler.spawn(empty).unwrap();
        assert_eq!(first.slot(), second.slot());
        assert_ne!(first.generation(), second.generation());
        assert!(!scheduler.wake(first));
        assert_eq!(scheduler.state(first), None);
    }

    #[test]
    fn concurrent_cpu_claims_never_duplicate_a_task() {
        let scheduler = Arc::new(Scheduler::new());
        let handle = scheduler.spawn(empty).unwrap();
        for cpu in 0..MAX_CPUS {
            scheduler.online_cpu(cpu);
        }
        let barrier = Arc::new(Barrier::new(MAX_CPUS));
        let mut workers = Vec::new();
        for cpu in 0..MAX_CPUS {
            let scheduler = Arc::clone(&scheduler);
            let barrier = Arc::clone(&barrier);
            workers.push(thread::spawn(move || {
                barrier.wait();
                scheduler.claim_next(cpu)
            }));
        }
        let claims: Vec<_> = workers.into_iter().map(|worker| worker.join().unwrap()).collect();
        assert_eq!(claims.iter().flatten().count(), 1);
        assert_eq!(claims.into_iter().flatten().next(), Some(handle));
    }

    #[test]
    fn stop_reclaims_runnable_task_without_running_old_context() {
        let scheduler = Scheduler::new();
        let handle = scheduler.spawn(empty).unwrap();
        assert!(scheduler.request_stop(handle));
        assert_eq!(scheduler.state(handle), Some(TaskState::Completed));
        assert!(scheduler.reclaim_completed(handle));
        assert_eq!(scheduler.state(handle), None);
    }

    #[test]
    fn stop_waits_for_running_task_to_publish_context() {
        let scheduler = Scheduler::new();
        let handle = running(&scheduler);
        assert!(scheduler.request_stop(handle));
        assert_eq!(scheduler.state(handle), Some(TaskState::Stopping));
        assert!(scheduler.save_context(handle, 0x4400));
        assert!(scheduler.finish(handle, FinishState::Runnable));
        assert_eq!(scheduler.state(handle), Some(TaskState::Completed));
        assert!(scheduler.reclaim_completed(handle));
    }

    #[test]
    fn capacity_is_bounded() {
        let scheduler = Scheduler::new();
        for _ in 0..MAX_TASKS {
            assert!(scheduler.spawn(empty).is_ok());
        }
        assert_eq!(scheduler.spawn(empty), Err(SpawnError::Capacity));
    }

    #[test]
    fn address_space_root_is_published_with_task_generation() {
        let scheduler = Scheduler::new();
        assert_eq!(scheduler.spawn_with_address_space(empty, 0x123), Err(SpawnError::AddressSpace));
        let handle = scheduler.spawn_with_address_space(empty, 0x40_000).unwrap();
        assert_eq!(scheduler.address_space(handle), Some(0x40_000));
        scheduler.online_cpu(0);
        assert_eq!(scheduler.claim_next(0), Some(handle));
        assert_eq!(scheduler.address_space(handle), Some(0x40_000));
        assert!(scheduler.save_context(handle, 0x5000));
        assert!(scheduler.finish(handle, FinishState::Completed));
        assert!(scheduler.reclaim_completed(handle));
        assert_eq!(scheduler.address_space(handle), None);
    }

    #[test]
    fn user_task_can_be_claimed_by_a_non_bsp_cpu() {
        let scheduler = Scheduler::new();
        let process = ProcessHandle::from_raw(0x100);
        let root = crate::process::AddressSpaceRoot::new(0x80_000).unwrap();
        let launch = UserLaunch::new(0x4000, 0x9000, root).unwrap();
        let handle = scheduler.spawn_user(empty, process, launch).unwrap();
        scheduler.online_cpu(1);

        assert_eq!(scheduler.claim_next(1), Some(handle));
        assert_eq!(scheduler.state(handle), Some(TaskState::Running));
    }

    #[test]
    fn user_task_migrates_after_context_publication() {
        let scheduler = Scheduler::new();
        let process = ProcessHandle::from_raw(0x100);
        let root = crate::process::AddressSpaceRoot::new(0x80_000).unwrap();
        let launch = UserLaunch::new(0x4000, 0x9000, root).unwrap();
        let handle = scheduler.spawn_user(empty, process, launch).unwrap();
        scheduler.online_cpu(0);
        scheduler.online_cpu(1);

        assert_eq!(scheduler.claim_next(0), Some(handle));
        assert!(scheduler.save_context(handle, 0x5000));
        assert!(scheduler.finish(handle, FinishState::Runnable));
        assert_eq!(scheduler.claim_next(1), Some(handle));
        assert_eq!(scheduler.state(handle), Some(TaskState::Running));
    }

    #[test]
    fn loaded_user_launch_is_published_with_task_generation() {
        let scheduler = Scheduler::new();
        let process = ProcessHandle::from_raw(0x100);
        let root = crate::process::AddressSpaceRoot::new(0x80_000).unwrap();
        let launch = UserLaunch::new(0x4000, 0x9000, root).unwrap();
        let handle = scheduler.spawn_user(empty, process, launch).unwrap();
        let scheduled = scheduler.user_launch(handle).unwrap();
        assert_eq!(scheduled.process(), process);
        assert_eq!(scheduled.entry(), 0x4000);
        assert_eq!(scheduled.stack_top(), 0x9000);
        assert_eq!(scheduled.address_space_root(), 0x80_000);
    }

    #[test]
    fn user_launch_rejects_null_process_handles() {
        let scheduler = Scheduler::new();
        let process = ProcessHandle::from_raw(0);
        let root = crate::process::AddressSpaceRoot::new(0x80_000).unwrap();
        let launch = UserLaunch::new(0x4000, 0x9000, root).unwrap();
        assert_eq!(scheduler.spawn_user(empty, process, launch), Err(SpawnError::UserLaunch));
    }

    #[test]
    fn concurrent_claims_never_duplicate_a_slot() {
        let scheduler = Arc::new(Scheduler::new());
        for cpu in 0..MAX_CPUS {
            scheduler.online_cpu(cpu);
        }
        let handle = scheduler.spawn(empty).unwrap();
        let mut workers = Vec::new();
        for cpu in 0..MAX_CPUS {
            let scheduler = Arc::clone(&scheduler);
            workers.push(thread::spawn(move || scheduler.claim_next(cpu)));
        }
        let claims = workers.into_iter().filter_map(|worker| worker.join().unwrap()).count();
        assert_eq!(claims, 1);
        assert_eq!(scheduler.state(handle), Some(TaskState::Running));
    }
}
