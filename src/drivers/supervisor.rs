use crate::debug;
use logos_core::capabilities::{Capability, CapabilityKind, CapabilityManager};

const MAX_MANIFESTS: usize = 4;

pub const SUPERVISOR: &[u8] = b"supervisor";
pub const VIRTIO_BALLOON: &[u8] = b"virtio-balloon";
pub const VIRTIO_BLOCK: &[u8] = b"virtio-block";
pub const STORAGE: &[u8] = b"storage";

#[derive(Clone, Copy)]
pub enum Profile {
    Normal,
    Recovery,
    Diagnostics,
}

#[derive(Clone, Copy)]
pub struct Profiles(u8);

impl Profiles {
    const NORMAL: u8 = 1;
    const RECOVERY: u8 = 2;
    const DIAGNOSTICS: u8 = 4;

    pub const ALL: Self = Self(Self::NORMAL | Self::RECOVERY | Self::DIAGNOSTICS);
    pub const NORMAL_RECOVERY: Self = Self(Self::NORMAL | Self::RECOVERY);

    const fn includes(self, profile: Profile) -> bool {
        self.0
            & match profile {
                Profile::Normal => Self::NORMAL,
                Profile::Recovery => Self::RECOVERY,
                Profile::Diagnostics => Self::DIAGNOSTICS,
            }
            != 0
    }
}

#[derive(Clone, Copy)]
pub enum StartStage {
    Protocol,
    Capability,
    Register,
    Bind,
    Task,
}

impl StartStage {
    const fn message(self) -> &'static [u8] {
        match self {
            Self::Protocol => b"protocol",
            Self::Capability => b"capability",
            Self::Register => b"register",
            Self::Bind => b"bind",
            Self::Task => b"task",
        }
    }
}

pub fn report_start_failure(name: &[u8], stage: StartStage) {
    debug::write(b"LogOS: service start failed ");
    debug::write(name);
    debug::write(b" at ");
    debug::write_line(stage.message());
}

pub struct Manifest {
    pub name: &'static [u8],
    pub dependencies: &'static [&'static [u8]],
    pub capabilities: &'static [CapabilityKind],
    pub protocol: Protocol,
    pub restart: RestartPolicy,
    pub profiles: Profiles,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Protocol {
    pub abi: u16,
    pub version: u16,
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
    protocol: Protocol { abi: 1, version: 0 },
    restart: RestartPolicy { retries: 0, backoff_ticks: 0 },
    profiles: Profiles::ALL,
};
const VIRTIO_MANIFEST: Manifest = Manifest {
    name: VIRTIO_BALLOON,
    dependencies: &[SUPERVISOR],
    capabilities: &[CapabilityKind::Service],
    protocol: Protocol { abi: 1, version: 0 },
    restart: RestartPolicy { retries: 3, backoff_ticks: 2 },
    profiles: Profiles::NORMAL_RECOVERY,
};
const BLOCK_MANIFEST: Manifest = Manifest {
    name: VIRTIO_BLOCK,
    dependencies: &[SUPERVISOR],
    capabilities: &[CapabilityKind::Service, CapabilityKind::Block, CapabilityKind::Memory],
    protocol: Protocol { abi: 1, version: 0 },
    restart: RestartPolicy { retries: 3, backoff_ticks: 2 },
    profiles: Profiles::NORMAL_RECOVERY,
};
const STORAGE_MANIFEST: Manifest = Manifest {
    name: STORAGE,
    dependencies: &[VIRTIO_BLOCK],
    capabilities: &[
        CapabilityKind::Service,
        CapabilityKind::Memory,
        CapabilityKind::StoreRead,
        CapabilityKind::StoreWrite,
    ],
    protocol: Protocol { abi: 1, version: 0 },
    restart: RestartPolicy { retries: 3, backoff_ticks: 2 },
    profiles: Profiles::NORMAL_RECOVERY,
};
const BOOT_MANIFESTS: &[Manifest] =
    &[SUPERVISOR_MANIFEST, VIRTIO_MANIFEST, BLOCK_MANIFEST, STORAGE_MANIFEST];

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
    pub fn build(manifests: &'static [Manifest], profile: Profile) -> Result<Self, Error> {
        let selected = manifests.iter().filter(|manifest| manifest.profiles.includes(profile));
        let selected_len = selected.count();
        if selected_len > MAX_MANIFESTS {
            return Err(Error::Cycle);
        }
        for (index, manifest) in manifests.iter().enumerate() {
            if !manifest.profiles.includes(profile) {
                continue;
            }
            if manifest.name.is_empty()
                || manifests[..index]
                    .iter()
                    .any(|other| other.profiles.includes(profile) && other.name == manifest.name)
            {
                return Err(Error::Duplicate);
            }
            if manifest.dependencies.iter().any(|dependency| {
                !manifests
                    .iter()
                    .any(|other| other.profiles.includes(profile) && other.name == *dependency)
            }) {
                return Err(Error::MissingDependency);
            }
        }
        let mut plan = Self { order: [None; MAX_MANIFESTS], len: 0 };
        while plan.len < selected_len {
            let mut progressed = false;
            for manifest in manifests {
                if manifest.profiles.includes(profile)
                    && !plan.contains(manifest.name)
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

    pub fn negotiate(&self, name: &[u8], offered: Protocol) -> Option<Protocol> {
        let required = self.manifest(name)?.protocol;
        (required.abi == offered.abi).then_some(Protocol {
            abi: required.abi,
            version: required.version.min(offered.version),
        })
    }

    pub fn replace(&self, name: &[u8], action: impl FnOnce() -> bool) -> bool {
        self.manifest(name).is_some_and(|_| action())
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

pub fn boot_plan(profile: Profile) -> Result<Plan, Error> {
    Plan::build(BOOT_MANIFESTS, profile)
}

pub fn self_check() -> bool {
    const A: &[u8] = b"a";
    const B: &[u8] = b"b";
    const OK: &[Manifest] = &[
        Manifest {
            name: B,
            dependencies: &[A],
            capabilities: &[],
            protocol: Protocol { abi: 1, version: 0 },
            restart: RestartPolicy { retries: 1, backoff_ticks: 1 },
            profiles: Profiles::ALL,
        },
        Manifest {
            name: A,
            dependencies: &[],
            capabilities: &[],
            protocol: Protocol { abi: 1, version: 0 },
            restart: RestartPolicy { retries: 1, backoff_ticks: 1 },
            profiles: Profiles::ALL,
        },
    ];
    const MISSING: &[Manifest] = &[Manifest {
        name: A,
        dependencies: &[B],
        capabilities: &[],
        protocol: Protocol { abi: 1, version: 0 },
        restart: RestartPolicy { retries: 1, backoff_ticks: 1 },
        profiles: Profiles::ALL,
    }];
    const CYCLE: &[Manifest] = &[
        Manifest {
            name: A,
            dependencies: &[B],
            capabilities: &[],
            protocol: Protocol { abi: 1, version: 0 },
            restart: RestartPolicy { retries: 1, backoff_ticks: 1 },
            profiles: Profiles::ALL,
        },
        Manifest {
            name: B,
            dependencies: &[A],
            capabilities: &[],
            protocol: Protocol { abi: 1, version: 0 },
            restart: RestartPolicy { retries: 1, backoff_ticks: 1 },
            profiles: Profiles::ALL,
        },
    ];
    Plan::build(OK, Profile::Normal).is_ok_and(|plan| plan.starts(A) && plan.starts(B))
        && matches!(Plan::build(MISSING, Profile::Normal), Err(Error::MissingDependency))
        && matches!(Plan::build(CYCLE, Profile::Normal), Err(Error::Cycle))
}

pub fn protocol_self_check() -> bool {
    let Ok(plan) = boot_plan(Profile::Normal) else {
        return false;
    };
    plan.negotiate(VIRTIO_BALLOON, Protocol { abi: 1, version: 2 })
        == Some(Protocol { abi: 1, version: 0 })
        && plan.negotiate(VIRTIO_BALLOON, Protocol { abi: 2, version: 0 }).is_none()
}

pub fn dependency_loss_self_check() -> bool {
    const MISSING: &[Manifest] = &[Manifest {
        name: b"a",
        dependencies: &[b"missing"],
        capabilities: &[],
        protocol: Protocol { abi: 1, version: 0 },
        restart: RestartPolicy { retries: 1, backoff_ticks: 1 },
        profiles: Profiles::ALL,
    }];
    matches!(Plan::build(MISSING, Profile::Normal), Err(Error::MissingDependency))
}

pub fn startup_failure_self_check() -> bool {
    boot_plan(Profile::Normal)
        .is_ok_and(|plan| plan.negotiate(VIRTIO_BALLOON, Protocol { abi: 2, version: 0 }).is_none())
}

pub fn diagnostics_self_check() -> bool {
    StartStage::Protocol.message() == b"protocol"
        && StartStage::Capability.message() == b"capability"
        && StartStage::Register.message() == b"register"
        && StartStage::Bind.message() == b"bind"
        && StartStage::Task.message() == b"task"
}

pub fn replacement_self_check() -> bool {
    let Ok(plan) = boot_plan(Profile::Normal) else {
        return false;
    };
    plan.replace(VIRTIO_BALLOON, || true) && !plan.replace(b"missing", || true)
}

pub fn grant_self_check() -> bool {
    let Ok(plan) = boot_plan(Profile::Normal) else {
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
    let Ok(plan) = boot_plan(Profile::Normal) else {
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

pub fn profiles_self_check() -> bool {
    boot_plan(Profile::Normal).is_ok_and(|plan| plan.starts(VIRTIO_BALLOON))
        && boot_plan(Profile::Recovery).is_ok_and(|plan| plan.starts(VIRTIO_BALLOON))
        && boot_plan(Profile::Diagnostics)
            .is_ok_and(|plan| plan.starts(SUPERVISOR) && !plan.starts(VIRTIO_BALLOON))
}

pub fn health_self_check() -> bool {
    let Ok(plan) = boot_plan(Profile::Normal) else {
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
