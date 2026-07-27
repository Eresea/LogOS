use crate::capabilities::{Capability, CapabilityKind, CapabilityManager};

const MAX_MANIFESTS: usize = 4;

pub const SUPERVISOR: &[u8] = b"supervisor";
pub const VIRTIO_BALLOON: &[u8] = b"virtio-balloon";

pub struct Manifest {
    pub name: &'static [u8],
    pub dependencies: &'static [&'static [u8]],
    pub capabilities: &'static [CapabilityKind],
    pub restart: RestartPolicy,
}

#[derive(Clone, Copy)]
pub struct RestartPolicy {
    retries: u8,
    backoff_ticks: u64,
}

const SUPERVISOR_MANIFEST: Manifest = Manifest {
    name: SUPERVISOR,
    dependencies: &[],
    capabilities: &[],
    restart: RestartPolicy { retries: 0, backoff_ticks: 0 },
};
const VIRTIO_MANIFEST: Manifest = Manifest {
    name: VIRTIO_BALLOON,
    dependencies: &[SUPERVISOR],
    capabilities: &[CapabilityKind::Service],
    restart: RestartPolicy { retries: 3, backoff_ticks: 2 },
};
const BOOT_MANIFESTS: &[Manifest] = &[SUPERVISOR_MANIFEST, VIRTIO_MANIFEST];

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Error {
    Duplicate,
    MissingDependency,
    Cycle,
}

pub struct Plan {
    order: [Option<&'static Manifest>; MAX_MANIFESTS],
    len: usize,
}

#[derive(Clone, Copy)]
struct Heartbeat {
    name: &'static [u8],
    timeout: u64,
    last: u64,
}

pub struct Health {
    heartbeats: [Option<Heartbeat>; MAX_MANIFESTS],
}

impl Health {
    pub const fn new() -> Self {
        Self { heartbeats: [None; MAX_MANIFESTS] }
    }

    pub fn watch(&mut self, plan: &Plan, name: &'static [u8], timeout: u64, tick: u64) -> bool {
        if timeout == 0 || !plan.starts(name) || self.find(name).is_some() {
            return false;
        }
        for slot in &mut self.heartbeats {
            if slot.is_none() {
                *slot = Some(Heartbeat { name, timeout, last: tick });
                return true;
            }
        }
        false
    }

    pub fn beat(&mut self, name: &[u8], tick: u64) -> bool {
        let Some(index) = self.find(name) else {
            return false;
        };
        self.heartbeats[index].as_mut().unwrap().last = tick;
        true
    }

    pub fn healthy(&self, name: &[u8], tick: u64) -> bool {
        self.find(name).is_some_and(|index| {
            let heartbeat = self.heartbeats[index].unwrap();
            tick.wrapping_sub(heartbeat.last) <= heartbeat.timeout
        })
    }

    fn find(&self, name: &[u8]) -> Option<usize> {
        self.heartbeats
            .iter()
            .position(|heartbeat| heartbeat.is_some_and(|heartbeat| heartbeat.name == name))
    }
}

impl Plan {
    pub fn build(manifests: &'static [Manifest]) -> Result<Self, Error> {
        if manifests.len() > MAX_MANIFESTS {
            return Err(Error::Cycle);
        }
        for (index, manifest) in manifests.iter().enumerate() {
            if manifest.name.is_empty()
                || manifests[..index].iter().any(|other| other.name == manifest.name)
            {
                return Err(Error::Duplicate);
            }
            if manifest
                .dependencies
                .iter()
                .any(|dependency| !manifests.iter().any(|other| other.name == *dependency))
            {
                return Err(Error::MissingDependency);
            }
        }
        let mut plan = Self { order: [None; MAX_MANIFESTS], len: 0 };
        while plan.len < manifests.len() {
            let mut progressed = false;
            for manifest in manifests {
                if !plan.contains(manifest.name)
                    && manifest.dependencies.iter().all(|dependency| plan.contains(dependency))
                {
                    plan.order[plan.len] = Some(manifest);
                    plan.len += 1;
                    progressed = true;
                }
            }
            if !progressed {
                return Err(Error::Cycle);
            }
        }
        Ok(plan)
    }

    pub fn starts(&self, name: &[u8]) -> bool {
        self.contains(name)
    }

    fn contains(&self, name: &[u8]) -> bool {
        self.order[..self.len]
            .iter()
            .any(|manifest| manifest.is_some_and(|manifest| manifest.name == name))
    }

    fn manifest(&self, name: &[u8]) -> Option<&'static Manifest> {
        self.order[..self.len].iter().flatten().find(|manifest| manifest.name == name).copied()
    }

    pub fn grant(
        &self,
        name: &[u8],
        manager: &mut CapabilityManager,
        kind: CapabilityKind,
    ) -> Option<Capability> {
        self.manifest(name)
            .filter(|manifest| manifest.capabilities.contains(&kind))
            .and_then(|_| manager.grant(kind))
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum LifecycleState {
    Running,
    Backoff,
    Stopped,
}

pub struct Lifecycle {
    policy: RestartPolicy,
    retries: u8,
    retry_at: u64,
    state: LifecycleState,
}

impl Lifecycle {
    pub fn new(plan: &Plan, name: &[u8]) -> Option<Self> {
        plan.manifest(name).map(|manifest| Self {
            policy: manifest.restart,
            retries: 0,
            retry_at: 0,
            state: LifecycleState::Running,
        })
    }

    pub fn restart(&mut self, tick: u64) -> bool {
        self.schedule(tick)
    }

    pub fn failed(&mut self, tick: u64) -> bool {
        if self.state != LifecycleState::Running || self.retries >= self.policy.retries {
            self.state = LifecycleState::Stopped;
            return false;
        }
        self.retries += 1;
        self.schedule(tick)
    }

    pub fn due(&mut self, tick: u64) -> bool {
        if self.state == LifecycleState::Backoff && tick.wrapping_sub(self.retry_at) < (1 << 63) {
            self.state = LifecycleState::Running;
            return true;
        }
        false
    }

    pub fn shutdown(&mut self) {
        self.state = LifecycleState::Stopped;
    }

    fn schedule(&mut self, tick: u64) -> bool {
        if self.state != LifecycleState::Running || self.policy.backoff_ticks == 0 {
            return false;
        }
        let shift = self.retries.saturating_sub(1).min(3);
        self.retry_at = tick.saturating_add(self.policy.backoff_ticks << shift);
        self.state = LifecycleState::Backoff;
        true
    }
}

pub fn boot_plan() -> Result<Plan, Error> {
    Plan::build(BOOT_MANIFESTS)
}

pub fn self_check() -> bool {
    const A: &[u8] = b"a";
    const B: &[u8] = b"b";
    const OK: &[Manifest] = &[
        Manifest {
            name: B,
            dependencies: &[A],
            capabilities: &[],
            restart: RestartPolicy { retries: 1, backoff_ticks: 1 },
        },
        Manifest {
            name: A,
            dependencies: &[],
            capabilities: &[],
            restart: RestartPolicy { retries: 1, backoff_ticks: 1 },
        },
    ];
    const MISSING: &[Manifest] = &[Manifest {
        name: A,
        dependencies: &[B],
        capabilities: &[],
        restart: RestartPolicy { retries: 1, backoff_ticks: 1 },
    }];
    const CYCLE: &[Manifest] = &[
        Manifest {
            name: A,
            dependencies: &[B],
            capabilities: &[],
            restart: RestartPolicy { retries: 1, backoff_ticks: 1 },
        },
        Manifest {
            name: B,
            dependencies: &[A],
            capabilities: &[],
            restart: RestartPolicy { retries: 1, backoff_ticks: 1 },
        },
    ];
    Plan::build(OK).is_ok_and(|plan| plan.starts(A) && plan.starts(B))
        && matches!(Plan::build(MISSING), Err(Error::MissingDependency))
        && matches!(Plan::build(CYCLE), Err(Error::Cycle))
}

pub fn grant_self_check() -> bool {
    let Ok(plan) = boot_plan() else {
        return false;
    };
    let mut manager = CapabilityManager::new();
    let Some(service) = plan.grant(VIRTIO_BALLOON, &mut manager, CapabilityKind::Service) else {
        return false;
    };
    manager.allows(service, CapabilityKind::Service)
        && plan.grant(VIRTIO_BALLOON, &mut manager, CapabilityKind::Debug).is_none()
        && plan.grant(SUPERVISOR, &mut manager, CapabilityKind::Service).is_none()
}

pub fn lifecycle_self_check() -> bool {
    let Ok(plan) = boot_plan() else {
        return false;
    };
    let Some(mut lifecycle) = Lifecycle::new(&plan, VIRTIO_BALLOON) else {
        return false;
    };
    let Some(mut shutdown) = Lifecycle::new(&plan, VIRTIO_BALLOON) else {
        return false;
    };
    lifecycle.restart(10)
        && !lifecycle.due(11)
        && lifecycle.due(12)
        && lifecycle.failed(20)
        && !lifecycle.due(21)
        && lifecycle.due(22)
        && lifecycle.failed(30)
        && lifecycle.due(34)
        && lifecycle.failed(40)
        && lifecycle.due(48)
        && !lifecycle.failed(50)
        && !lifecycle.restart(51)
        && {
            shutdown.shutdown();
            !shutdown.restart(1)
        }
}

pub fn health_self_check() -> bool {
    let Ok(plan) = boot_plan() else {
        return false;
    };
    let mut health = Health::new();
    health.watch(&plan, VIRTIO_BALLOON, 2, 10)
        && health.healthy(VIRTIO_BALLOON, 12)
        && !health.healthy(VIRTIO_BALLOON, 13)
        && health.beat(VIRTIO_BALLOON, 13)
        && health.healthy(VIRTIO_BALLOON, 15)
        && !health.watch(&plan, VIRTIO_BALLOON, 2, 15)
}
