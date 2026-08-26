use crate::runtime::UiNodeHandle;
use crate::template::UiBindingProperty;

pub const MAX_UI_DEPENDENCIES: usize = 4;
pub const MAX_UI_DEPENDENCY_RECORDS: usize = 16;
pub const MAX_UI_INVALIDATIONS: usize = 32;
pub const MAX_UI_TRACE_ENTRIES: usize = 64;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct UiSignalId(u16);

impl UiSignalId {
    pub const EMPTY: Self = Self(u16::MAX);

    pub const fn new(index: u16) -> Self {
        Self(index)
    }

    pub const fn index(self) -> u16 {
        self.0
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct UiSignalChange {
    pub signal: UiSignalId,
    pub revision: u32,
}

pub trait UiReadable<T: Copy> {
    fn read(&self) -> T;
    fn signal_id(&self) -> UiSignalId;
    fn revision(&self) -> u32;
}

pub trait UiWritable<T: Copy + PartialEq>: UiReadable<T> {
    fn write(&mut self, value: T) -> Option<UiSignalChange>;
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct UiSignal<T: Copy + PartialEq> {
    id: UiSignalId,
    value: T,
    revision: u32,
}

impl<T: Copy + PartialEq> UiSignal<T> {
    pub const fn new(id: UiSignalId, value: T) -> Self {
        Self { id, value, revision: 1 }
    }

    pub const fn value(&self) -> T {
        self.value
    }

    pub const fn revision(&self) -> u32 {
        self.revision
    }

    pub fn set(&mut self, value: T) -> Option<UiSignalChange> {
        if self.value == value {
            return None;
        }
        self.value = value;
        self.bump_revision();
        Some(UiSignalChange { signal: self.id, revision: self.revision })
    }

    fn bump_revision(&mut self) {
        self.revision = self.revision.wrapping_add(1).max(1);
    }
}

impl<T: Copy + PartialEq> UiReadable<T> for UiSignal<T> {
    fn read(&self) -> T {
        self.value
    }

    fn signal_id(&self) -> UiSignalId {
        self.id
    }

    fn revision(&self) -> u32 {
        self.revision
    }
}

impl<T: Copy + PartialEq> UiWritable<T> for UiSignal<T> {
    fn write(&mut self, value: T) -> Option<UiSignalChange> {
        self.set(value)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct UiComputed<T: Copy + PartialEq> {
    id: UiSignalId,
    value: T,
    revision: u32,
}

impl<T: Copy + PartialEq> UiComputed<T> {
    pub const fn new(id: UiSignalId, value: T) -> Self {
        Self { id, value, revision: 1 }
    }

    pub const fn value(&self) -> T {
        self.value
    }

    pub const fn revision(&self) -> u32 {
        self.revision
    }

    pub fn update(&mut self, value: T) -> Option<UiSignalChange> {
        if self.value == value {
            return None;
        }
        self.value = value;
        self.revision = self.revision.wrapping_add(1).max(1);
        Some(UiSignalChange { signal: self.id, revision: self.revision })
    }
}

impl<T: Copy + PartialEq> UiReadable<T> for UiComputed<T> {
    fn read(&self) -> T {
        self.value
    }

    fn signal_id(&self) -> UiSignalId {
        self.id
    }

    fn revision(&self) -> u32 {
        self.revision
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct UiDependencySet {
    entries: [UiSignalId; MAX_UI_DEPENDENCIES],
    len: u8,
}

impl UiDependencySet {
    pub const EMPTY: Self = Self { entries: [UiSignalId::new(0); MAX_UI_DEPENDENCIES], len: 0 };

    pub fn add(&mut self, signal: UiSignalId) -> bool {
        if self.contains(signal) {
            return true;
        }
        if usize::from(self.len) == MAX_UI_DEPENDENCIES {
            return false;
        }
        self.entries[usize::from(self.len)] = signal;
        self.len += 1;
        true
    }

    pub const fn len(&self) -> usize {
        self.len as usize
    }

    pub const fn is_empty(&self) -> bool {
        self.len == 0
    }

    pub fn contains(&self, signal: UiSignalId) -> bool {
        self.entries[..usize::from(self.len)].contains(&signal)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct UiBindingTarget {
    pub node: UiNodeHandle,
    pub property: UiBindingProperty,
}

impl UiBindingTarget {
    pub const EMPTY: Self = Self { node: UiNodeHandle::EMPTY, property: UiBindingProperty::Value };
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum UiInvalidationKind {
    Paint = 1,
    Layout = 2,
    Rebuild = 3,
}

impl UiInvalidationKind {
    const fn merge(self, other: Self) -> Self {
        if (self as u8) >= (other as u8) { self } else { other }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct UiInvalidation {
    pub target: UiBindingTarget,
    pub kind: UiInvalidationKind,
    pub revision: u32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum UiReactiveError {
    Capacity,
    QueueFull,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct UiDependencyRecord {
    target: UiBindingTarget,
    dependencies: UiDependencySet,
    kind: UiInvalidationKind,
}

pub struct UiInvalidationQueue {
    entries: [UiInvalidation; MAX_UI_INVALIDATIONS],
    len: usize,
}

impl UiInvalidationQueue {
    pub const fn new() -> Self {
        Self {
            entries: [UiInvalidation {
                target: UiBindingTarget {
                    node: UiNodeHandle::EMPTY,
                    property: UiBindingProperty::Value,
                },
                kind: UiInvalidationKind::Paint,
                revision: 0,
            }; MAX_UI_INVALIDATIONS],
            len: 0,
        }
    }

    pub const fn len(&self) -> usize {
        self.len
    }

    pub const fn is_empty(&self) -> bool {
        self.len == 0
    }

    pub const fn peek(&self) -> Option<UiInvalidation> {
        if self.len == 0 { None } else { Some(self.entries[0]) }
    }

    pub fn pop(&mut self) -> Option<UiInvalidation> {
        if self.len == 0 {
            return None;
        }
        let entry = self.entries[0];
        self.entries.copy_within(1..self.len, 0);
        self.len -= 1;
        Some(entry)
    }

    fn contains_target(&self, target: UiBindingTarget) -> bool {
        self.entries[..self.len].iter().any(|entry| entry.target == target)
    }

    fn remaining(&self) -> usize {
        MAX_UI_INVALIDATIONS - self.len
    }

    fn push(&mut self, invalidation: UiInvalidation) -> Result<(), UiReactiveError> {
        if let Some(existing) =
            self.entries.iter_mut().take(self.len).find(|entry| entry.target == invalidation.target)
        {
            existing.kind = existing.kind.merge(invalidation.kind);
            existing.revision = invalidation.revision;
            return Ok(());
        }
        if self.len == MAX_UI_INVALIDATIONS {
            return Err(UiReactiveError::QueueFull);
        }
        self.entries[self.len] = invalidation;
        self.len += 1;
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum UiTraceKind {
    SignalChanged = 1,
    InvalidationsQueued = 2,
    QueueBackpressure = 3,
    CommitStarted = 4,
    InvalidationApplied = 5,
    CommitFinished = 6,
    RefreshRequested = 7,
    RefreshTaken = 8,
    CommitRejected = 9,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct UiTraceEntry {
    pub sequence: u32,
    pub kind: UiTraceKind,
    pub signal: UiSignalId,
    pub target: UiBindingTarget,
    pub invalidation: UiInvalidationKind,
    pub revision: u32,
    pub count: u16,
}

impl UiTraceEntry {
    const EMPTY: Self = Self {
        sequence: 0,
        kind: UiTraceKind::SignalChanged,
        signal: UiSignalId::EMPTY,
        target: UiBindingTarget::EMPTY,
        invalidation: UiInvalidationKind::Paint,
        revision: 0,
        count: 0,
    };

    const fn event(kind: UiTraceKind, count: usize) -> Self {
        Self { kind, count: count as u16, ..Self::EMPTY }
    }

    const fn signal(change: UiSignalChange) -> Self {
        Self {
            kind: UiTraceKind::SignalChanged,
            signal: change.signal,
            revision: change.revision,
            ..Self::EMPTY
        }
    }

    const fn invalidation(invalidation: UiInvalidation) -> Self {
        Self {
            kind: UiTraceKind::InvalidationApplied,
            target: invalidation.target,
            invalidation: invalidation.kind,
            revision: invalidation.revision,
            ..Self::EMPTY
        }
    }
}

pub struct UiDebugTrace {
    entries: [UiTraceEntry; MAX_UI_TRACE_ENTRIES],
    next: usize,
    len: usize,
    sequence: u32,
}

impl UiDebugTrace {
    pub const fn new() -> Self {
        Self { entries: [UiTraceEntry::EMPTY; MAX_UI_TRACE_ENTRIES], next: 0, len: 0, sequence: 0 }
    }

    pub const fn len(&self) -> usize {
        self.len
    }

    pub const fn is_empty(&self) -> bool {
        self.len == 0
    }

    pub fn entry(&self, index: usize) -> Option<UiTraceEntry> {
        if index >= self.len {
            return None;
        }
        let oldest = (self.next + MAX_UI_TRACE_ENTRIES - self.len) % MAX_UI_TRACE_ENTRIES;
        Some(self.entries[(oldest + index) % MAX_UI_TRACE_ENTRIES])
    }

    pub fn clear(&mut self) {
        self.next = 0;
        self.len = 0;
        self.sequence = 0;
    }

    fn record(&mut self, mut entry: UiTraceEntry) {
        self.sequence = self.sequence.wrapping_add(1).max(1);
        entry.sequence = self.sequence;
        self.entries[self.next] = entry;
        self.next = (self.next + 1) % MAX_UI_TRACE_ENTRIES;
        self.len = self.len.saturating_add(1).min(MAX_UI_TRACE_ENTRIES);
    }
}

impl Default for UiDebugTrace {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum UiCommitError {
    ApplyRejected,
}

pub struct UiCommitCoordinator {
    graph: UiDependencyGraph,
    queue: UiInvalidationQueue,
    trace: UiDebugTrace,
    refresh_deadline: Option<u64>,
}

impl UiCommitCoordinator {
    pub const fn new() -> Self {
        Self {
            graph: UiDependencyGraph::new(),
            queue: UiInvalidationQueue::new(),
            trace: UiDebugTrace::new(),
            refresh_deadline: None,
        }
    }

    pub fn watch(
        &mut self,
        target: UiBindingTarget,
        dependencies: UiDependencySet,
        kind: UiInvalidationKind,
    ) -> Result<(), UiReactiveError> {
        self.graph.watch(target, dependencies, kind)
    }

    pub fn unwatch(&mut self, target: UiBindingTarget) -> bool {
        self.graph.unwatch(target)
    }

    pub fn publish(&mut self, change: UiSignalChange) -> Result<usize, UiReactiveError> {
        self.trace.record(UiTraceEntry::signal(change));
        let result = self.graph.invalidate(change, &mut self.queue);
        match result {
            Ok(routed) => {
                if routed != 0 {
                    self.trace
                        .record(UiTraceEntry::event(UiTraceKind::InvalidationsQueued, routed));
                    self.request_refresh(0);
                }
                Ok(routed)
            }
            Err(error) => {
                self.trace
                    .record(UiTraceEntry::event(UiTraceKind::QueueBackpressure, self.queue.len()));
                Err(error)
            }
        }
    }

    pub fn pending_invalidations(&self) -> usize {
        self.queue.len()
    }

    pub fn request_refresh(&mut self, deadline: u64) {
        let replace = self.refresh_deadline.is_none_or(|current| deadline < current);
        if replace {
            self.refresh_deadline = Some(deadline);
            self.trace.record(UiTraceEntry::event(UiTraceKind::RefreshRequested, 1));
        }
    }

    pub fn take_refresh(&mut self) -> Option<u64> {
        let deadline = self.refresh_deadline.take();
        if deadline.is_some() {
            self.trace.record(UiTraceEntry::event(UiTraceKind::RefreshTaken, 1));
        }
        deadline
    }

    pub fn commit<F>(&mut self, mut apply: F) -> Result<usize, UiCommitError>
    where
        F: FnMut(UiInvalidation) -> bool,
    {
        if self.queue.is_empty() {
            return Ok(0);
        }
        self.trace.record(UiTraceEntry::event(UiTraceKind::CommitStarted, self.queue.len()));
        let mut committed = 0;
        while let Some(invalidation) = self.queue.peek() {
            if !apply(invalidation) {
                self.trace
                    .record(UiTraceEntry::event(UiTraceKind::CommitRejected, self.queue.len()));
                return Err(UiCommitError::ApplyRejected);
            }
            let _ = self.queue.pop();
            self.trace.record(UiTraceEntry::invalidation(invalidation));
            committed += 1;
        }
        self.trace.record(UiTraceEntry::event(UiTraceKind::CommitFinished, committed));
        Ok(committed)
    }

    pub const fn trace(&self) -> &UiDebugTrace {
        &self.trace
    }
}

impl Default for UiCommitCoordinator {
    fn default() -> Self {
        Self::new()
    }
}

impl Default for UiInvalidationQueue {
    fn default() -> Self {
        Self::new()
    }
}

pub struct UiDependencyGraph {
    records: [UiDependencyRecord; MAX_UI_DEPENDENCY_RECORDS],
    len: usize,
}

impl UiDependencyGraph {
    pub const fn new() -> Self {
        Self {
            records: [UiDependencyRecord {
                target: UiBindingTarget {
                    node: UiNodeHandle::EMPTY,
                    property: UiBindingProperty::Value,
                },
                dependencies: UiDependencySet::EMPTY,
                kind: UiInvalidationKind::Paint,
            }; MAX_UI_DEPENDENCY_RECORDS],
            len: 0,
        }
    }

    pub const fn len(&self) -> usize {
        self.len
    }

    pub const fn is_empty(&self) -> bool {
        self.len == 0
    }

    pub fn watch(
        &mut self,
        target: UiBindingTarget,
        dependencies: UiDependencySet,
        kind: UiInvalidationKind,
    ) -> Result<(), UiReactiveError> {
        if let Some(record) =
            self.records.iter_mut().take(self.len).find(|record| record.target == target)
        {
            *record = UiDependencyRecord { target, dependencies, kind };
            return Ok(());
        }
        if self.len == MAX_UI_DEPENDENCY_RECORDS {
            return Err(UiReactiveError::Capacity);
        }
        self.records[self.len] = UiDependencyRecord { target, dependencies, kind };
        self.len += 1;
        Ok(())
    }

    pub fn unwatch(&mut self, target: UiBindingTarget) -> bool {
        let Some(index) =
            self.records[..self.len].iter().position(|record| record.target == target)
        else {
            return false;
        };
        self.records.copy_within(index + 1..self.len, index);
        self.len -= 1;
        true
    }

    pub fn invalidate(
        &self,
        change: UiSignalChange,
        queue: &mut UiInvalidationQueue,
    ) -> Result<usize, UiReactiveError> {
        let needed = self.records[..self.len]
            .iter()
            .filter(|record| {
                record.dependencies.contains(change.signal) && !queue.contains_target(record.target)
            })
            .count();
        if needed > queue.remaining() {
            return Err(UiReactiveError::QueueFull);
        }
        let mut routed = 0;
        for record in self.records[..self.len]
            .iter()
            .filter(|record| record.dependencies.contains(change.signal))
        {
            queue.push(UiInvalidation {
                target: record.target,
                kind: record.kind,
                revision: change.revision,
            })?;
            routed += 1;
        }
        Ok(routed)
    }
}

impl Default for UiDependencyGraph {
    fn default() -> Self {
        Self::new()
    }
}

const _: () = assert!(core::mem::size_of::<UiDependencyGraph>() <= 1024);
const _: () = assert!(core::mem::size_of::<UiInvalidationQueue>() <= 1024);
const _: () = assert!(core::mem::size_of::<UiDebugTrace>() <= 4096);
const _: () = assert!(core::mem::size_of::<UiCommitCoordinator>() <= 8192);

#[cfg(test)]
mod tests {
    use super::*;

    fn target(slot: u16) -> UiBindingTarget {
        UiBindingTarget {
            node: UiNodeHandle { slot, generation: 1 },
            property: UiBindingProperty::Disabled,
        }
    }

    fn dependencies(signal: UiSignalId) -> UiDependencySet {
        let mut dependencies = UiDependencySet::EMPTY;
        assert!(dependencies.add(signal));
        dependencies
    }

    #[test]
    fn signals_only_publish_real_changes_and_computed_values_are_readable() {
        let signal_id = UiSignalId::new(7);
        let mut signal = UiSignal::new(signal_id, 10u16);
        assert_eq!(signal.set(10), None);
        let change = signal.set(11).unwrap();
        assert_eq!(change.signal, signal_id);
        assert_eq!(signal.value(), 11);

        let mut computed = UiComputed::new(UiSignalId::new(8), 22u16);
        assert_eq!(computed.read(), 22);
        assert_eq!(computed.update(22), None);
        assert!(computed.update(23).is_some());
    }

    #[test]
    fn dependency_sets_are_deduplicated_and_bounded() {
        let mut dependencies = UiDependencySet::EMPTY;
        for index in 0..MAX_UI_DEPENDENCIES {
            assert!(dependencies.add(UiSignalId::new(index as u16)));
        }
        assert!(dependencies.add(UiSignalId::new(0)));
        assert!(!dependencies.add(UiSignalId::new(99)));
        assert_eq!(dependencies.len(), MAX_UI_DEPENDENCIES);
    }

    #[test]
    fn graph_routes_only_matching_signals_and_coalesces_work() {
        let signal = UiSignalId::new(1);
        let other = UiSignalId::new(2);
        let first = target(1);
        let second = target(2);
        let mut graph = UiDependencyGraph::new();
        graph.watch(first, dependencies(signal), UiInvalidationKind::Paint).unwrap();
        graph.watch(second, dependencies(other), UiInvalidationKind::Layout).unwrap();
        let mut queue = UiInvalidationQueue::new();

        assert_eq!(graph.invalidate(UiSignalChange { signal, revision: 4 }, &mut queue), Ok(1));
        assert_eq!(graph.invalidate(UiSignalChange { signal, revision: 5 }, &mut queue), Ok(1));
        assert_eq!(queue.len(), 1);
        let invalidation = queue.pop().unwrap();
        assert_eq!(invalidation.target, first);
        assert_eq!(invalidation.revision, 5);
        assert_eq!(
            graph.invalidate(UiSignalChange { signal: other, revision: 1 }, &mut queue),
            Ok(1)
        );
        assert_eq!(queue.pop().unwrap().target, second);
    }

    #[test]
    fn coalescing_keeps_the_strongest_invalidation_kind() {
        let mut queue = UiInvalidationQueue::new();
        let value =
            UiInvalidation { target: target(3), kind: UiInvalidationKind::Paint, revision: 1 };
        queue.push(value).unwrap();
        queue
            .push(UiInvalidation { kind: UiInvalidationKind::Rebuild, revision: 2, ..value })
            .unwrap();
        let result = queue.pop().unwrap();
        assert_eq!(result.kind, UiInvalidationKind::Rebuild);
        assert_eq!(result.revision, 2);
    }

    #[test]
    fn graph_backpressure_is_transactional() {
        let signal = UiSignalId::new(3);
        let mut graph = UiDependencyGraph::new();
        graph.watch(target(1), dependencies(signal), UiInvalidationKind::Paint).unwrap();
        graph.watch(target(2), dependencies(signal), UiInvalidationKind::Paint).unwrap();
        let mut queue = UiInvalidationQueue::new();
        for index in 0..MAX_UI_INVALIDATIONS - 1 {
            queue
                .push(UiInvalidation {
                    target: target(100 + index as u16),
                    kind: UiInvalidationKind::Paint,
                    revision: 1,
                })
                .unwrap();
        }
        assert_eq!(
            graph.invalidate(UiSignalChange { signal, revision: 1 }, &mut queue),
            Err(UiReactiveError::QueueFull)
        );
        assert_eq!(queue.len(), MAX_UI_INVALIDATIONS - 1);
    }

    #[test]
    fn coordinator_commits_targeted_work_and_consumes_one_shot_refresh() {
        let signal = UiSignalId::new(9);
        let target = target(4);
        let mut coordinator = UiCommitCoordinator::new();
        coordinator.watch(target, dependencies(signal), UiInvalidationKind::Paint).unwrap();

        assert_eq!(coordinator.publish(UiSignalChange { signal, revision: 2 }), Ok(1));
        assert_eq!(coordinator.pending_invalidations(), 1);
        assert_eq!(coordinator.take_refresh(), Some(0));
        assert_eq!(coordinator.take_refresh(), None);

        let mut applied = [UiBindingTarget::EMPTY; 1];
        let mut count = 0;
        assert_eq!(
            coordinator.commit(|invalidation| {
                applied[count] = invalidation.target;
                count += 1;
                true
            }),
            Ok(1)
        );
        assert_eq!(applied[0], target);
        assert_eq!(coordinator.pending_invalidations(), 0);
        assert!(
            coordinator
                .trace()
                .entry(coordinator.trace().len() - 1)
                .is_some_and(|entry| entry.kind == UiTraceKind::CommitFinished)
        );
    }

    #[test]
    fn coordinator_coalesces_changes_and_preserves_failed_work() {
        let signal = UiSignalId::new(10);
        let mut coordinator = UiCommitCoordinator::new();
        coordinator.watch(target(5), dependencies(signal), UiInvalidationKind::Layout).unwrap();

        assert_eq!(coordinator.publish(UiSignalChange { signal, revision: 1 }), Ok(1));
        assert_eq!(coordinator.publish(UiSignalChange { signal, revision: 2 }), Ok(1));
        assert_eq!(coordinator.pending_invalidations(), 1);
        assert_eq!(coordinator.commit(|_| false), Err(UiCommitError::ApplyRejected));
        assert_eq!(coordinator.pending_invalidations(), 1);

        let mut revision = 0;
        assert_eq!(
            coordinator.commit(|invalidation| {
                revision = invalidation.revision;
                true
            }),
            Ok(1)
        );
        assert_eq!(revision, 2);
    }

    #[test]
    fn debug_trace_is_bounded_and_keeps_oldest_retained_order() {
        let mut trace = UiDebugTrace::new();
        for index in 0..MAX_UI_TRACE_ENTRIES + 2 {
            trace.record(UiTraceEntry::event(UiTraceKind::CommitFinished, index));
        }
        assert_eq!(trace.len(), MAX_UI_TRACE_ENTRIES);
        assert_eq!(trace.entry(0).unwrap().count, 2);
        assert_eq!(
            trace.entry(MAX_UI_TRACE_ENTRIES - 1).unwrap().count,
            (MAX_UI_TRACE_ENTRIES + 1) as u16
        );
    }
}
